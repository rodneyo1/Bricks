use bricks_protocol::{Epoch, PhysicalKey, Transition};
use bricks_transport::LoopbackReference;
use serde_json::json;
use std::{hint::black_box, process::Command, time::Instant};

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        println!(
            "Usage: bricks-bench loopback-reference [--samples <100..1000000>] [--config <path.toml>]"
        );
        println!(
            "Software UDP echo only; no OS input capture, injection, or physical latency measurement."
        );
        return Ok(());
    };
    if command != "loopback-reference" {
        return Err("expected loopback-reference".into());
    }
    let mut samples: usize = 10_000;
    let mut config_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--samples" => samples = args.next().ok_or("--samples requires a count")?.parse()?,
            "--config" => config_path = Some(args.next().ok_or("--config requires a path")?),
            _ => return Err(format!("unknown argument {arg:?}").into()),
        }
    }
    if !(100..=1_000_000).contains(&samples) {
        return Err("samples must be 100..=1000000".into());
    }
    let config = if let Some(path) = config_path {
        Some(bricks_config::resolve(
            &std::fs::read_to_string(path)?,
            None,
        )?)
    } else {
        None
    };
    let transport = LoopbackReference::new()?;
    let origin = Instant::now();
    let make_event = |id| Transition {
        epoch: Epoch([0x42; 16]),
        id,
        captured_ns: origin.elapsed().as_nanos() as u64,
        key: PhysicalKey {
            usage_page: 7,
            usage: 4,
        },
        down: id % 2 == 1,
    };
    for id in 1..=1000 {
        black_box(transport.roundtrip(make_event(id))?);
    }
    let mut times = Vec::with_capacity(samples);
    for index in 0..samples {
        let event = make_event(index as u64 + 1001);
        let start = Instant::now();
        black_box(transport.roundtrip(event)?);
        times.push(start.elapsed().as_nanos() as u64);
    }
    times.sort_unstable();
    let mean = times.iter().map(|&n| n as f64).sum::<f64>() / samples as f64;
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
    let config_hash = config.as_ref().map(|c| c.fingerprint()).transpose()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "report_schema_version": 1,
            "measurement_class": "loopback_software_reference",
            "boundary": { "start": "before client encode/send", "end": "after echo decode/identity validation" },
            "metric": "same_thread_udp_codec_echo_round_trip",
            "backend": "loopback_udp_reference_unauthenticated",
            "samples": samples, "warmup_samples": 1000,
            "latency_ns": { "mean": mean, "p50": percentile(&times, 50), "p95": percentile(&times, 95), "p99": percentile(&times, 99) },
            "successful_roundtrips": samples, "mismatched_events": 0,
            "uncertainty": {
                "clock": "std::time::Instant, one machine",
                "cross_clock_offset": "not applicable",
                "clock_resolution_ns": null,
                "confidence_interval": null,
                "status": "uncalibrated exploratory reference; not an acceptance benchmark",
                "scheduler_jitter": "included in measured latency"
            },
            "manifest": {
                "package_version": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                "debug_assertions": cfg!(debug_assertions), "rustc": rustc,
                "source_revision": revision,
                "hardware_inventory": null,
                "config_sha256": config_hash,
                "resolved_application_config": config,
                "reference_settings": { "bind": "IPv4 loopback only", "payload_bytes": bricks_protocol::TRANSITION_BYTES, "timeout_ms": 2000 },
                "application_config_affects_reference_transport": false
            },
            "limitations": [
                "No OS input capture, SendInput call, or game consumption is measured",
                "Not one-way latency, not a two-PC result, not input-to-photon",
                "No authentication/recovery equivalence to QUIC is established",
                "No LAN acceptance target or competitor claim can be evaluated from this run"
            ]
        }))?
    );
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("benchmark failed; no valid report emitted: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_rank_percentiles_handle_small_and_uneven_samples() {
        assert_eq!(percentile(&[7], 99), 7);
        assert_eq!(percentile(&[1, 2, 3], 50), 2);
        assert_eq!(percentile(&(1..=100).collect::<Vec<_>>(), 95), 95);
    }
}
