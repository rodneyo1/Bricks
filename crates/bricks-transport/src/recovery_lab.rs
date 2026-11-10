//! Same-clock, recording-only QUIC recovery characterization. Impairments are
//! injected at the authenticated application-datagram boundary, not a real NIC.
use crate::{
    auth::AuthError,
    identity::Store,
    probe,
    reliable::{self, RecoveryHost},
};
use bricks_core::{Injector, recovery::Sender};
use bricks_protocol::PhysicalKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::time::{sleep_until, timeout};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Impairment {
    pub loss_per_10000: u32,
    pub duplicate_per_10000: u32,
    pub reorder_per_10000: u32,
    pub jitter_ms: u64,
    pub burst_packets: u32,
    pub drop_final_release_once: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub events: u64,
    pub rate_hz: u32,
    pub seed: u64,
    pub impairment: Impairment,
    pub feedback_every_events: u16,
    pub feedback_max_delay_ms: u64,
    pub recovery_deadline_ms: Option<u64>,
    pub games_scheduling: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            events: 2000,
            rate_hz: 1000,
            seed: 1,
            impairment: Impairment::default(),
            feedback_every_events: 8,
            feedback_max_delay_ms: 5,
            recovery_deadline_ms: None,
            games_scheduling: false,
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<(), AuthError> {
        if self
            .recovery_deadline_ms
            .is_some_and(|ms| !(30..=150).contains(&ms))
            || !(1..=8).contains(&self.feedback_every_events)
            || !(1..=5).contains(&self.feedback_max_delay_ms)
            || self.events == 0
            || self.events > 2_000_000
            || !self.events.is_multiple_of(2)
            || ![1000, 4000, 8000].contains(&self.rate_hz)
            || self.impairment.loss_per_10000 > 200
            || self.impairment.duplicate_per_10000 > 100
            || self.impairment.reorder_per_10000 > 100
            || self.impairment.jitter_ms > 10
            || self.impairment.burst_packets > 10
        {
            return Err("invalid recovery lab configuration or bounds".into());
        }
        Ok(())
    }
}

fn recovery_policy(
    options: &Options,
    config: &bricks_config::Recovery,
    srtt_ms: u64,
) -> Result<bricks_core::recovery::Policy, AuthError> {
    let mut policy = reliable::policy(config, 1)?;
    if let Some(deadline) = options.recovery_deadline_ms {
        if deadline < u64::from(config.deadline_min_ms)
            || deadline > u64::from(config.deadline_max_ms)
        {
            return Err("selected recovery deadline exceeds configured bounds".into());
        }
        policy.recovery_ms = deadline;
    }
    // Feedback may be coalesced without delaying application. Its maximum wait
    // belongs in the retry budget; preserve at least two retry opportunities.
    policy.retry_ms = options
        .feedback_max_delay_ms
        .saturating_add(srtt_ms.saturating_mul(2))
        .saturating_add(1)
        .clamp(
            u64::from(config.retry_min_ms),
            u64::from(config.retry_max_ms),
        )
        .min(policy.recovery_ms / 3);
    Ok(policy.validate()?)
}
#[derive(Default, Serialize)]
pub struct Distribution {
    pub count: usize,
    pub mean_ns: f64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}
pub fn distribution(samples: &mut [u64]) -> Distribution {
    if samples.is_empty() {
        return Distribution::default();
    }
    samples.sort_unstable();
    let quantile = |p: usize| samples[(samples.len() * p).div_ceil(100).saturating_sub(1)];
    Distribution {
        count: samples.len(),
        mean_ns: samples.iter().map(|&v| v as f64).sum::<f64>() / samples.len() as f64,
        p50_ns: quantile(50),
        p95_ns: quantile(95),
        p99_ns: quantile(99),
        max_ns: *samples.last().unwrap(),
    }
}
#[derive(Default)]
struct Recorded {
    applications: u64,
    fidelity_errors: u64,
    receipts: Vec<u64>,
}
#[derive(Clone)]
struct Recorder {
    record: Arc<Mutex<Recorded>>,
    origin: Arc<OnceLock<Instant>>,
}
fn expected(id: u64) -> (PhysicalKey, bool) {
    (
        PhysicalKey {
            usage_page: 7,
            usage: 4 + ((id - 1) / 2 % 32) as u16,
        },
        id % 2 == 1,
    )
}
impl Injector for Recorder {
    type Error = std::convert::Infallible;
    fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), Self::Error> {
        let mut record = self.record.lock().unwrap();
        record.applications += 1;
        if expected(record.applications) != (key, down) {
            record.fidelity_errors += 1;
        }
        record.receipts.push(
            self.origin
                .get()
                .expect("recording starts after setup")
                .elapsed()
                .as_nanos() as u64,
        );
        Ok(())
    }
}

pub struct EphemeralPair {
    root: PathBuf,
    pub host: Store,
    pub client: Store,
    pub host_fp: String,
    pub client_fp: String,
}
impl EphemeralPair {
    pub fn directory(&self) -> &std::path::Path {
        &self.root
    }
    pub fn new() -> Result<Self, AuthError> {
        let mut bytes = [0; 16];
        rustls::crypto::ring::default_provider()
            .secure_random
            .fill(&mut bytes)
            .map_err(|_| "secure random unavailable")?;
        let suffix: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("bricks-recovery-lab-{suffix}"));
        std::fs::create_dir(&root)?;
        let host = Store::new(root.join("host"));
        let client = Store::new(root.join("client"));
        let mut pair = Self {
            root,
            host,
            client,
            host_fp: String::new(),
            client_fp: String::new(),
        };
        pair.host_fp = pair.host.init("bricks-host")?;
        pair.client_fp = pair.client.init("bricks-client")?;
        pair.host
            .trust(pair.client.certificate()?, &pair.client_fp, true)?;
        pair.client
            .trust(pair.host.certificate()?, &pair.host_fp, false)?;
        Ok(pair)
    }
}
impl Drop for EphemeralPair {
    fn drop(&mut self) {
        if let (Ok(path), Ok(base)) = (
            self.root.canonicalize(),
            std::env::temp_dir().canonicalize(),
        ) && path.parent() == Some(base.as_path())
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("bricks-recovery-lab-"))
        {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

struct Scheduled {
    when: Instant,
    bytes: Vec<u8>,
}
struct Link {
    options: Impairment,
    random: u64,
    pending: Vec<Scheduled>,
    attempted: u64,
    dropped: u64,
    duplicated: u64,
    peak: usize,
    final_dropped: bool,
}
impl Link {
    fn new(options: Impairment, seed: u64) -> Self {
        Self {
            options,
            random: seed.max(1),
            pending: Vec::with_capacity(1024),
            attempted: 0,
            dropped: 0,
            duplicated: 0,
            peak: 0,
            final_dropped: false,
        }
    }
    fn roll(&mut self) -> u32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random % 10000) as u32
    }
    fn send(
        &mut self,
        connection: &quinn::Connection,
        bytes: Vec<u8>,
        final_release: bool,
    ) -> Result<(), AuthError> {
        self.attempted += 1;
        if (final_release
            && self.options.drop_final_release_once
            && !std::mem::replace(&mut self.final_dropped, true))
            || (self.attempted > 10 && self.attempted <= 10 + u64::from(self.options.burst_packets))
            || self.roll() < self.options.loss_per_10000
        {
            self.dropped += 1;
            return Ok(());
        }
        let mut delay = if self.options.jitter_ms == 0 {
            0
        } else {
            u64::from(self.roll()) % (self.options.jitter_ms + 1)
        };
        if self.roll() < self.options.reorder_per_10000 {
            delay += 5;
        }
        let duplicate = self.roll() < self.options.duplicate_per_10000;
        if self.pending.len() >= 1022 {
            return Err("bounded impairment queue full".into());
        }
        if duplicate {
            self.duplicated += 1;
            self.pending.push(Scheduled {
                when: Instant::now() + Duration::from_millis(delay + 1),
                bytes: bytes.clone(),
            });
        }
        if delay == 0 {
            connection.send_datagram(bytes.into())?;
        } else {
            self.pending.push(Scheduled {
                when: Instant::now() + Duration::from_millis(delay),
                bytes,
            });
        }
        self.peak = self.peak.max(self.pending.len());
        Ok(())
    }
    fn flush(&mut self, connection: &quinn::Connection) -> Result<(), AuthError> {
        let now = Instant::now();
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].when <= now {
                let packet = self.pending.swap_remove(i);
                connection.send_datagram(packet.bytes.into())?;
            } else {
                i += 1;
            }
        }
        Ok(())
    }
    fn next_due(&self) -> Option<Instant> {
        self.pending.iter().map(|packet| packet.when).min()
    }
}

#[derive(Serialize)]
pub struct Report {
    pub io_workers: usize,
    pub runtime_flavor: &'static str,
    pub configuration_hash_schema: u16,
    pub report_schema_version: u16,
    pub measurement_class: &'static str,
    pub options: Options,
    pub status: &'static str,
    pub generated: u64,
    pub applied: u64,
    pub acknowledged: u64,
    pub missing: u64,
    pub not_generated: u64,
    pub delivered_before_recovery_deadline: u64,
    pub retries: u64,
    pub duplicates: u64,
    pub fidelity_errors: u64,
    pub cleanup_failures: usize,
    pub owned_after_cleanup: usize,
    pub send_history_peak: usize,
    pub reorder_peak: usize,
    pub impairment_queue_peak: usize,
    pub dropped_forward: u64,
    pub dropped_feedback: u64,
    pub injected_duplicates: u64,
    pub elapsed_ns: u64,
    pub achieved_rate_hz: f64,
    pub same_clock_generated_to_recording_submit: Distribution,
    pub generation_lateness: Distribution,
    pub client_enqueue_send_overhead: Distribution,
    pub host_receive_apply_overhead: Distribution,
    pub ipv4_wire_estimate_bits_per_second_both_endpoints: f64,
    pub client_udp_datagrams: u64,
    pub host_udp_datagrams: u64,
    pub estimated_ipv4_bytes: u64,
    pub stop_reason: Option<String>,
    pub config_sha256: String,
    pub effective_recovery_deadline_ms: u64,
    pub effective_retry_ms: u64,
    pub setup_elapsed_ns: u64,
    pub maintenance_wakes: u64,
    pub maximum_actor_gap_ns: u64,
    pub actor_gaps_over_25_ms: u64,
    pub environment_interrupted: bool,
    pub resources: Vec<crate::resources::Point>,
    pub process_cpu_percent_one_logical_core: Option<f64>,
    pub private_memory_peak_bytes: Option<u64>,
    pub limits: [&'static str; 5],
}
pub async fn run(options: Options, cancelled: Arc<AtomicBool>) -> Result<Report, AuthError> {
    options.validate()?;
    let pair = EphemeralPair::new()?;
    run_with_pair(&pair, options, cancelled).await
}
pub async fn run_with_pair(
    pair: &EphemeralPair,
    options: Options,
    cancelled: Arc<AtomicBool>,
) -> Result<Report, AuthError> {
    options.validate()?;
    let setup_started = Instant::now();
    let _timer_resolution = crate::timing::ActiveTimerResolution::request()?;
    let _games_task = options
        .games_scheduling
        .then(crate::timing::GamesTask::request)
        .transpose()?;
    let mut transport = probe::transport()?;
    // Negotiate bounded QUIC feedback so standalone transport ACKs can share
    // packets with application feedback. Input datagrams are never held for it.
    let mut ack_frequency = quinn::AckFrequencyConfig::default();
    ack_frequency.ack_eliciting_threshold(u32::from(options.feedback_every_events - 1).into());
    ack_frequency.max_ack_delay(Some(Duration::from_millis(options.feedback_max_delay_ms)));
    transport.ack_frequency_config(Some(ack_frequency));
    let transport = Arc::new(transport);
    let endpoint = probe::host_endpoint_with_transport(
        &pair.host,
        &pair.client_fp,
        "127.0.0.1:0".parse()?,
        transport.clone(),
    )?;
    let remote = endpoint.local_addr()?;
    let ce = probe::client_endpoint_with_transport(&pair.client, &pair.host_fp, remote, transport)?;
    let client_connect = ce.connect(remote, "bricks-host")?;
    let (client, host) = timeout(Duration::from_secs(4), async {
        tokio::join!(client_connect, async {
            endpoint
                .accept()
                .await
                .ok_or("accept stopped")?
                .await
                .map_err(AuthError::from)
        })
    })
    .await?;
    let client = client?;
    let host = host?;
    let recording_origin = Arc::new(OnceLock::new());
    let record = Arc::new(Mutex::new(Recorded {
        receipts: Vec::with_capacity(options.events as usize + 64),
        ..Recorded::default()
    }));
    let recorder = Recorder {
        record: record.clone(),
        origin: recording_origin.clone(),
    };
    let config = bricks_config::resolve(include_str!("../../../docs/examples/host.toml"), None)?;
    let srtt_ms = client.rtt().as_millis().max(1) as u64;
    let policy = recovery_policy(&options, &config.recovery, srtt_ms)?;
    let mut receiver = RecoveryHost::admit(&host, &pair.host, true, recorder, policy)?;
    let (client_grant, host_grant) = tokio::join!(
        reliable::negotiate_client(&client),
        reliable::grant_host(&host, receiver.epoch(), policy)
    );
    host_grant?;
    let (epoch, client_policy) = client_grant?;
    let handle = tokio::runtime::Handle::current();
    let (runtime_flavor, io_workers) = match handle.runtime_flavor() {
        tokio::runtime::RuntimeFlavor::CurrentThread => ("current_thread", 0),
        tokio::runtime::RuntimeFlavor::MultiThread => {
            ("multi_thread", handle.metrics().num_workers())
        }
        _ => ("unknown", handle.metrics().num_workers()),
    };
    let config_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&serde_json::json!({
            "application":config,"lab":options,"runtime_flavor":runtime_flavor,"io_workers":io_workers,"effective_retry_ms":policy.retry_ms,"effective_recovery_ms":policy.recovery_ms
        }))?)
    );
    let mut sender = Sender::new(epoch, client_policy, 0)?;
    let mut forward = Link::new(options.impairment.clone(), options.seed);
    let mut reverse_options = options.impairment.clone();
    reverse_options.drop_final_release_once = false;
    let mut reverse = Link::new(reverse_options, options.seed ^ 0x9e3779b97f4a7c15);
    let mut captures = Vec::with_capacity(options.events as usize);
    let mut enqueue = Vec::with_capacity(options.events as usize);
    let mut apply = Vec::with_capacity(options.events as usize);
    let mut generated = 0;
    let mut acknowledged = 0;
    let mut retries = 0;
    let mut duplicates = 0;
    let mut send_peak = 0;
    let mut reorder_peak = 0;
    let mut stop_reason = None;
    let mut ack_dirty = false;
    let mut ack_pending_at = Instant::now();
    let mut last_ack_through = 0;
    let initial_resource = crate::resources::sample(std::process::id()).ok();
    // Handshake, config parsing, admission and allocation are setup, not paced
    // input. Start the receiver's local timer domain and generator together.
    let origin = Instant::now();
    recording_origin.set(origin).unwrap();
    let setup_elapsed_ns = setup_started.elapsed().as_nanos() as u64;
    let monitor = crate::resources::Monitor::start(origin)?;
    let period_ns = 1_000_000_000 / u64::from(options.rate_hz);
    let maintenance = sleep_until((origin + Duration::from_millis(policy.silence_ms)).into());
    tokio::pin!(maintenance);
    let mut maintenance_wakes = 0;
    let generation_timer = sleep_until(origin.into());
    tokio::pin!(generation_timer);
    let mut generation_burst = 0;
    let mut previous_turn = origin;
    let mut maximum_actor_gap_ns = 0;
    let mut actor_gaps_over_25_ms = 0;
    // One actor owns both endpoint application state for this lab only. QUIC I/O
    // still traverses real encrypted UDP sockets; no stop-and-wait is used.
    loop {
        let turn = Instant::now();
        let gap_ns = turn.duration_since(previous_turn).as_nanos() as u64;
        maximum_actor_gap_ns = maximum_actor_gap_ns.max(gap_ns);
        actor_gaps_over_25_ms += u64::from(gap_ns > 25_000_000);
        previous_turn = turn;
        let now_ms = turn.duration_since(origin).as_millis() as u64;
        if cancelled.load(Ordering::Acquire) {
            stop_reason = Some("cancelled".into());
            break;
        }
        if let Err(e) = receiver.tick(now_ms) {
            stop_reason = Some(e.to_string());
            break;
        }
        if generated == options.events && sender.pending_count() == 0 {
            break;
        }
        if origin.elapsed() > Duration::from_secs(options.events / u64::from(options.rate_hz) + 5) {
            stop_reason = Some("lab time budget expired".into());
            break;
        }
        let mut next_maintenance = origin
            + Duration::from_millis(
                receiver
                    .next_wake_ms()
                    .unwrap_or(now_ms + policy.silence_ms),
            );
        if let Some(retry) = sender.next_wake_ms() {
            next_maintenance = next_maintenance.min(origin + Duration::from_millis(retry));
        }
        if ack_dirty {
            next_maintenance = next_maintenance
                .min(ack_pending_at + Duration::from_millis(options.feedback_max_delay_ms));
        }
        for due in [forward.next_due(), reverse.next_due()]
            .into_iter()
            .flatten()
        {
            next_maintenance = next_maintenance.min(due);
        }
        if maintenance.deadline() != next_maintenance.into() {
            maintenance.as_mut().reset(next_maintenance.into());
        }
        tokio::select! {
            biased;
            _ = &mut maintenance => {
                maintenance_wakes += 1;
                forward.flush(&client)?; reverse.flush(&host)?;
                if ack_dirty && ack_pending_at.elapsed()>=Duration::from_millis(options.feedback_max_delay_ms) {
                    let ack=receiver.ack();reverse.send(&host,reliable::encode_ack(ack).to_vec(),false)?;last_ack_through=ack.through;ack_dirty=false;
                }
                // Bounded retries per tick; no jitter buffer on fresh events.
                for _ in 0..8 {
                    match sender.retry(origin.elapsed().as_millis() as u64) {
                        Ok(Some(event)) => { retries += 1; forward.send(&client, event.encode()?.to_vec(), false)?; }
                        Ok(None) => break,
                        Err(e) => { stop_reason = Some(e.to_string()); break; }
                    }
                }
                if stop_reason.is_some() { break; }
            }

            packet = host.read_datagram() => {
                let packet = packet?; let before = Instant::now();
                match receiver.receive(&packet, origin.elapsed().as_millis() as u64) {
                    Ok(delivery) => {
                        apply.push(before.elapsed().as_nanos() as u64);
                        duplicates += u64::from(delivery.duplicate);
                        reorder_peak = reorder_peak.max(receiver.pending_count());
                        // Apply immediately. Only cumulative feedback is coalesced;
                        // duplicates/gaps get prompt feedback and idle ACKs are timed.
                        if !ack_dirty {ack_pending_at=Instant::now();}
                        ack_dirty=true;
                        if delivery.duplicate || delivery.buffered || delivery.ack.through.saturating_sub(last_ack_through)>=u64::from(options.feedback_every_events) {
                            reverse.send(&host, reliable::encode_ack(delivery.ack).to_vec(), false)?;last_ack_through=delivery.ack.through;ack_dirty=false;
                        }
                    }
                    Err(e) => { stop_reason = Some(e.to_string()); break; }
                }
            }
            packet = client.read_datagram() => {
                let ack = reliable::decode_ack(&packet?)?;
                if let Err(e) = sender.acknowledge(ack, origin.elapsed().as_millis() as u64) { stop_reason = Some(e.to_string()); break; }
                acknowledged = acknowledged.max(ack.through);
            }
            _ = &mut generation_timer, if generated < options.events => {
                let before = Instant::now(); let captured = origin.elapsed().as_nanos() as u64;
                let (key, down) = expected(generated + 1);
                let event = match sender.enqueue(key, down, captured, origin.elapsed().as_millis() as u64) { Ok(e) => e, Err(e) => { stop_reason = Some(e.to_string()); break; } };
                captures.push(captured); generated += 1;
                forward.send(&client, event.encode()?.to_vec(), generated == options.events)?;
                enqueue.push(before.elapsed().as_nanos() as u64);
                send_peak = send_peak.max(sender.pending_count());
                generation_timer.as_mut().reset((origin + Duration::from_nanos(generated.saturating_mul(period_ns))).into());
                generation_burst += 1;
                if generation_burst == 8 { generation_burst = 0; tokio::task::yield_now().await; }
            }
        }
    }
    // Include a pause inside the terminal handler too; no next loop iteration
    // exists to observe it after an error/cancellation break.
    let final_gap_ns = previous_turn.elapsed().as_nanos() as u64;
    maximum_actor_gap_ns = maximum_actor_gap_ns.max(final_gap_ns);
    actor_gaps_over_25_ms += u64::from(final_gap_ns > 25_000_000);
    let applied = receiver.ack().through;
    receiver.stop();
    let cleanup_failures = receiver.cleanup_failures_observed;
    let owned_after_cleanup = receiver.pressed_count();
    let elapsed_ns = origin.elapsed().as_nanos() as u64;
    let final_resource = crate::resources::sample(std::process::id()).ok();
    let mut resources = monitor.finish();
    resources.retain(|point| point.elapsed_ns <= elapsed_ns);
    if let Some(sample) = initial_resource {
        resources.insert(
            0,
            crate::resources::Point {
                elapsed_ns: 0,
                sample,
            },
        );
    }
    if let Some(sample) = final_resource {
        resources.push(crate::resources::Point { elapsed_ns, sample });
    }
    let process_cpu_percent_one_logical_core =
        initial_resource.zip(final_resource).map(|(start, end)| {
            end.cpu_ns.saturating_sub(start.cpu_ns) as f64 / elapsed_ns as f64 * 100.0
        });
    let private_memory_peak_bytes = resources.iter().map(|p| p.sample.private_bytes).max();
    let record = record.lock().unwrap();
    let mut latencies: Vec<u64> = record
        .receipts
        .iter()
        .zip(&captures)
        .take(applied as usize)
        .map(|(receipt, captured)| receipt.saturating_sub(*captured))
        .collect();
    let cs = client.stats().udp_tx;
    let hs = host.stats().udp_tx;
    let estimated_bytes = cs.bytes + hs.bytes + (cs.datagrams + hs.datagrams) * 28;
    let mut generation_lateness: Vec<u64> = captures
        .iter()
        .enumerate()
        .map(|(i, ns)| ns.saturating_sub(i as u64 * period_ns))
        .collect();
    let not_generated = options.events - generated;
    let delivered_before_recovery_deadline = latencies
        .iter()
        .filter(|&&ns| ns < policy.recovery_ms * 1_000_000)
        .count() as u64;
    let report = Report {
        io_workers,
        runtime_flavor,
        configuration_hash_schema: 3,
        report_schema_version: 2,
        measurement_class: "loopback_quic_recovery_recording",
        options,
        status: if maximum_actor_gap_ns > 1_000_000_000 {
            "INTERRUPTED"
        } else if stop_reason.is_none()
            && generated == applied
            && applied == acknowledged
            && record.fidelity_errors == 0
            && cleanup_failures == 0
            && owned_after_cleanup == 0
        {
            "PASS"
        } else {
            "STOPPED"
        },
        generated,
        applied,
        acknowledged,
        missing: generated - applied,
        not_generated,
        delivered_before_recovery_deadline,
        retries,
        duplicates,
        fidelity_errors: record.fidelity_errors,
        cleanup_failures,
        owned_after_cleanup,
        send_history_peak: send_peak,
        reorder_peak,
        impairment_queue_peak: forward.peak.max(reverse.peak),
        dropped_forward: forward.dropped,
        dropped_feedback: reverse.dropped,
        injected_duplicates: forward.duplicated + reverse.duplicated,
        elapsed_ns,
        achieved_rate_hz: generated as f64 * 1e9 / elapsed_ns as f64,
        same_clock_generated_to_recording_submit: distribution(&mut latencies),
        generation_lateness: distribution(&mut generation_lateness),
        client_enqueue_send_overhead: distribution(&mut enqueue),
        host_receive_apply_overhead: distribution(&mut apply),
        ipv4_wire_estimate_bits_per_second_both_endpoints: estimated_bytes as f64 * 8e9
            / elapsed_ns as f64,
        client_udp_datagrams: cs.datagrams,
        host_udp_datagrams: hs.datagrams,
        estimated_ipv4_bytes: estimated_bytes,
        stop_reason,
        config_sha256,
        effective_recovery_deadline_ms: policy.recovery_ms,
        effective_retry_ms: policy.retry_ms,
        setup_elapsed_ns,
        maintenance_wakes,
        maximum_actor_gap_ns,
        actor_gaps_over_25_ms,
        environment_interrupted: maximum_actor_gap_ns > 1_000_000_000,
        resources,
        process_cpu_percent_one_logical_core,
        private_memory_peak_bytes,
        limits: [
            "Synthetic generation to recording submission; no OS capture/injection or physical gaming result",
            "Single-clock single-process actor for both loopback endpoints; not a two-PC overhead sum",
            "Seeded application datagram impairment in both directions, not measured NIC/router loss",
            "Complete bounded receipt vectors; timing overhead included; resource history retains most recent 600 points plus endpoints",
            "IPv4 estimate includes QUIC UDP payload and IP/UDP headers; excludes Ethernet and tunnels",
        ],
    };
    client.close(0u32.into(), b"lab done");
    endpoint.close(0u32.into(), b"lab done");
    ce.close(0u32.into(), b"lab done");
    Ok(report)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_quic_recovers_final_release_and_bidirectional_loss_without_duplicate_application()
    {
        let pair = EphemeralPair::new().unwrap();
        for impairment in [
            Impairment {
                drop_final_release_once: true,
                ..Default::default()
            },
            Impairment {
                loss_per_10000: 100,
                duplicate_per_10000: 100,
                reorder_per_10000: 100,
                ..Default::default()
            },
        ] {
            let report = run_with_pair(
                &pair,
                Options {
                    events: 1000,
                    impairment,
                    ..Default::default()
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap();
            assert_eq!(report.status, "PASS", "{:?}", report.stop_reason);
            assert_eq!(report.applied, 1000);
            assert_eq!(report.fidelity_errors, 0);
            assert_eq!(report.owned_after_cleanup, 0);
            assert!(report.retries > 0);
        }
    }
    #[test]
    fn bounds_and_nearest_rank_metrics_are_explicit() {
        assert!(
            Options {
                events: 3,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        let stats = distribution(&mut [9, 1, 2]);
        assert_eq!(stats.p50_ns, 2);
        assert_eq!(stats.p99_ns, 9);
    }
    #[test]
    fn feedback_wait_does_not_trigger_healthy_retries_or_relax_history_cutoff() {
        let config =
            bricks_config::resolve(include_str!("../../../docs/examples/host.toml"), None).unwrap();
        for deadline in [30, 150] {
            for rtt in [1, 4, 100] {
                let policy = recovery_policy(
                    &Options {
                        recovery_deadline_ms: Some(deadline),
                        ..Default::default()
                    },
                    &config.recovery,
                    rtt,
                )
                .unwrap();
                assert_eq!(policy.recovery_ms, deadline);
                assert!(policy.retry_ms > Options::default().feedback_max_delay_ms);
                assert!(policy.retry_ms <= deadline / 3);
                assert!((2..=20).contains(&policy.retry_ms));
            }
        }
    }
}
