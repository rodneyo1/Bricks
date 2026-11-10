//! Explicit v1 live sessions. Authentication, grant and crash protection precede
//! input. The same bounded recovery and motion barriers serve recording checks.
use crate::{
    auth::AuthError,
    identity::Store,
    probe,
    reliable::{self, RecoveryHost},
    resources, timing,
};
use bricks_core::{
    Injector,
    motion::{Adapter, MotionInjector, State},
    recovery::Sender,
};
use bricks_input_windows::{
    live::{CaptureEvent, Signal},
    watchdog::{Guard, SupervisedInjector},
};
use bricks_protocol::{
    Epoch, PhysicalKey,
    live::{CRITICAL_BYTES, Critical, Motion, Totals},
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::{
    sync::mpsc,
    time::{sleep_until, timeout},
};

const PROTOCOL: &[u8; 8] = b"BRKLIV01";
struct TrustWatch {
    stop: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl TrustWatch {
    fn start(store: Store, peer: String, signal: Arc<Signal>) -> std::io::Result<Self> {
        let stop = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let worker_stop = stop.clone();
        let worker = std::thread::Builder::new()
            .name("bricks-trust".into())
            .spawn(move || {
                loop {
                    if store.trusted_certificate(&peer).is_err() || store.refresh_leases().is_err()
                    {
                        signal.stop("Peer trust was revoked or could not be refreshed.");
                        break;
                    }
                    let guard = worker_stop.0.lock().unwrap();
                    let (guard, _) = worker_stop
                        .1
                        .wait_timeout_while(guard, Duration::from_millis(100), |stop| !*stop)
                        .unwrap();
                    if *guard {
                        break;
                    }
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for TrustWatch {
    fn drop(&mut self) {
        *self.stop.0.lock().unwrap() = true;
        self.stop.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn transport() -> Result<Arc<quinn::TransportConfig>, AuthError> {
    let mut transport = probe::transport()?;
    let mut frequency = quinn::AckFrequencyConfig::default();
    frequency.ack_eliciting_threshold(7u32.into());
    frequency.max_ack_delay(Some(Duration::from_millis(5)));
    transport.ack_frequency_config(Some(frequency));
    Ok(Arc::new(transport))
}
fn active_policy(
    config: &bricks_config::ResolvedConfig,
    srtt: u64,
) -> Result<bricks_core::recovery::Policy, AuthError> {
    let mut policy = reliable::policy(&config.recovery, srtt)?;
    policy.retry_ms = (5 + srtt.saturating_mul(2) + 1)
        .clamp(
            config.recovery.retry_min_ms.into(),
            config.recovery.retry_max_ms.into(),
        )
        .min(policy.recovery_ms / 3);
    Ok(policy.validate()?)
}
pub fn runtime(
    config: &bricks_config::ResolvedConfig,
) -> Result<tokio::runtime::Runtime, AuthError> {
    let mut builder = if config.execution.io_workers == 0 {
        tokio::runtime::Builder::new_current_thread()
    } else {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.worker_threads(usize::from(config.execution.io_workers));
        builder
    };
    Ok(builder.enable_all().build()?)
}
#[derive(Default)]
struct Recording {
    keys: u64,
    motion: u64,
}
impl Injector for Recording {
    type Error = std::convert::Infallible;
    fn submit(&mut self, _: PhysicalKey, _: bool) -> Result<(), Self::Error> {
        self.keys += 1;
        Ok(())
    }
}
impl MotionInjector for Recording {
    fn submit_motion(&mut self, _: [i32; 4]) -> Result<(), Self::Error> {
        self.motion += 1;
        Ok(())
    }
}

async fn receive<I: MotionInjector>(
    connection: &quinn::Connection,
    store: &Store,
    approved: bool,
    config: &bricks_config::ResolvedConfig,
    make: impl FnOnce(Epoch) -> Result<I, AuthError>,
    mut alive: impl FnMut() -> Result<(), AuthError>,
    signal: &Signal,
) -> Result<(serde_json::Value, Option<quinn::SendStream>), AuthError>
where
    I::Error: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
{
    let policy = active_policy(config, connection.rtt().as_millis().max(1) as u64)?;
    let mut shared = None;
    let mut receiver = RecoveryHost::admit_with(connection, store, approved, policy, |epoch| {
        let state = Rc::new(RefCell::new(State::new(
            epoch,
            make(epoch)?,
            policy.receive_capacity,
        )));
        shared = Some(state.clone());
        Ok(Adapter(state))
    })?;
    let state = shared.unwrap();
    reliable::grant_host_protocol(connection, receiver.epoch(), policy, PROTOCOL).await?;
    let _timer = timing::ActiveTimerResolution::request()?;
    let _games = config
        .execution
        .games_scheduling
        .then(timing::GamesTask::request)
        .transpose()?;
    let initial = resources::sample(std::process::id()).ok();
    let origin = Instant::now();
    let monitor = resources::Monitor::start(origin)?;
    let mut pulses = tokio::time::interval(Duration::from_millis(20));
    pulses.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let maintenance = sleep_until((origin + Duration::from_millis(policy.silence_ms)).into());
    tokio::pin!(maintenance);
    let mut ack_due = None;
    let mut last_ack = 0;
    let mut packets = 0u64;
    let mut stale = 0u64;
    let mut criticals = 0u64;
    let mut last_pulse = origin - Duration::from_secs(1);
    let control = async {
        let (send, mut recv) = connection.accept_bi().await?;
        let bytes = timeout(Duration::from_secs(1), recv.read_to_end(88)).await??;
        Ok::<_, AuthError>((send, bytes))
    };
    tokio::pin!(control);
    signal.status("Connected. Keyboard/mouse sharing approved; waiting for input.");
    println!("Bricks live input connected.");
    signal.ready.store(true, Ordering::Release);
    let mut end_stream = None;
    let reason: String = loop {
        if signal.stopped.load(Ordering::Acquire) {
            break "stopped_by_owner".into();
        }
        let now = origin.elapsed().as_millis() as u64;
        if let Err(error) = receiver.tick(now) {
            break error.to_string();
        }
        if last_pulse.elapsed() >= Duration::from_millis(20) {
            if let Err(error) = alive() {
                break error.to_string();
            }
            last_pulse = Instant::now();
        }
        let mut wake = origin + Duration::from_millis(receiver.next_wake_ms().unwrap_or(now + 250));
        if let Some(due) = ack_due {
            wake = wake.min(due);
        }
        maintenance.as_mut().reset(wake.into());
        tokio::select! {
            biased;
            packet=connection.read_datagram()=>{
                let bytes=match packet {Ok(bytes)=>bytes,Err(error)=>break format!("disconnected: {error}")}; packets+=1;
                let now=origin.elapsed().as_millis() as u64;
                let received=(||->Result<(),AuthError>{
                    if bytes.len()==CRITICAL_BYTES {
                        let critical=Critical::decode(&bytes)?;
                        if critical.transition.epoch!=receiver.epoch(){stale+=1;return Ok(());}
                        bricks_input_windows::key_record(critical.transition.key,critical.transition.down)?;
                        state.borrow_mut().stage(critical,receiver.ack().through)?;
                        let delivery=receiver.receive_event(critical.transition,now)?;criticals+=1;
                        if ack_due.is_none(){ack_due=Some(Instant::now()+Duration::from_millis(5));}
                        if delivery.duplicate||delivery.buffered||delivery.ack.through.saturating_sub(last_ack)>=8 {
                            connection.send_datagram(reliable::encode_ack(delivery.ack).to_vec().into())?;last_ack=delivery.ack.through;ack_due=None;
                        }
                    }else{
                        let (motion,heartbeat)=Motion::decode(&bytes)?;
                        if motion.epoch!=receiver.epoch(){stale+=1;return Ok(());}
                        receiver.heartbeat(&reliable::encode_heartbeat(motion.epoch,motion.segment),now)?;
                        state.borrow_mut().motion(motion)?;
                        if heartbeat {connection.send_datagram(reliable::encode_ack(receiver.ack()).to_vec().into())?;last_ack=receiver.ack().through;ack_due=None;}
                    }
                    Ok(())
                })();
                if let Err(error)=received{break error.to_string();}
            }
            result=&mut control=>{
                let (send,bytes)=match result{Ok(result)=>result,Err(error)=>break error.to_string()};
                let result=(||->Result<(),AuthError>{
                    if bytes.len()!=88||&bytes[..8]!=b"BLSTOP01"{return Err("invalid live stop".into());}
                    let (final_motion,_)=Motion::decode(&bytes[8..])?;
                    if final_motion.epoch!=receiver.epoch()||final_motion.segment!=receiver.ack().through{return Err("stop has unapplied history".into());}
                    state.borrow_mut().motion(final_motion)?;Ok(())
                })();
                if let Err(error)=result{break error.to_string();}
                end_stream=Some(send);break "completed".into();
            }
            _=&mut maintenance=>{
                if ack_due.is_some_and(|due|due<=Instant::now()) {
                    if let Err(error)=connection.send_datagram(reliable::encode_ack(receiver.ack()).to_vec().into()){break error.to_string();}
                    last_ack=receiver.ack().through;ack_due=None;
                }
            }
            _=pulses.tick()=>{}
        }
    };
    let applied = receiver.ack().through;
    let owned_before = receiver.pressed_count();
    receiver.stop();
    let owned_after = receiver.pressed_count();
    let failures = receiver.cleanup_failures_observed;
    let duration_ns = origin.elapsed().as_nanos() as u64;
    let final_resource = resources::sample(std::process::id()).ok();
    let points = monitor.finish();
    let (totals, motion_submissions) = {
        let snapshot = state.borrow();
        (snapshot.submitted_totals, snapshot.motion_submissions)
    };
    if let Some(stream) = end_stream.as_mut() {
        stream
            .write_all(if owned_after == 0 && failures == 0 {
                b"STOPPED1"
            } else {
                b"UNSAFE01"
            })
            .await?;
        stream.finish()?;
    }
    signal.ready.store(false, Ordering::Release);
    signal.stop(format!(
        "Session ended: {reason}; cleanup failures: {failures}"
    ));
    Ok((
        serde_json::json!({"report_schema_version":1,"role":"host","measurement_class":"live_input_host_local_accounting","reason":reason,"duration_ns":duration_ns,"received_datagrams":packets,"critical_datagrams":criticals,"critical_applied":applied,"stale_discards":stale,"motion_submissions":motion_submissions,"relative_and_wheel_totals":totals,"owned_before_cleanup":owned_before,"owned_after_cleanup":owned_after,"cleanup_failures":failures,"process_cpu_percent_one_logical_core":initial.zip(final_resource).map(|(a,b)|(b.cpu_ns-a.cpu_ns) as f64/duration_ns as f64*100.),"resources":points,"recovery_ms":policy.recovery_ms,"retry_ms":policy.retry_ms,"config_sha256":config.fingerprint()?,"io_workers":config.execution.io_workers,"limits":"Host-local accounting. No cross-PC C0/I0, physical or game acceptance claim; check live/recording mode."}),
        end_stream,
    ))
}

pub async fn host(
    config: &bricks_config::ResolvedConfig,
    store: &Store,
    peer: &str,
    approved: bool,
    recording: bool,
    report: Option<&Path>,
    signal: Arc<Signal>,
) -> Result<serde_json::Value, AuthError> {
    if !approved && !recording {
        return Err("explicit host approval is required before live sharing".into());
    }
    if config.role != bricks_config::Role::Host {
        return Err("host role configuration required".into());
    }
    let endpoint = probe::host_endpoint_with_transport(
        store,
        peer,
        config.transport.listen.ok_or("listen address required")?,
        transport()?,
    )?;
    println!(
        "Bricks {} listening on {}. Only the pinned peer can connect.",
        if recording {
            "recording host"
        } else {
            "LIVE INPUT HOST"
        },
        endpoint.local_addr()?
    );
    let no_input = || serde_json::json!({"report_schema_version":1,"role":"host","recording_only":recording,"input_admitted":false,"reason":"sharing stopped before admitting input","owned_after_cleanup":0,"cleanup_failures":0});
    let mut rejected_handshakes = 0u64;
    let connection = loop {
        let incoming = tokio::select! {incoming=endpoint.accept()=>incoming.ok_or("listener closed")?,_=tokio::time::sleep(Duration::from_millis(20))=>{if signal.stopped.load(Ordering::Acquire){return Ok(no_input());}continue;}};
        let handshake = timeout(Duration::from_secs(4), incoming);
        tokio::pin!(handshake);
        let result = loop {
            tokio::select! {result=&mut handshake=>break result,_=tokio::time::sleep(Duration::from_millis(20))=>if signal.stopped.load(Ordering::Acquire){return Ok(no_input());}}
        };
        if let Ok(Ok(connection)) = result {
            break connection;
        }
        rejected_handshakes = rejected_handshakes.saturating_add(1);
        if rejected_handshakes == 1 {
            eprintln!(
                "Unverified or incomplete connection rejected. Sharing remains available to the pinned peer."
            );
        }
    };
    if signal.stopped.load(Ordering::Acquire) {
        return Ok(no_input());
    }
    let _trust = TrustWatch::start(store.clone(), peer.to_owned(), signal.clone())?;
    let _policy = crate::power::ProcessPolicy::request(false)?;
    let _awake = crate::power::KeepAwake::request()?;
    let (mut result, mut terminal) = if recording {
        receive(
            &connection,
            store,
            true,
            config,
            |_| Ok(Recording::default()),
            || Ok(()),
            &signal,
        )
        .await?
    } else {
        let executable = std::env::current_exe()?;
        let watcher_report = report.map(|path| path.with_extension("watchdog.json"));
        let guard = Guard::start(&executable, watcher_report.as_deref())?;
        println!("Bricks input watchdog: {}", guard.child_id());
        let result = receive(
            &connection,
            store,
            approved,
            config,
            |_| Ok(SupervisedInjector(guard.clone())),
            || guard.pulse().map_err(AuthError::from),
            &signal,
        )
        .await;
        let owned = guard.owned();
        drop(guard);
        let mut result = result?;
        result.0["journal_owned_after_cleanup"] = serde_json::json!(owned);
        result
    };
    result["recording_only"] = serde_json::json!(recording);
    result["rejected_handshakes"] = serde_json::json!(rejected_handshakes);
    result["input_admitted"] = serde_json::json!(true);
    result["timer_resolution_override"] = serde_json::json!(_policy.timer_override);
    if let Some(stream) = terminal.as_mut() {
        let _ = timeout(Duration::from_secs(1), stream.stopped()).await;
    }
    connection.close(0u32.into(), b"input stopped");
    endpoint.close(0u32.into(), b"input stopped");
    // Allow the reliable terminal reply to reach a synthetic client before exit.
    let _ = timeout(Duration::from_millis(200), endpoint.wait_idle()).await;
    Ok(result)
}

struct Output<'a> {
    connection: &'a quinn::Connection,
    sender: Sender,
    barriers: VecDeque<(u64, Totals)>,
    totals: Totals,
    epoch: Epoch,
    segment: u64,
    motion_events: u64,
    critical_events: u64,
    retries: u64,
}
impl Output<'_> {
    fn send(&mut self, event: CaptureEvent, now_ms: u64) -> Result<(), AuthError> {
        match event {
            CaptureEvent::Key {
                key,
                down,
                captured_ns,
            } => {
                let event = self.sender.enqueue(key, down, captured_ns, now_ms)?;
                let barrier = self.totals;
                let frame = Critical {
                    transition: event,
                    barrier,
                };
                self.connection
                    .send_datagram(frame.encode()?.to_vec().into())?;
                self.barriers.push_back((event.id, barrier));
                self.segment = event.id;
                self.totals = Totals::default();
                self.critical_events += 1;
            }
            CaptureEvent::Motion { delta, captured_ns } => {
                self.totals.advance(delta)?;
                self.motion_events += 1;
                self.connection.send_datagram(
                    Motion {
                        epoch: self.epoch,
                        segment: self.segment,
                        captured_ns,
                        totals: self.totals,
                    }
                    .encode(false)?
                    .to_vec()
                    .into(),
                )?;
            }
        }
        Ok(())
    }
    fn retry(&mut self, now_ms: u64) -> Result<(), AuthError> {
        for _ in 0..8 {
            let Some(event) = self.sender.retry(now_ms)? else {
                break;
            };
            let barrier = self
                .barriers
                .iter()
                .find(|(id, _)| *id == event.id)
                .ok_or("missing retained motion barrier")?
                .1;
            self.connection.send_datagram(
                Critical {
                    transition: event,
                    barrier,
                }
                .encode()?
                .to_vec()
                .into(),
            )?;
            self.retries += 1;
        }
        Ok(())
    }
    fn motion(&self, captured_ns: u64) -> Motion {
        Motion {
            epoch: self.epoch,
            segment: self.segment,
            captured_ns,
            totals: self.totals,
        }
    }
}
pub async fn client(
    config: &bricks_config::ResolvedConfig,
    store: &Store,
    peer: &str,
    signal: Arc<Signal>,
    mut input: mpsc::Receiver<CaptureEvent>,
    origin: Instant,
    synthetic: Option<(u64, u32)>,
) -> Result<serde_json::Value, AuthError> {
    if config.role != bricks_config::Role::Client {
        return Err("client role configuration required".into());
    }
    let remote = config.transport.peer.ok_or("peer address required")?;
    let endpoint = probe::client_endpoint_with_transport(store, peer, remote, transport()?)?;
    let connection = timeout(
        Duration::from_secs(4),
        endpoint.connect(remote, "bricks-host")?,
    )
    .await??;
    let _trust = TrustWatch::start(store.clone(), peer.to_owned(), signal.clone())?;
    let (epoch, policy) = reliable::negotiate_client_protocol(&connection, PROTOCOL).await?;
    let _power = crate::power::ProcessPolicy::request(false)?;
    let _awake = crate::power::KeepAwake::request()?;
    let _timer = timing::ActiveTimerResolution::request()?;
    let _games = config
        .execution
        .games_scheduling
        .then(timing::GamesTask::request)
        .transpose()?;
    let start = Instant::now();
    let initial = resources::sample(std::process::id()).ok();
    let monitor = resources::Monitor::start(start)?;
    let mut output = Output {
        connection: &connection,
        sender: Sender::new(epoch, policy, 0)?,
        barriers: VecDeque::with_capacity(policy.send_capacity),
        totals: Totals::default(),
        epoch,
        segment: 0,
        motion_events: 0,
        critical_events: 0,
        retries: 0,
    };
    let mut next_heartbeat = start;
    let mut generated = 0;
    let mut completed = false;
    let mut acknowledged = 0;
    let (limit, rate) = synthetic.unwrap_or((0, 1000));
    let period = Duration::from_nanos(1_000_000_000 / u64::from(rate));
    let maintenance = sleep_until(start.into());
    tokio::pin!(maintenance);
    let generation = sleep_until(start.into());
    tokio::pin!(generation);
    let mut max_lateness_ns = 0u64;
    signal.status("Host verified and sharing approved. Click Start forwarding (or F8).");
    signal.ready.store(true, Ordering::Release);
    let result:Result<(),AuthError>=async {
        loop {
            if signal.stopped.load(Ordering::Acquire){break;}
            let now=start.elapsed().as_millis() as u64;
            if synthetic.is_some()&&generated==limit&&output.sender.pending_count()==0{completed=true;break;}
            let wake=output.sender.next_wake_ms().map(|ms|start+Duration::from_millis(ms)).unwrap_or(next_heartbeat).min(next_heartbeat);
            maintenance.as_mut().reset(wake.into());
            tokio::select! {
                biased;
                ack=connection.read_datagram()=>{
                    let ack=reliable::decode_ack(&ack?)?;output.sender.acknowledge(ack,start.elapsed().as_millis() as u64)?;
                    acknowledged=acknowledged.max(ack.through);while output.barriers.front().is_some_and(|(id,_)|*id<=ack.through){output.barriers.pop_front();}
                }
                event=input.recv(),if synthetic.is_none()=>{
                    let Some(event)=event else{break;};
                    if signal.stopped.load(Ordering::Acquire){break;}
                    let captured=match event {CaptureEvent::Key{captured_ns,..}|CaptureEvent::Motion{captured_ns,..}=>captured_ns};
                    if origin.elapsed().as_nanos().saturating_sub(u128::from(captured))>=u128::from(policy.recovery_ms)*1_000_000 {return Err("capture queue exceeded the recovery deadline; session stopped".into());}
                    output.send(event,start.elapsed().as_millis() as u64)?;
                }
                _=&mut generation,if synthetic.is_some()&&generated<limit=>{
                    let captured_ns=origin.elapsed().as_nanos() as u64;
                    max_lateness_ns=max_lateness_ns.max(start.elapsed().saturating_sub(period*generated as u32).as_nanos() as u64);
                    output.send(CaptureEvent::Motion{delta:[1,-1,if generated.is_multiple_of(1000){120}else{0},if generated.is_multiple_of(2000){-120}else{0}],captured_ns},now)?;
                    for (modulus,offset,key) in [(300,0,PhysicalKey{usage_page:9,usage:1}),(1000,2,PhysicalKey{usage_page:7,usage:4})] {
                        if generated%modulus==offset {output.send(CaptureEvent::Key{key,down:true,captured_ns},now)?;}
                        if generated%modulus==offset+1 {output.send(CaptureEvent::Key{key,down:false,captured_ns},now)?;}
                    }
                    generated+=1;generation.as_mut().reset((start+period*generated as u32).into());
                }
                _=&mut maintenance=>{
                    let now=start.elapsed().as_millis() as u64;output.retry(now)?;
                    if Instant::now()>=next_heartbeat {
                        output.sender.heartbeat(now)?;
                        connection.send_datagram(output.motion(origin.elapsed().as_nanos() as u64).encode(true)?.to_vec().into())?;
                        next_heartbeat=Instant::now()+Duration::from_millis(20);
                    }
                }
            }
        }
        Ok(())
    }.await;
    let succeeded = result.is_ok();
    let mut reason = match result {
        Ok(()) => {
            if completed {
                "completed".to_owned()
            } else {
                format!("stopped: {}", signal.message())
            }
        }
        Err(error) => error.to_string(),
    };
    let mut cleanup_confirmed = false;
    if completed || (succeeded && output.sender.pending_count() == 0) {
        let end = async {
            let (mut send, mut recv) = connection.open_bi().await?;
            let mut bytes = Vec::from(*b"BLSTOP01");
            bytes.extend(
                output
                    .motion(origin.elapsed().as_nanos() as u64)
                    .encode(true)?,
            );
            send.write_all(&bytes).await?;
            send.finish()?;
            let reply = recv.read_to_end(8).await?;
            if reply != b"STOPPED1" {
                return Err("invalid stop confirmation".into());
            }
            Ok::<(), AuthError>(())
        };
        match timeout(Duration::from_secs(1), end).await {
            Ok(Ok(())) => cleanup_confirmed = true,
            _ => {
                completed = false;
                reason.push_str("; remote cleanup not confirmed");
            }
        }
    }
    connection.close(0u32.into(), b"client stopped");
    let pending = output.sender.pending_count();
    output.sender.stop();
    let duration_ns = start.elapsed().as_nanos() as u64;
    let last = resources::sample(std::process::id()).ok();
    let points = monitor.finish();
    let stats = connection.stats().udp_tx;
    signal.ready.store(false, Ordering::Release);
    signal.stop(format!(
        "Session ended: {reason}. Reconnect to start again."
    ));
    connection.close(0u32.into(), b"client stopped");
    endpoint.close(0u32.into(), b"client stopped");
    Ok(
        serde_json::json!({"report_schema_version":1,"role":"client","measurement_class":if synthetic.is_some(){"synthetic_separate_process_live_wire"}else{"windows_foreground_live_client"},"reason":reason,"duration_ns":duration_ns,"generated_motion":output.motion_events,"generated_critical":output.critical_events,"critical_acknowledged":acknowledged,"unacknowledged_at_stop":pending,"remote_cleanup_confirmed":cleanup_confirmed,"completed":completed,"retries":output.retries,"synthetic_motion_scheduled":synthetic.map(|_|limit),"synthetic_motion_generated":generated,"generator_max_lateness_ns":max_lateness_ns,"process_cpu_percent_one_logical_core":initial.zip(last).map(|(a,b)|(b.cpu_ns-a.cpu_ns) as f64/duration_ns as f64*100.),"resources":points,"udp_datagrams":stats.datagrams,"udp_bytes":stats.bytes,"io_workers":config.execution.io_workers,"config_sha256":config.fingerprint()?,"limits":"Per-process accounting; no cross-clock latency or physical gaming claim. Motion is cumulative with critical barriers, not per-sample reliable replay."}),
    )
}

fn flag<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>, AuthError> {
    if let Some(i) = args.iter().position(|arg| arg == name) {
        return Ok(Some(args.get(i + 1).ok_or("missing option value")?));
    }
    Ok(None)
}

pub fn run_cli_if_requested(is_host: bool) -> Result<bool, AuthError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--internal-input-watchdog")
    {
        if !is_host {
            return Err("watchdog belongs to the host executable".into());
        }
        bricks_input_windows::watchdog::run(args.get(1).map(Path::new))?;
        return Ok(true);
    }
    let live = if is_host { "live-host" } else { "live-client" };
    let record = if is_host {
        "record-host"
    } else {
        "record-client"
    };
    if !args.first().is_some_and(|arg| arg == live || arg == record) {
        return Ok(false);
    }
    let recording = args[0] == record;
    let config_path = flag(&args, "--config")?.ok_or("--config is required")?;
    let source = std::fs::read_to_string(config_path)?;
    let config = bricks_config::resolve(&source, None)?;
    let store = if let Some(path) = flag(&args, "--identity-dir")? {
        Store::new(path)
    } else {
        Store::from_reference(&config.security.identity_ref)?
    };
    let peer = flag(&args, "--peer")?.ok_or("--peer verified fingerprint is required")?;
    let report = flag(&args, "--report")?.map(PathBuf::from);
    let signal = Arc::new(Signal::default());
    if args.iter().any(|arg| arg == "--owned-session") {
        signal.close_on_stop.store(true, Ordering::Release);
        let signal = signal.clone();
        std::thread::spawn(move || {
            let mut byte = [0];
            loop {
                match std::io::Read::read(&mut std::io::stdin(), &mut byte) {
                    Ok(0) | Err(_) => {
                        signal.stop("Owning app disconnected. Session stopped.");
                        break;
                    }
                    Ok(_) => {}
                }
            }
        });
    }
    let rt = runtime(&config)?;
    let result = if is_host {
        let approved = args.iter().any(|arg| arg == "--approve-live");
        rt.block_on(host(
            &config,
            &store,
            peer,
            approved,
            recording,
            report.as_deref(),
            signal,
        ))?
    } else {
        let (origin, (tx, rx)) = (Instant::now(), mpsc::channel(256));
        if recording {
            let events: u64 = flag(&args, "--events")?.unwrap_or("60000").parse()?;
            let rate: u32 = flag(&args, "--rate")?.unwrap_or("1000").parse()?;
            if events == 0 || events > 2_000_000 || ![1000, 4000, 8000].contains(&rate) {
                return Err("invalid synthetic load bounds".into());
            }
            rt.block_on(client(
                &config,
                &store,
                peer,
                signal,
                rx,
                origin,
                Some((events, rate)),
            ))?
        } else {
            let peer = peer.to_owned();
            let worker_signal = signal.clone();
            let worker = std::thread::spawn(move || {
                let result = rt.block_on(client(
                    &config,
                    &store,
                    &peer,
                    worker_signal.clone(),
                    rx,
                    origin,
                    None,
                ));
                if let Err(error) = &result {
                    worker_signal.stop(error.to_string());
                }
                result
            });
            let captured = bricks_input_windows::live::run(signal.clone(), origin, move |event| {
                tx.try_send(event).is_ok()
            });
            signal.stopped.store(true, Ordering::Release);
            let mut result = worker.join().map_err(|_| "client worker panicked")??;
            result["capture"] = captured?;
            result
        }
    };
    if let Some(report) = report {
        std::fs::write(report, serde_json::to_vec_pretty(&result)?)?;
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn live_wire_repairs_motion_barriers_and_reordered_clicks_without_os_input() {
        let pair = crate::recovery_lab::EphemeralPair::new().unwrap();
        let endpoint =
            probe::host_endpoint(&pair.host, &pair.client_fp, "127.0.0.1:0".parse().unwrap())
                .unwrap();
        let remote = endpoint.local_addr().unwrap();
        let client_endpoint = probe::client_endpoint(&pair.client, &pair.host_fp, remote).unwrap();
        let (client, host) = tokio::join!(
            client_endpoint.connect(remote, "bricks-host").unwrap(),
            async { endpoint.accept().await.unwrap().await }
        );
        let (client, host) = (client.unwrap(), host.unwrap());
        let mut config =
            bricks_config::resolve(include_str!("../../../docs/examples/host.toml"), None).unwrap();
        config.execution.games_scheduling = false;
        let signal = Signal::default();
        let server = receive(
            &host,
            &pair.host,
            true,
            &config,
            |_| Ok(Recording::default()),
            || Ok(()),
            &signal,
        );
        let source = async {
            let (epoch, _) = reliable::negotiate_client_protocol(&client, PROTOCOL)
                .await
                .unwrap();
            let motion = Motion {
                epoch,
                segment: 0,
                captured_ns: 0,
                totals: Totals {
                    sample: 1,
                    axes: [5, 1, 120, -120],
                },
            };
            client
                .send_datagram(motion.encode(false).unwrap().to_vec().into())
                .unwrap();
            let future = Motion {
                epoch,
                segment: 2,
                captured_ns: 0,
                totals: Totals {
                    sample: 1,
                    axes: [-3, 0, 0, 0],
                },
            };
            client
                .send_datagram(future.encode(false).unwrap().to_vec().into())
                .unwrap();
            let event = |id, down, barrier| Critical {
                transition: bricks_protocol::Transition {
                    epoch,
                    id,
                    captured_ns: id,
                    key: PhysicalKey {
                        usage_page: 9,
                        usage: 1,
                    },
                    down,
                },
                barrier,
            };
            client
                .send_datagram(
                    event(
                        2,
                        false,
                        Totals {
                            sample: 1,
                            axes: [2, 0, 0, 0],
                        },
                    )
                    .encode()
                    .unwrap()
                    .to_vec()
                    .into(),
                )
                .unwrap();
            client
                .send_datagram(
                    event(
                        1,
                        true,
                        Totals {
                            sample: 2,
                            axes: [10, 2, 120, -120],
                        },
                    )
                    .encode()
                    .unwrap()
                    .to_vec()
                    .into(),
                )
                .unwrap();
            loop {
                if reliable::decode_ack(
                    &timeout(Duration::from_secs(1), client.read_datagram())
                        .await
                        .unwrap()
                        .unwrap(),
                )
                .unwrap()
                .through
                    == 2
                {
                    break;
                }
            }
            let mut stale = event(3, true, Totals::default());
            stale.transition.epoch = Epoch([9; 16]);
            client
                .send_datagram(stale.encode().unwrap().to_vec().into())
                .unwrap();
            let (mut send, mut recv) = client.open_bi().await.unwrap();
            let mut end = Vec::from(*b"BLSTOP01");
            end.extend(future.encode(true).unwrap());
            send.write_all(&end).await.unwrap();
            send.finish().unwrap();
            assert_eq!(recv.read_to_end(8).await.unwrap(), b"STOPPED1");
        };
        let (report, _) = tokio::join!(server, source);
        let (report, _) = report.unwrap();
        assert_eq!(report["critical_applied"], 2);
        assert_eq!(
            report["relative_and_wheel_totals"],
            serde_json::json!([9, 2, 120, -120])
        );
        assert_eq!(report["owned_after_cleanup"], 0);
        assert_eq!(report["cleanup_failures"], 0);
        // A stream stop can race the deliberately stale datagram, so its exact
        // discard count is diagnostic; no old-epoch input may alter the totals.
        endpoint.close(0u32.into(), b"done");
        client_endpoint.close(0u32.into(), b"done");
    }
    #[tokio::test]
    async fn explicit_approval_is_required_before_opening_a_live_listener() {
        let config =
            bricks_config::resolve(include_str!("../../../docs/examples/host.toml"), None).unwrap();
        let store = Store::new(std::env::temp_dir().join("bricks-no-identity-needed-for-denial"));
        assert!(
            host(
                &config,
                &store,
                "",
                false,
                false,
                None,
                Arc::new(Signal::default())
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("approval")
        );
    }
}
