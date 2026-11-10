//! Offline, virtual-time fault scenarios exercising the real recovery state machines.
//! No socket, identity store, input capture, OS injection or sleeps are used.

use crate::auth::AuthError;
use bricks_core::{
    Injector,
    recovery::{Fault, Policy, Receiver, Sender},
};
use bricks_protocol::{Epoch, PhysicalKey, Transition};
use serde::Serialize;
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Recording {
    events: Vec<(PhysicalKey, bool)>,
    fail_releases: bool,
}
#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Recording>>);
impl Injector for Recorder {
    type Error = &'static str;
    fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), Self::Error> {
        let mut record = self.0.borrow_mut();
        if !down && record.fail_releases {
            return Err("deliberate cleanup failure");
        }
        record.events.push((key, down));
        Ok(())
    }
}

#[derive(Serialize)]
pub struct CaseReport {
    pub scenario: &'static str,
    pub passed: bool,
    pub applied_through: u64,
    pub retries_sent: usize,
    pub duplicates_observed: usize,
    pub expected_stop: Option<String>,
    pub expected_unapplied_transitions: u64,
    pub cleanup_failures_observed: usize,
    pub owned_keys_after_cleanup: usize,
}

#[derive(Serialize)]
pub struct Report {
    pub report_schema_version: u16,
    pub measurement_class: &'static str,
    pub status: &'static str,
    pub scenarios_passed: usize,
    pub scenarios_total: usize,
    pub simulated_srtt_ms: u64,
    pub retry_interval_ms: u64,
    pub recovery_deadline_ms: u64,
    pub config_sha256: String,
    pub resolved_config: bricks_config::ResolvedConfig,
    pub scenarios: Vec<CaseReport>,
    pub limitations: [&'static str; 4],
}

fn require(condition: bool, message: &str) -> Result<(), AuthError> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}

/// Compute once for this diagnostic. Live RTT adaptation/negotiation is not wired.
fn policy(config: &bricks_config::Recovery, srtt_ms: u64) -> Result<Policy, Fault> {
    Policy {
        send_capacity: usize::from(config.send_history_capacity),
        receive_capacity: usize::from(config.receive_buffer_capacity),
        retry_ms: (srtt_ms / 4).clamp(
            u64::from(config.retry_min_ms),
            u64::from(config.retry_max_ms),
        ),
        recovery_ms: srtt_ms.saturating_mul(2).saturating_add(10).clamp(
            u64::from(config.deadline_min_ms),
            u64::from(config.deadline_max_ms),
        ),
        silence_ms: u64::from(config.silence_timeout_ms),
    }
    .validate()
}

struct Scenario {
    name: &'static str,
    tx: Sender,
    rx: Receiver<Recorder>,
    epoch: Epoch,
    retries: usize,
    duplicates: usize,
}
impl Scenario {
    fn new(name: &'static str, policy: Policy, epoch: Epoch) -> Result<Self, AuthError> {
        Ok(Self {
            name,
            tx: Sender::new(epoch, policy, 0)?,
            rx: Receiver::new(epoch, Recorder::default(), 16, policy, 0)?,
            epoch,
            retries: 0,
            duplicates: 0,
        })
    }
    fn capture(&mut self, down: bool, now: u64) -> Result<Transition, AuthError> {
        Ok(self.tx.enqueue(
            PhysicalKey {
                usage_page: 7,
                usage: 4,
            },
            down,
            now * 1_000_000,
            now,
        )?)
    }
    fn deliver(&mut self, event: Transition, now: u64, deliver_ack: bool) -> Result<(), AuthError> {
        // Exercise the existing reference wire codec too, not native struct casting.
        let decoded = Transition::decode(&event.encode()?)?;
        let delivery = self
            .rx
            .receive(decoded, now)
            .map_err(|e| format!("{}: {e:?}", self.name))?;
        self.duplicates += usize::from(delivery.duplicate);
        if deliver_ack {
            self.tx.acknowledge(delivery.ack, now)?;
        }
        Ok(())
    }
    fn retry(&mut self, now: u64) -> Result<(), AuthError> {
        let event = self.tx.retry(now)?.ok_or("expected a due retry")?;
        self.retries += 1;
        self.deliver(event, now, true)
    }
    fn complete(
        mut self,
        expected_trace: &[bool],
        expected_applied: u64,
        stop: Option<Fault>,
        unapplied: u64,
        cleanup_failures: usize,
    ) -> Result<CaseReport, AuthError> {
        require(
            self.rx.ack().through == expected_applied,
            "unexpected applied history",
        )?;
        if stop.is_some() {
            require(
                self.rx.pressed_count() == 0 && self.rx.pending_count() == 0,
                "fault handling did not release/discard recorded state before final cleanup",
            )?;
        }
        if stop.is_none() {
            require(
                self.tx.pending_count() == 0,
                "unacknowledged history at successful completion",
            )?;
        }
        require(self.rx.stop().is_empty(), "unexpected cleanup failure")?;
        self.tx.stop();
        let trace: Vec<_> = self
            .rx
            .injector()
            .0
            .borrow()
            .events
            .iter()
            .map(|(_, down)| *down)
            .collect();
        require(
            trace == expected_trace,
            &format!("{}: wrong injection history {trace:?}", self.name),
        )?;
        require(
            self.rx.pressed_count() == 0 && self.rx.pending_count() == 0,
            "owned or buffered state remains",
        )?;
        // Even a healthy explicit stop invalidates the epoch.
        let event = Transition {
            epoch: self.epoch,
            id: expected_applied.saturating_add(1),
            captured_ns: 0,
            key: PhysicalKey {
                usage_page: 7,
                usage: 4,
            },
            down: true,
        };
        require(
            self.rx.receive(event, 1000).unwrap_err().fault == Fault::Inactive,
            "stopped receiver accepted work",
        )?;
        Ok(CaseReport {
            scenario: self.name,
            passed: true,
            applied_through: expected_applied,
            retries_sent: self.retries,
            duplicates_observed: self.duplicates,
            expected_stop: stop.map(|f| f.to_string()),
            expected_unapplied_transitions: unapplied,
            cleanup_failures_observed: cleanup_failures,
            owned_keys_after_cleanup: self.rx.pressed_count(),
        })
    }
}

pub fn run(config: bricks_config::ResolvedConfig) -> Result<Report, AuthError> {
    let srtt = 8;
    let limits = policy(&config.recovery, srtt)?;
    require(
        limits.send_capacity >= 2,
        "fault suite requires send_history_capacity >= 2; capacity-one behavior is covered by unit tests",
    )?;
    let epoch = Epoch([0x41; 16]); // Deliberately synthetic, not a real ownership grant.
    let mut reports = Vec::new();

    let mut s = Scenario::new("healthy_immediate_delivery", limits, epoch)?;
    let down = s.capture(true, 0)?;
    s.deliver(down, 0, true)?;
    let up = s.capture(false, 0)?;
    s.deliver(up, 0, true)?;
    reports.push(s.complete(&[true, false], 2, None, 0, 0)?);

    let mut s = Scenario::new("lost_press_recovered_before_release", limits, epoch)?;
    let _lost_down = s.capture(true, 0)?;
    let up = s.capture(false, 0)?;
    s.deliver(up, 0, true)?;
    require(
        s.rx.ack().through == 0 && s.rx.pressed_count() == 0,
        "release crossed a missing press",
    )?;
    s.retry(limits.retry_ms)?;
    reports.push(s.complete(&[true, false], 2, None, 0, 0)?);

    let mut s = Scenario::new(
        "lost_final_release_retried_without_new_input",
        limits,
        epoch,
    )?;
    let down = s.capture(true, 0)?;
    s.deliver(down, 0, true)?;
    let _lost_up = s.capture(false, 0)?;
    s.retry(limits.retry_ms)?;
    reports.push(s.complete(&[true, false], 2, None, 0, 0)?);

    let mut s = Scenario::new("reordered_press_and_release", limits, epoch)?;
    let down = s.capture(true, 0)?;
    let up = s.capture(false, 0)?;
    s.deliver(up, 0, true)?;
    s.deliver(down, 0, true)?;
    reports.push(s.complete(&[true, false], 2, None, 0, 0)?);

    let mut s = Scenario::new("duplicate_packets_do_not_repeat_injection", limits, epoch)?;
    let down = s.capture(true, 0)?;
    s.deliver(down, 0, true)?;
    s.deliver(down, 0, true)?;
    let up = s.capture(false, 0)?;
    s.deliver(up, 0, true)?;
    s.deliver(up, 0, true)?;
    require(s.duplicates == 2, "duplicate packets were not classified")?;
    reports.push(s.complete(&[true, false], 2, None, 0, 0)?);

    let mut s = Scenario::new("lost_ack_triggers_safe_duplicate_retry", limits, epoch)?;
    let down = s.capture(true, 0)?;
    s.deliver(down, 0, false)?;
    s.retry(limits.retry_ms)?;
    let up = s.capture(false, limits.retry_ms)?;
    s.deliver(up, limits.retry_ms, true)?;
    require(
        s.duplicates == 1,
        "ACK-loss retry was not classified as duplicate",
    )?;
    reports.push(s.complete(&[true, false], 2, None, 0, 0)?);

    let mut s = Scenario::new(
        "unrecoverable_gap_releases_and_discards_history",
        limits,
        epoch,
    )?;
    let down = s.capture(true, 0)?;
    s.deliver(down, 0, true)?;
    let _lost_up = s.capture(false, 0)?;
    let later_down = s.capture(true, 0)?;
    s.deliver(later_down, 0, true)?;
    let error = s.rx.tick(limits.recovery_ms).unwrap_err();
    require(
        error.fault == Fault::Deadline && error.cleanup_failures.is_empty(),
        "wrong gap timeout result",
    )?;
    reports.push(s.complete(&[true, false], 1, Some(Fault::Deadline), 2, 0)?);

    let mut s = Scenario::new("total_silence_releases_recorded_key", limits, epoch)?;
    let down = s.capture(true, 0)?;
    s.deliver(down, 0, true)?;
    let error = s.rx.tick(limits.silence_ms).unwrap_err();
    require(
        error.fault == Fault::Silence && error.cleanup_failures.is_empty(),
        "wrong silence timeout result",
    )?;
    reports.push(s.complete(&[true, false], 1, Some(Fault::Silence), 0, 0)?);

    let mut s = Scenario::new("send_history_overflow_stops_epoch", limits, epoch)?;
    for i in 0..limits.send_capacity {
        s.capture(i % 2 == 0, 0)?;
    }
    require(
        s.capture(false, 0).unwrap_err().to_string() == Fault::SendOverflow.to_string(),
        "send overflow was not explicit",
    )?;
    require(
        s.tx.retry(limits.retry_ms) == Err(Fault::Inactive),
        "sender continued after overflow",
    )?;
    reports.push(s.complete(
        &[],
        0,
        Some(Fault::SendOverflow),
        limits.send_capacity as u64 + 1,
        0,
    )?);

    let mut s = Scenario::new(
        "receive_window_overflow_releases_recorded_key",
        limits,
        epoch,
    )?;
    let down = s.capture(true, 0)?;
    s.deliver(down, 0, true)?;
    let outside_window = Transition {
        id: limits.receive_capacity as u64 + 3,
        down: false,
        ..down
    };
    let error = s.rx.receive(outside_window, 0).unwrap_err();
    require(
        error.fault == Fault::ReceiveOverflow && error.cleanup_failures.is_empty(),
        "receive window did not fail closed",
    )?;
    reports.push(s.complete(
        &[true, false],
        1,
        Some(Fault::ReceiveOverflow),
        limits.receive_capacity as u64 + 2,
        0,
    )?);

    // A stopped epoch is never reused. Check cleanup of old ownership, then show
    // old datagrams/ACKs cannot apply to or clear the newly constructed state.
    let mut old = Scenario::new("old_session", limits, epoch)?;
    let stale = old.capture(true, 0)?;
    old.deliver(stale, 0, true)?;
    require(old.rx.stop().is_empty(), "old session cleanup failed")?;
    require(old.rx.pressed_count() == 0, "old session was not neutral")?;
    old.tx.stop();
    let mut s = Scenario::new(
        "fresh_epoch_rejects_old_input_and_feedback",
        limits,
        Epoch([0x42; 16]),
    )?;
    let down = s.capture(true, 0)?;
    require(
        s.rx.receive(stale, 0).unwrap_err().fault == Fault::Epoch,
        "old input accepted",
    )?;
    require(
        s.tx.acknowledge(old.rx.ack(), 0) == Err(Fault::Epoch) && s.tx.pending_count() == 1,
        "old ACK cleared fresh history",
    )?;
    s.deliver(down, 0, true)?;
    let up = s.capture(false, 0)?;
    s.deliver(up, 0, true)?;
    reports.push(s.complete(&[true, false], 2, None, 0, 0)?);

    let mut s = Scenario::new("cleanup_failure_is_reported_and_retryable", limits, epoch)?;
    let down = s.capture(true, 0)?;
    s.deliver(down, 0, true)?;
    s.rx.injector().0.borrow_mut().fail_releases = true;
    let error = s.rx.tick(limits.silence_ms).unwrap_err();
    require(
        error.cleanup_failures.len() == 1 && s.rx.pressed_count() == 1,
        "cleanup failure lost owned state",
    )?;
    s.rx.injector().0.borrow_mut().fail_releases = false;
    require(
        s.rx.stop().is_empty() && s.rx.pressed_count() == 0,
        "cleanup retry did not release key",
    )?;
    reports.push(s.complete(&[true, false], 1, Some(Fault::Silence), 0, 1)?);

    Ok(Report {
        report_schema_version: 1,
        measurement_class: "deterministic_recovery_simulation",
        status: "PASS",
        scenarios_passed: reports.len(),
        scenarios_total: reports.len(),
        simulated_srtt_ms: srtt,
        retry_interval_ms: limits.retry_ms,
        recovery_deadline_ms: limits.recovery_ms,
        config_sha256: config.fingerprint()?,
        resolved_config: config,
        scenarios: reports,
        limitations: [
            "Virtual local milliseconds; no real network, scheduler, timing or latency benchmark",
            "Synthetic keyboard transitions and recording injector; no capture, SendInput or game acceptance",
            "Recovery engine is not connected to the authenticated QUIC probe yet",
            "No motion barriers, snapshots/resync handshake, heartbeat, external crash watchdog or two-PC proof",
        ],
    })
}

pub fn run_cli_if_requested() -> Result<bool, AuthError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("recovery-selftest") {
        return Ok(false);
    }
    if args.len() > 2 {
        return Err("usage: recovery-selftest [config.toml]".into());
    }
    let source = match args.get(1) {
        Some(path) => std::fs::read_to_string(path)?,
        None => include_str!("../../../docs/examples/host.toml").to_owned(),
    };
    let report = run(bricks_config::resolve(&source, None)?)?;
    eprintln!(
        "PASS: {}/{} recovery scenarios; simulated input only",
        report.scenarios_passed, report.scenarios_total
    );
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_fault_suite_accepts_example_and_nondefault_limits() {
        let source = include_str!("../../../docs/examples/host.toml");
        for text in [
            source.to_owned(),
            source
                .replace("silence_timeout_ms = 250", "silence_timeout_ms = 500")
                .replace("send_history_capacity = 256", "send_history_capacity = 2")
                .replace(
                    "receive_buffer_capacity = 64",
                    "receive_buffer_capacity = 1",
                )
                .replace("retry_min_ms = 2", "retry_min_ms = 3")
                .replace("retry_max_ms = 20", "retry_max_ms = 10")
                .replace("deadline_min_ms = 30", "deadline_min_ms = 40")
                .replace("deadline_max_ms = 150", "deadline_max_ms = 100"),
        ] {
            let report = run(bricks_config::resolve(&text, None).unwrap()).unwrap();
            assert_eq!(report.scenarios_passed, 12);
            assert!(
                report
                    .scenarios
                    .iter()
                    .all(|s| s.passed && s.owned_keys_after_cleanup == 0)
            );
        }
    }

    #[test]
    fn rtt_estimate_is_clamped_without_arithmetic_overflow() {
        let config = bricks_config::Recovery::default();
        let short = policy(&config, 0).unwrap();
        assert_eq!((short.retry_ms, short.recovery_ms), (2, 30));
        let long = policy(&config, u64::MAX).unwrap();
        assert_eq!((long.retry_ms, long.recovery_ms), (20, 150));
    }
}
