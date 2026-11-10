//! Headless desktop controller. No UI toolkit, input capture, or injection.
//! The controller is hosted by the per-user agent; views use the bounded IPC SDK.

#[cfg(windows)]
pub mod ipc;
pub mod protocol;
pub mod service;
pub mod session;

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub const MAX_DEVICES: usize = 64;
const MAX_STATE_BYTES: u64 = 128 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: u64,
    pub name: String,
    pub address: SocketAddr,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub computer_name: String,
    pub default_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u16,
    pub revision: u64,
    pub next_id: u64,
    pub settings: Settings,
    pub devices: Vec<Device>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub device_management: bool,
    pub settings: bool,
    pub local_diagnostics: bool,
    pub identity_setup: bool,
    pub live_input: bool,
    pub video: bool,
    pub code_pairing: bool,
    pub background_agent: bool,
}

pub const CAPABILITIES: Capabilities = Capabilities {
    device_management: true,
    settings: true,
    local_diagnostics: true,
    identity_setup: cfg!(windows),
    live_input: cfg!(all(windows, target_pointer_width = "64")),
    video: false,
    code_pairing: false,
    background_agent: cfg!(windows),
};

impl Snapshot {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || self.devices.len() > MAX_DEVICES {
            return Err("Unsupported desktop data or too many saved computers.".into());
        }
        validate_name(&self.settings.computer_name)?;
        if self.settings.default_port == 0 || self.next_id == 0 {
            return Err("Invalid saved port or computer ID.".into());
        }
        for (i, device) in self.devices.iter().enumerate() {
            validate_name(&device.name)?;
            validate_address(device.address)?;
            if device.id == 0
                || device.id >= self.next_id
                || self.devices[..i]
                    .iter()
                    .any(|d| d.id == device.id || d.address == device.address)
            {
                return Err("Duplicate or invalid saved computer.".into());
            }
        }
        Ok(())
    }
}

impl Controller {
    pub fn peers(&self) -> Result<protocol::TrustedPeers> {
        Ok(protocol::TrustedPeers {
            hosts: self.identity_store(false).trusted_peers(false)?,
            clients: self.identity_store(true).trusted_peers(true)?,
        })
    }
    pub(crate) fn identity_store(
        &self,
        incoming_client: bool,
    ) -> bricks_transport::identity::Store {
        bricks_transport::identity::Store::new(self.root.join("identities").join(
            if incoming_client {
                "bricks-host"
            } else {
                "bricks-client"
            },
        ))
    }
    pub fn trust_peer(&self, file: &str, expected: &str, incoming_client: bool) -> Result<()> {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Compare the SHA-256 fingerprint on the other PC, then paste all 64 hexadecimal characters.".into());
        }
        let bytes = bricks_transport::identity::read_bounded(Path::new(file))?;
        let expected = expected.to_ascii_lowercase();
        let store = self.identity_store(incoming_client);
        if let Ok(existing) = store.trusted_certificate(&expected) {
            if existing.as_ref() == bytes.as_slice() {
                if incoming_client && !store.trusted_peers(true)?.contains(&expected) {
                    return Err("This peer has no control permission. Revoke its old trust before explicitly importing it for control.".into());
                }
                return Ok(());
            }
            return Err(
                "Stored certificate differs; revoke it before importing a replacement.".into(),
            );
        }
        store.trust(bytes.into(), &expected, incoming_client)
    }
}

pub fn default_root() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .ok_or("Windows application data folder is unavailable.")?,
    )
    .join("Bricks"))
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.trim() != name
        || name.is_empty()
        || name.chars().count() > 64
        || name.chars().any(char::is_control)
    {
        return Err(
            "Use a name of 1–64 characters, without surrounding spaces or control characters."
                .into(),
        );
    }
    Ok(())
}

pub fn parse_address(ip: &str, port: &str) -> Result<SocketAddr> {
    let ip: IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| "Enter a numeric LAN or tunnel IP address, for example 192.168.1.20.")?;
    let port: u16 = port
        .trim()
        .parse()
        .map_err(|_| "Port must be a number from 1 to 65535.")?;
    let address = SocketAddr::new(ip, port);
    validate_address(address)?;
    Ok(address)
}

fn validate_address(address: SocketAddr) -> Result<()> {
    if address.port() == 0
        || address.ip().is_unspecified()
        || address.ip().is_multicast()
        || address.ip() == IpAddr::V4(std::net::Ipv4Addr::BROADCAST)
    {
        return Err("Use a reachable unicast IP address and a port from 1 to 65535.".into());
    }
    Ok(())
}

pub struct Controller {
    root: PathBuf,
    state: Snapshot,
    // Held for the controller lifetime; a second desktop cannot overwrite state.
    _lock: File,
}

impl Controller {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("desktop.lock"))?;
        lock.try_lock().map_err(
            |_| "Bricks is already open for this account. Switch to its existing window.",
        )?;
        let path = root.join("desktop.json");
        let state = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MAX_STATE_BYTES {
                    return Err(
                        "Desktop data exceeds its size limit; the original file was kept.".into(),
                    );
                }
                let state: Snapshot = serde_json::from_slice(&bytes).map_err(|e| {
                    format!(
                        "Could not read {}: {e}. Original data was kept.",
                        path.display()
                    )
                })?;
                state.validate()?;
                state
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Snapshot {
                schema_version: 1,
                revision: 0,
                next_id: 1,
                settings: Settings {
                    computer_name: std::env::var("COMPUTERNAME")
                        .ok()
                        .filter(|s| validate_name(s).is_ok())
                        .unwrap_or_else(|| "This PC".into()),
                    default_port: 47990,
                },
                devices: Vec::new(),
            },
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            root,
            state,
            _lock: lock,
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.clone()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn commit(&mut self, expected_revision: u64, mut next: Snapshot) -> Result<()> {
        if expected_revision != self.state.revision {
            return Err("Settings changed. Reopen the editor and try again.".into());
        }
        next.revision = expected_revision
            .checked_add(1)
            .ok_or("Revision limit reached.")?;
        next.validate()?;
        let bytes = serde_json::to_vec_pretty(&next)?;
        // Lock makes the temporary path exclusive to this controller. A previous
        // interrupted write may leave it behind; desktop.json remains authoritative.
        let temporary = self.root.join("desktop.json.pending");
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, self.root.join("desktop.json"))?;
        self.state = next;
        Ok(())
    }

    pub fn save_device(
        &mut self,
        revision: u64,
        id: Option<u64>,
        name: String,
        address: SocketAddr,
    ) -> Result<u64> {
        validate_name(&name)?;
        validate_address(address)?;
        let mut next = self.snapshot();
        if next
            .devices
            .iter()
            .any(|d| Some(d.id) != id && d.address == address)
        {
            return Err("That IP address and port are already saved.".into());
        }
        let id = if let Some(id) = id {
            let device = next
                .devices
                .iter_mut()
                .find(|d| d.id == id)
                .ok_or("That computer no longer exists.")?;
            device.name = name;
            device.address = address;
            id
        } else {
            if next.devices.len() >= MAX_DEVICES {
                return Err(
                    "You can save up to 64 computers. Forget one before adding another.".into(),
                );
            }
            let id = next.next_id;
            next.next_id = id.checked_add(1).ok_or("Computer ID limit reached.")?;
            next.devices.push(Device { id, name, address });
            id
        };
        self.commit(revision, next)?;
        Ok(id)
    }

    pub fn forget_device(&mut self, revision: u64, id: u64) -> Result<()> {
        let mut next = self.snapshot();
        let index = next
            .devices
            .iter()
            .position(|d| d.id == id)
            .ok_or("That computer no longer exists.")?;
        next.devices.remove(index);
        self.commit(revision, next)
    }

    pub fn update_settings(&mut self, revision: u64, settings: Settings) -> Result<()> {
        let mut next = self.snapshot();
        next.settings = settings;
        self.commit(revision, next)
    }

    /// Explicit user action only. Preserves existing identities and private keys.
    /// Exports public certificates only; never creates or changes peer trust.
    pub fn setup_identities(&self) -> Result<String> {
        let public = self.root.join("public");
        fs::create_dir_all(&public)?;
        let mut descriptions = Vec::new();
        for name in ["bricks-host", "bricks-client"] {
            let path = self.root.join("identities").join(name);
            let exists = path.join("identity.json").try_exists()?;
            let store = bricks_transport::identity::Store::new(path);
            if !exists {
                store.init(name)?;
            }
            let cert = store.certificate()?;
            let fingerprint = bricks_transport::identity::fingerprint(cert.as_ref());
            // The DER certificate is public; no key material enters this buffer.
            let export = public.join(format!("{name}.der"));
            fs::write(&export, cert.as_ref())?;
            descriptions.push(format!(
                "{name}\nSHA-256: {fingerprint}\nPublic certificate: {}",
                export.display()
            ));
        }
        Ok(format!(
            "This PC is set up. Private keys are protected by Windows for your account.\n\n{}\n\nNo peer has been trusted. Code pairing is not available yet.",
            descriptions.join("\n\n")
        ))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Diagnostic {
    Recovery,
    Loopback,
    RecoveryNetwork,
}

pub fn run_diagnostic(kind: Diagnostic) -> Result<serde_json::Value> {
    match kind {
        Diagnostic::Recovery => {
            let config =
                bricks_config::resolve(include_str!("../../../docs/examples/host.toml"), None)?;
            Ok(serde_json::to_value(
                bricks_transport::recovery_check::run(config)?,
            )?)
        }
        Diagnostic::Loopback => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            Ok(serde_json::to_value(runtime.block_on(
                bricks_transport::probe::loopback_selftest(100),
            )?)?)
        }
        Diagnostic::RecoveryNetwork => run_network_diagnostic(std::sync::Arc::new(
            std::sync::atomic::AtomicBool::new(false),
        )),
    }
}
pub fn run_network_diagnostic(
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<serde_json::Value> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    Ok(serde_json::to_value(runtime.block_on(
        bricks_transport::recovery_lab::run(Default::default(), cancel),
    )?)?)
}

/// Preserve reports without overwriting previous evidence.
pub fn export_report(root: &Path, report: &serde_json::Value) -> Result<PathBuf> {
    let reports = root.join("reports");
    fs::create_dir_all(&reports)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let path = reports.join(format!("bricks-{stamp}.json"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(&serde_json::to_vec_pretty(report)?)?;
    file.sync_all()?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "bricks-desktop-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn edits_persist_with_stable_ids_and_revision_conflicts_do_not_write() {
        let root = TestRoot::new();
        let mut control = Controller::open(root.0.clone()).unwrap();
        let address = "192.168.1.20:47990".parse().unwrap();
        let id = control
            .save_device(0, None, "Gaming PC".into(), address)
            .unwrap();
        let before = fs::read(root.0.join("desktop.json")).unwrap();
        assert!(
            control
                .save_device(0, Some(id), "Stale".into(), address)
                .is_err()
        );
        assert_eq!(before, fs::read(root.0.join("desktop.json")).unwrap());
        control
            .save_device(1, Some(id), "Renamed".into(), address)
            .unwrap();
        control
            .update_settings(
                2,
                Settings {
                    computer_name: "Laptop".into(),
                    default_port: 48000,
                },
            )
            .unwrap();
        drop(control);
        let mut control = Controller::open(root.0.clone()).unwrap();
        assert_eq!(control.snapshot().devices[0].name, "Renamed");
        assert_eq!(control.snapshot().settings.default_port, 48000);
        assert_eq!(control.snapshot().devices[0].id, id);
        control.forget_device(3, id).unwrap();
        drop(control);
        assert!(
            Controller::open(root.0.clone())
                .unwrap()
                .snapshot()
                .devices
                .is_empty()
        );
    }

    #[test]
    fn invalid_inputs_and_duplicates_leave_state_unchanged() {
        let root = TestRoot::new();
        let mut control = Controller::open(root.0.clone()).unwrap();
        for ip in [
            "0.0.0.0",
            "::",
            "224.0.0.1",
            "255.255.255.255",
            "example.com",
        ] {
            assert!(parse_address(ip, "47990").is_err());
        }
        assert!(parse_address("127.0.0.1", "0").is_err());
        assert!(parse_address("fd00::1", "47990").is_ok());
        let address = parse_address("100.101.102.103", "47990").unwrap();
        for name in ["", " ", "bad\nname"] {
            assert!(control.save_device(0, None, name.into(), address).is_err());
        }
        control
            .save_device(0, None, "Tunnel PC".into(), address)
            .unwrap();
        let snapshot = control.snapshot();
        assert!(
            control
                .save_device(1, None, "Duplicate".into(), address)
                .is_err()
        );
        assert!(
            control
                .update_settings(
                    1,
                    Settings {
                        computer_name: "Valid".into(),
                        default_port: 0
                    }
                )
                .is_err()
        );
        assert_eq!(snapshot, control.snapshot());
    }

    #[test]
    fn corrupt_or_future_data_is_preserved_and_second_writer_is_rejected() {
        let root = TestRoot::new();
        let control = Controller::open(root.0.clone()).unwrap();
        let mut future = control.snapshot();
        assert!(Controller::open(root.0.clone()).is_err());
        drop(control);
        let path = root.0.join("desktop.json");
        fs::write(&path, b"broken data").unwrap();
        assert!(Controller::open(root.0.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken data");
        future.schema_version = 2;
        let future_bytes = serde_json::to_vec(&future).unwrap();
        fs::write(&path, &future_bytes).unwrap();
        assert!(Controller::open(root.0.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), future_bytes);
        fs::write(&path, vec![b'x'; MAX_STATE_BYTES as usize + 1]).unwrap();
        assert!(Controller::open(root.0.clone()).is_err());
        assert_eq!(fs::metadata(path).unwrap().len(), MAX_STATE_BYTES + 1);
    }

    #[test]
    fn capacity_limit_and_failed_disk_commit_keep_previous_state() {
        let root = TestRoot::new();
        let mut control = Controller::open(root.0.clone()).unwrap();
        for i in 0..MAX_DEVICES {
            let address = format!("192.168.1.{}:47990", i + 1).parse().unwrap();
            control
                .save_device(i as u64, None, format!("PC {i}"), address)
                .unwrap();
        }
        let before = control.snapshot();
        assert!(
            control
                .save_device(
                    64,
                    None,
                    "Too many".into(),
                    "127.0.0.1:47990".parse().unwrap()
                )
                .is_err()
        );
        fs::create_dir(root.0.join("desktop.json.pending")).unwrap();
        assert!(control.forget_device(64, 1).is_err());
        assert_eq!(before, control.snapshot());
        drop(control);
        assert_eq!(before, Controller::open(root.0.clone()).unwrap().snapshot());
    }

    #[test]
    fn real_recovery_adapter_exports_reopenable_distinct_reports() {
        let root = TestRoot::new();
        let report = run_diagnostic(Diagnostic::Recovery).unwrap();
        assert_eq!(report["status"], "PASS");
        assert_eq!(report["scenarios_passed"], report["scenarios_total"]);
        let first = export_report(&root.0, &report).unwrap();
        let second = export_report(&root.0, &report).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(first).unwrap()).unwrap(),
            report
        );
    }

    #[cfg(windows)]
    #[test]
    fn identity_setup_is_idempotent_and_exports_public_certificates_only() {
        let root = TestRoot::new();
        let control = Controller::open(root.0.clone()).unwrap();
        let first = control.setup_identities().unwrap();
        let identity = root.0.join("identities/bricks-host/identity.json");
        let original = fs::read(&identity).unwrap();
        assert_eq!(first, control.setup_identities().unwrap());
        assert_eq!(original, fs::read(&identity).unwrap());
        let store = bricks_transport::identity::Store::new(root.0.join("identities/bricks-host"));
        let certificate = store.certificate().unwrap();
        assert_eq!(
            fs::read(root.0.join("public/bricks-host.der")).unwrap(),
            certificate.as_ref()
        );
        assert_eq!(fs::read_dir(root.0.join("public")).unwrap().count(), 2);
        assert!(!root.0.join("identities/bricks-host/peers").exists());
        fs::write(&identity, b"damaged metadata").unwrap();
        assert!(control.setup_identities().is_err());
        assert_eq!(fs::read(identity).unwrap(), b"damaged metadata");
    }
    #[cfg(windows)]
    #[test]
    fn peer_import_requires_the_expected_digest_and_revoke_removes_control_permission() {
        let root = TestRoot::new();
        let control = Controller::open(root.0.clone()).unwrap();
        control.setup_identities().unwrap();
        let certificate = root.0.join("public/bricks-client.der");
        let bytes = fs::read(&certificate).unwrap();
        let fingerprint = bricks_transport::identity::fingerprint(&bytes);
        assert!(
            control
                .trust_peer(certificate.to_str().unwrap(), &"0".repeat(64), true)
                .is_err()
        );
        assert!(control.peers().unwrap().clients.is_empty());
        control
            .trust_peer(certificate.to_str().unwrap(), &fingerprint, true)
            .unwrap();
        assert_eq!(control.peers().unwrap().clients, vec![fingerprint.clone()]);
        assert!(control.peers().unwrap().hosts.is_empty());
        control.identity_store(true).revoke(&fingerprint).unwrap();
        assert!(control.peers().unwrap().clients.is_empty());
    }
}
