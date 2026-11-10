use bricks_control::{
    Diagnostic, Result,
    protocol::{Command, Failure},
};
mod power;
use bricks_transport::{
    recovery_lab::{self, Impairment, Options},
    resources,
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
fn main() -> Result<()> {
    let _awake = power::KeepAwake::request()?;
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let high_qos = args.iter().any(|arg| arg == "--high-qos");
    args.retain(|arg| arg != "--high-qos");
    // The root actor remains on the calling thread in either runtime, including
    // its scoped MMCSS guard. Workers only service spawned QUIC I/O tasks.
    let io_workers: usize = if let Some(index) = args.iter().position(|arg| arg == "--io-workers") {
        let value = args
            .get(index + 1)
            .ok_or("--io-workers requires 0, 1 or 2")?
            .parse()?;
        args.drain(index..=index + 1);
        value
    } else {
        2
    };
    if io_workers > 2 {
        return Err("--io-workers requires 0, 1 or 2".into());
    }
    let _policy = power::ProcessPolicy::request(high_qos)?;
    let timer_override = _policy.timer_override;
    let mut builder = if io_workers == 0 {
        tokio::runtime::Builder::new_current_thread()
    } else {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.worker_threads(io_workers);
        builder
    };
    let runtime = builder.enable_all().build()?;
    runtime.block_on(async move {
        match args.first().map(String::as_str) {
            Some("suite")=>suite(Path::new(args.get(1).ok_or("suite needs an output directory")?),args.iter().any(|s|s=="--quick"),args.iter().any(|s|s=="--conservative-recovery"),args.iter().any(|s|s=="--games-scheduling"),high_qos,timer_override,io_workers).await?,
            Some("lifecycle")=>println!("{}",serde_json::to_string_pretty(&bricks_transport::lifecycle::run().await?)?),
            Some("sdk-crash-test")=>sdk_crash_test(Path::new(args.get(1).ok_or("agent executable required")?),Path::new(args.get(2).ok_or("fresh data directory required")?)).await?,
            Some("process-bench")=>process_bench(Path::new(args.get(1).ok_or("host executable required")?),Path::new(args.get(2).ok_or("client executable required")?),Path::new(args.get(3).ok_or("fresh output directory required")?),args.get(4).map(|value|value.parse()).transpose()?.unwrap_or(60)).await?,
            Some("timer-cost")=>{
                let seconds:u64=args.get(1).ok_or("seconds required")?.parse()?;
                if !(1..=60).contains(&seconds){return Err("timer duration must be 1..60 seconds".into());}
                let native=args.get(2).is_some_and(|mode|mode=="native");
                let _resolution=bricks_transport::timing::ActiveTimerResolution::request()?;
                let first=resources::sample(std::process::id())?;let started=Instant::now();let mut wakes=0;
                let mut ticks=tokio::time::interval(Duration::from_millis(1));
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                while started.elapsed()<Duration::from_secs(seconds){if native{std::thread::sleep(Duration::from_millis(1));}else{ticks.tick().await;}wakes+=1;}
                let wall=started.elapsed().as_nanos() as f64;let last=resources::sample(std::process::id())?;
                println!("{}",serde_json::to_string_pretty(&serde_json::json!({"measurement_class":"timer_cost_diagnostic","native":native,"wakes":wakes,"cpu_percent_one_core":(last.cpu_ns-first.cpu_ns) as f64/wall*100.,"user_percent_one_core":(last.user_cpu_ns-first.user_cpu_ns) as f64/wall*100.,"kernel_percent_one_core":(last.kernel_cpu_ns-first.kernel_cpu_ns) as f64/wall*100.}))?);
            }
            Some("resources")=>{
                let pid=args.get(1).ok_or("pid required")?.parse()?;let seconds:u64=args.get(2).ok_or("duration required")?.parse()?;
                if !(1..=600).contains(&seconds){return Err("resource duration must be 1..600 seconds".into());}
                let started=Instant::now();let first=resources::sample(pid)?;let mut points=Vec::new();
                for _ in 0..seconds {tokio::time::sleep(Duration::from_secs(1)).await;points.push(resources::sample(pid)?);}
                let last=*points.last().unwrap();let percent=(last.cpu_ns-first.cpu_ns) as f64/started.elapsed().as_nanos() as f64*100.0;
                println!("{}",serde_json::to_string_pretty(&serde_json::json!({"measurement_class":"windows_process_resources","pid":pid,"duration_ns":started.elapsed().as_nanos(),"cpu_percent_one_logical_core":percent,"private_peak_bytes":points.iter().map(|p|p.private_bytes).max(),"private_delta_bytes":last.private_bytes as i64-first.private_bytes as i64,"samples":points}))?);
            }
            _=>{
                let options=if let Some(path)=args.first(){serde_json::from_slice(&std::fs::read(path)?)?}else{Options::default()};
                let mut report=serde_json::to_value(recovery_lab::run(options,Arc::new(AtomicBool::new(false))).await?)?;
                report["timer_resolution_override"]=serde_json::json!(timer_override);
                report["high_qos_requested"]=serde_json::json!(high_qos);
                report["io_workers"]=serde_json::json!(io_workers);
                println!("{}",serde_json::to_string_pretty(&report)?);
            }
        }
        Ok::<(),bricks_control::Error>(())
    })?;
    Ok(())
}

#[cfg(windows)]
async fn process_bench(host: &Path, client: &Path, output: &Path, seconds: u64) -> Result<()> {
    use std::{fs, os::windows::process::CommandExt, process::Command};
    if !(1..=120).contains(&seconds) {
        return Err("process benchmark duration must be 1..120 seconds".into());
    }
    fs::create_dir(output)?;
    let output = output.canonicalize()?;
    let pair = recovery_lab::EphemeralPair::new()?;
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let address = socket.local_addr()?;
    drop(socket);
    let host_config = output.join("host.toml");
    let client_config = output.join("client.toml");
    fs::write(
        &host_config,
        include_str!("../../../docs/examples/host.toml")
            .replace("127.0.0.1:47990", &address.to_string()),
    )?;
    fs::write(
        &client_config,
        include_str!("../../../docs/examples/client.toml")
            .replace("127.0.0.1:47990", &address.to_string()),
    )?;
    struct Worker(std::process::Child);
    impl Drop for Worker {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let host_log = output.join("host.log");
    let mut server = Worker(
        Command::new(host)
            .arg("record-host")
            .arg("--config")
            .arg(&host_config)
            .arg("--identity-dir")
            .arg(pair.directory().join("host"))
            .args(["--peer", &pair.client_fp, "--report"])
            .arg(output.join("host-report.json"))
            .stdout(fs::File::create(&host_log)?)
            .stderr(fs::File::create(output.join("host-errors.log"))?)
            .creation_flags(0x0800_0000)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read_to_string(&host_log)
            .unwrap_or_default()
            .contains("listening on")
        {
            break;
        }
        if server.0.try_wait()?.is_some() || Instant::now() > deadline {
            return Err("recording host failed to start".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut sender = Worker(
        Command::new(client)
            .arg("record-client")
            .arg("--config")
            .arg(&client_config)
            .arg("--identity-dir")
            .arg(pair.directory().join("client"))
            .args(["--peer", &pair.host_fp, "--report"])
            .arg(output.join("client-report.json"))
            .arg("--events")
            .arg((seconds * 1000).to_string())
            .args(["--rate", "1000"])
            .stdout(fs::File::create(output.join("client.log"))?)
            .stderr(fs::File::create(output.join("client-errors.log"))?)
            .creation_flags(0x0800_0000)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(seconds + 8);
    while sender.0.try_wait()?.is_none() || server.0.try_wait()?.is_none() {
        if Instant::now() > deadline {
            return Err("separate-process benchmark did not finish".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let h: serde_json::Value = serde_json::from_slice(&fs::read(output.join("host-report.json"))?)?;
    let c: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("client-report.json"))?)?;
    let totals = serde_json::json!([
        seconds * 1000,
        -((seconds * 1000) as i64),
        seconds * 120,
        -((seconds.div_ceil(2) * 120) as i64)
    ]);
    let passed = h["recording_only"] == true
        && h["reason"] == "completed"
        && c["completed"] == true
        && h["relative_and_wheel_totals"] == totals
        && h["critical_applied"] == c["generated_critical"]
        && h["owned_after_cleanup"] == 0
        && h["cleanup_failures"] == 0
        && c["unacknowledged_at_stop"] == 0
        && c["synthetic_motion_generated"] == seconds * 1000;
    let summary = serde_json::json!({"status":if passed{"PASS"}else{"FAIL"},"measurement_class":"separate_process_synthetic_input_recording","seconds":seconds,"motion_hz":1000,"host_cpu_percent_one_core":h["process_cpu_percent_one_logical_core"],"client_cpu_percent_one_core":c["process_cpu_percent_one_logical_core"],"host_executable":host,"client_executable":client,"local_keys_and_mouse_injection":false,"host_peer_fingerprint":pair.host_fp,"client_peer_fingerprint":pair.client_fp,"limits":"Both processes run on one PC; client includes synthetic pacing, host uses a recording injector. Not per-endpoint live-input acceptance or a cross-clock latency measurement."});
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if !passed {
        return Err("separate-process fidelity/cleanup check failed".into());
    }
    Ok(())
}
#[cfg(not(windows))]
async fn process_bench(_host: &Path, _client: &Path, _output: &Path, _seconds: u64) -> Result<()> {
    Err("process benchmark currently requires Windows".into())
}

async fn suite(
    output: &Path,
    quick: bool,
    conservative: bool,
    games: bool,
    high_qos: bool,
    timer_override: bool,
    io_workers: usize,
) -> Result<()> {
    std::fs::create_dir_all(output)?;
    let manifest = serde_json::json!({"schema_version":1,"measurement_class":"single_pc_recording_suite","quick":quick,"history_cutoff_profile":if conservative{"150 ms, within documented bounds"}else{"30 ms minimum profile"},"toolchain":"Rust 1.99.1","profile":if cfg!(debug_assertions){"debug"}else{"release"},"logical_processors":std::thread::available_parallelism()?.get(),"targets_document":"docs/targets-and-benchmarks.md","scope":"Same-clock encrypted loopback with synthetic generation and recording injection; no capture, OS injection, two-PC or physical acceptance","impairment_boundary":"Application datagrams in both directions, before QUIC send; not a NIC/router impairment","clock_uncertainty":"uncalibrated Instant; nanosecond units are not nanosecond accuracy","warmup":if quick{"1-second development warm-up; not acceptance evidence"}else{"60 seconds of prior QUIC traffic on a separate connection; per-condition protocol-compliant warmup/alternating comparison remains pending"}});
    let mut manifest = manifest;
    manifest["timer_resolution_override"] = serde_json::json!(timer_override);
    manifest["io_workers"] = serde_json::json!(io_workers);
    manifest["power_qos"] = serde_json::json!(if high_qos {
        "Explicit process HighQoS"
    } else {
        "OS heuristics"
    });
    manifest["idle_sleep_request"] =
        serde_json::json!("Scoped system-required request; manual sleep and OS policy still apply");
    std::fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    let mut cases: Vec<(String, Options)> = Vec::new();
    cases.push((
        "warmup".into(),
        Options {
            events: if quick { 1000 } else { 60000 },
            ..Default::default()
        },
    ));
    for i in 0..if quick { 1 } else { 5 } {
        cases.push((
            format!("healthy-{i}"),
            Options {
                events: if quick { 1000 } else { 10000 },
                seed: i + 1,
                ..Default::default()
            },
        ));
    }
    for loss in [50, 100, 200] {
        for seed in 1..=if quick { 1 } else { 5 } {
            cases.push((
                format!("loss-{loss}-seed-{seed}"),
                Options {
                    events: if quick { 1000 } else { 20000 },
                    seed,
                    impairment: Impairment {
                        loss_per_10000: loss,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ));
        }
    }
    for rate in [1000, 4000, 8000] {
        cases.push((
            format!("stress-{rate}"),
            Options {
                events: if quick { 1000 } else { u64::from(rate) * 60 },
                rate_hz: rate,
                ..Default::default()
            },
        ));
    }
    for (name, impairment) in [
        (
            "lost-final-release",
            Impairment {
                drop_final_release_once: true,
                ..Default::default()
            },
        ),
        (
            "reorder-0.1pct",
            Impairment {
                reorder_per_10000: 10,
                ..Default::default()
            },
        ),
        (
            "duplicate-0.1pct",
            Impairment {
                duplicate_per_10000: 10,
                ..Default::default()
            },
        ),
        (
            "reorder-1pct",
            Impairment {
                reorder_per_10000: 100,
                ..Default::default()
            },
        ),
        (
            "duplicate-1pct",
            Impairment {
                duplicate_per_10000: 100,
                ..Default::default()
            },
        ),
        (
            "jitter-1ms",
            Impairment {
                jitter_ms: 1,
                ..Default::default()
            },
        ),
        (
            "jitter-5ms",
            Impairment {
                jitter_ms: 5,
                ..Default::default()
            },
        ),
        (
            "jitter-10ms",
            Impairment {
                jitter_ms: 10,
                ..Default::default()
            },
        ),
        (
            "burst-2",
            Impairment {
                burst_packets: 2,
                ..Default::default()
            },
        ),
        (
            "burst-5",
            Impairment {
                burst_packets: 5,
                ..Default::default()
            },
        ),
        (
            "burst-10",
            Impairment {
                burst_packets: 10,
                ..Default::default()
            },
        ),
    ] {
        cases.push((
            name.into(),
            Options {
                events: if quick { 1000 } else { 10000 },
                impairment,
                ..Default::default()
            },
        ));
    }
    let pair = recovery_lab::EphemeralPair::new()?;
    let mut summary = Vec::new();
    for (name, mut options) in cases {
        options.games_scheduling = games;
        if conservative {
            options.recovery_deadline_ms = Some(150);
        }
        eprintln!(
            "Running {name}: {} transitions at {} Hz",
            options.events, options.rate_hz
        );
        let mut value =
            match recovery_lab::run_with_pair(&pair, options, Arc::new(AtomicBool::new(false)))
                .await
            {
                Ok(report) => serde_json::to_value(report)?,
                Err(error) => {
                    serde_json::json!({"status":"HARNESS_ERROR","error":error.to_string()})
                }
            };
        value["timer_resolution_override"] = serde_json::json!(timer_override);
        std::fs::write(
            output.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value)?,
        )?;
        summary.push(serde_json::json!({"case":name,"status":value["status"],"generated":value["generated"],"applied":value["applied"],"missing":value["missing"],"stop_reason":value["stop_reason"]}));
        std::fs::write(
            output.join("summary.json"),
            serde_json::to_vec_pretty(&summary)?,
        )?;
    }
    let lifecycle = bricks_transport::lifecycle::run().await?;
    std::fs::write(
        output.join("lifecycle.json"),
        serde_json::to_vec_pretty(&lifecycle)?,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"directory":output,"cases":summary.len(),"lifecycle_passed":lifecycle.iter().filter(|c|c.passed).count(),"lifecycle_total":lifecycle.len()})
        )?
    );
    Ok(())
}

#[cfg(windows)]
async fn sdk_crash_test(executable: &Path, root: &Path) -> Result<()> {
    use bricks_control::ipc::Client;
    use std::os::windows::process::CommandExt;
    if root.join("desktop.json").exists() || root.join("agent-running.json").exists() {
        return Err("SDK crash test requires a fresh explicit data directory".into());
    }
    std::fs::create_dir_all(root)?;
    fn start(executable: &Path, root: &Path) -> Result<std::process::Child> {
        Ok(std::process::Command::new(executable)
            .arg("--data-dir")
            .arg(root)
            .arg("--idle-exit-ms")
            .arg("1000")
            .creation_flags(0x08000000)
            .spawn()?)
    }
    async fn connect(root: &Path) -> Result<Client> {
        for _ in 0..100 {
            if let Ok(client) = Client::connect(root).await {
                return Ok(client);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Err("agent did not start".into())
    }
    let mut first = start(executable, root)?;
    let mut client = connect(root).await?;
    let instance = client.state.instance.clone();
    client
        .execute(
            Command::SaveDevice {
                revision: 0,
                id: None,
                name: "Survives agent crash".into(),
                ip: "127.0.0.1".into(),
                port: "47990".into(),
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await?;
    drop(client);
    let mut client = connect(root).await?;
    assert_eq!(client.state.instance, instance);
    assert_eq!(client.state.snapshot.devices.len(), 1);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        flag.store(true, Ordering::Release);
    });
    let error = client
        .execute(
            Command::Check {
                kind: Diagnostic::RecoveryNetwork,
            },
            cancel,
        )
        .await
        .unwrap_err();
    if error
        .downcast_ref::<Failure>()
        .is_none_or(|e| e.code != "Cancelled")
    {
        return Err(error);
    }
    // Only this explicitly created, recording-only agent is terminated. Real OS
    // input is unavailable; this is never used to kill an injecting process.
    first.kill()?;
    first.wait()?;
    let disconnected = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if client.next_state().await.is_err() {
                break;
            }
        }
    })
    .await
    .is_ok();
    assert!(disconnected);
    drop(client);
    let mut second = start(executable, root)?;
    let mut restored = connect(root).await?;
    assert_ne!(restored.state.instance, instance);
    assert_eq!(restored.state.snapshot.devices.len(), 1);
    assert_eq!(restored.state.cleanup, "unknown_local");
    let cancel = Arc::new(AtomicBool::new(false));
    restored.execute(Command::StopAll, cancel.clone()).await?;
    restored.execute(Command::StopAll, cancel).await?;
    let report = serde_json::json!({"status":"PASS","checks":["separate process SDK","persisted state","reconnect without replay","cancel confirmed","agent crash disconnect detected","fresh instance after restart","snapshot reconciled","unclean restart remains unknown_local","idempotent stop"],"limits":"No OS input enabled; not an external Windows key-release supervisor test"});
    drop(restored);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    if second.try_wait()?.is_none() {
        second.kill()?;
        second.wait()?;
        return Err("agent did not complete idle shutdown".into());
    }
    std::fs::write(
        root.join("sdk-crash-result.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
#[cfg(not(windows))]
async fn sdk_crash_test(_: &Path, _: &Path) -> Result<()> {
    Err("Windows SDK test requires Windows".into())
}
