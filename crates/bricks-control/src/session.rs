//! Control-plane process ownership. The dedicated input worker and its watchdog
//! survive UI scheduling stalls; closing the owning pipe requests cleanup.
use crate::{
    Result, Snapshot,
    protocol::{Command, Outcome},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(all(windows, target_pointer_width = "64"))]
pub fn run(
    root: &Path,
    snapshot: Snapshot,
    command: Command,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(String),
) -> Result<(Outcome, String)> {
    use std::{
        fs,
        io::Read,
        net::{IpAddr, SocketAddr},
        os::windows::process::CommandExt,
        process::{Command as Process, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    let Command::StartLive {
        device_id,
        peer_fingerprint,
        bind_ip,
    } = command
    else {
        return Err("not a live-session request".into());
    };
    let incoming = device_id.is_none();
    let identity = root.join("identities").join(if incoming {
        "bricks-host"
    } else {
        "bricks-client"
    });
    let store = bricks_transport::identity::Store::new(&identity);
    store
        .certificate()
        .map_err(|_| "Choose Set up this PC before starting a live connection.")?;
    let peer = peer_fingerprint.trim().to_ascii_lowercase();
    if !store.trusted_peers(incoming)?.contains(&peer) {
        return Err("Verify and import this peer's public certificate in Settings first.".into());
    }
    let address = if let Some(id) = device_id {
        snapshot
            .devices
            .iter()
            .find(|device| device.id == id)
            .ok_or("Select a saved computer first.")?
            .address
    } else {
        let ip: IpAddr = bind_ip
            .trim()
            .parse()
            .map_err(|_| "Enter a numeric listen IP, or 0.0.0.0 for IPv4 interfaces.")?;
        if ip.is_multicast() {
            return Err("A multicast address cannot be used for sharing.".into());
        }
        SocketAddr::new(ip, snapshot.settings.default_port)
    };
    let executable = std::env::current_exe()?;
    if !executable.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .eq_ignore_ascii_case("bricks-agent.exe")
    }) {
        return Err("The bundled Bricks agent is required for live input.".into());
    }
    let sessions = root.join("sessions");
    fs::create_dir_all(&sessions)?;
    let directory = sessions.join(format!(
        "session-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&directory)?;
    let config = if incoming {
        include_str!("../../../docs/examples/host.toml")
    } else {
        include_str!("../../../docs/examples/client.toml")
    }
    .replace("127.0.0.1:47990", &address.to_string());
    let resolved = bricks_config::resolve(&config, None)?;
    let _ = resolved.fingerprint()?;
    let config_path = directory.join("session.toml");
    fs::write(&config_path, config)?;
    let report = directory.join("report.json");
    let log = directory.join("session.log");
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled before live startup".into());
    }
    let mut process = Process::new(executable);
    process
        .arg(if incoming { "live-host" } else { "live-client" })
        .arg("--config")
        .arg(config_path)
        .arg("--identity-dir")
        .arg(identity)
        .args(["--peer", &peer, "--owned-session", "--report"])
        .arg(&report)
        .stdin(Stdio::piped())
        .stdout(fs::File::create(&log)?)
        .stderr(fs::File::create(directory.join("errors.log"))?)
        .creation_flags(0x0800_0000);
    if incoming {
        process.arg("--approve-live");
    }
    let child = process.spawn()?;
    struct Owned(std::process::Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            drop(self.0.stdin.take());
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let mut owned = Owned(child);
    let mut stopping = None;
    let mut last_status = String::new();
    let status = loop {
        if cancel.load(Ordering::Acquire) && stopping.is_none() {
            drop(owned.0.stdin.take());
            stopping = Some(Instant::now());
            progress("Stopping input and waiting for cleanup confirmation...".into());
        }
        if let Some(status) = owned.0.try_wait()? {
            break status;
        }
        if stopping.is_some_and(|at| at.elapsed() > Duration::from_secs(2)) {
            owned.0.kill()?;
            break owned.0.wait()?;
        }
        let text = fs::File::open(&log)
            .and_then(|file| {
                let mut text = String::new();
                file.take(65536).read_to_string(&mut text)?;
                Ok(text)
            })
            .unwrap_or_default();
        let message = if text.contains("Bricks live input connected.") {
            "Live input connected. Switching the target window ends sharing.".to_owned()
        } else if text.contains("listening on") {
            format!("Sharing enabled on {address}. Waiting for the verified peer.")
        } else if !incoming {
            format!(
                "Connecting to {address}. Forwarding starts only after you click Start in the capture window."
            )
        } else {
            format!("Starting sharing on {address}...")
        };
        if stopping.is_none() && message != last_status {
            progress(message.clone());
            last_status = message;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let value: Option<serde_json::Value> = fs::read(&report)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let mut watcher: Option<serde_json::Value> = fs::read(report.with_extension("watchdog.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    if incoming && value.is_none() {
        let until = Instant::now() + Duration::from_millis(300);
        while watcher.is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
            watcher = fs::read(report.with_extension("watchdog.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        }
    }
    let cleanup = if incoming {
        if value
            .as_ref()
            .is_some_and(|value| value["input_admitted"] == false)
        {
            "not_needed"
        } else if value.as_ref().is_some_and(|value| {
            value["owned_after_cleanup"] == 0 && value["cleanup_failures"] == 0
        }) || watcher
            .as_ref()
            .is_some_and(|value| value["unresolved_owned"] == 0 && value["status"] == "complete")
        {
            "api_confirmed"
        } else {
            "unknown_local"
        }
    } else if value
        .as_ref()
        .is_some_and(|value| value["remote_cleanup_confirmed"] == true)
    {
        "remote_api_confirmed"
    } else {
        "remote_unconfirmed"
    };
    let errors = fs::File::open(directory.join("errors.log"))
        .and_then(|file| {
            let mut text = String::new();
            file.take(4096).read_to_string(&mut text)?;
            Ok(text)
        })
        .unwrap_or_default();
    let detail = value
        .as_ref()
        .and_then(|value| value["reason"].as_str())
        .or_else(|| watcher.as_ref().and_then(|value| value["reason"].as_str()))
        .unwrap_or(if !errors.trim().is_empty() {
            errors.trim()
        } else if status.success() {
            "worker exited"
        } else {
            "worker stopped unexpectedly"
        });
    let message = format!(
        "Input session ended: {detail}. Cleanup: {cleanup}. Report: {}",
        directory.display()
    );
    Ok((
        Outcome {
            message,
            ..Default::default()
        },
        cleanup.into(),
    ))
}
#[cfg(not(all(windows, target_pointer_width = "64")))]
pub fn run(
    _root: &Path,
    _snapshot: Snapshot,
    _command: Command,
    _cancel: Arc<AtomicBool>,
    _progress: impl FnMut(String),
) -> Result<(Outcome, String)> {
    Err("Live input currently requires Windows x64.".into())
}
