//! Live QUIC diagnostic with synthetic key transitions and a recording injector.
//! It never captures typing or calls SendInput. Not the finished input transport.
use crate::{
    auth::{self, AuthError},
    identity::Store,
    ownership::ControlledSession,
};
use bricks_core::{Applied, Injector};
use bricks_protocol::{Epoch, PhysicalKey, Transition};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::time::{MissedTickBehavior, interval, timeout};

const HELLO: &[u8; 8] = b"BRKPRB01";
const DONE: &[u8; 4] = b"DONE";
const MAX_EVENTS: u64 = 10_000;
const OP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Default)]
struct Recorder(Arc<AtomicU64>);
impl Injector for Recorder {
    type Error = std::convert::Infallible;
    fn submit(&mut self, _: PhysicalKey, _: bool) -> Result<(), Self::Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct HostReport {
    pub received_datagrams: u64,
    pub applied_transitions: u64,
    pub duplicate_transitions: u64,
    pub unchanged_transitions: u64,
    pub recording_injector_submissions: u64,
    pub owned_keys_before_cleanup: usize,
    pub owned_keys_after_cleanup: usize,
    pub cleanup_failures: usize,
}

pub(crate) fn transport() -> Result<quinn::TransportConfig, AuthError> {
    let mut config = quinn::TransportConfig::default();
    config.max_idle_timeout(Some(OP_TIMEOUT.try_into()?));
    config.max_concurrent_bidi_streams(1u32.into());
    config.max_concurrent_uni_streams(0u32.into());
    config.datagram_receive_buffer_size(Some(16 * 1024));
    config.datagram_send_buffer_size(16 * 1024);
    config.stream_receive_window(8192u32.into());
    config.receive_window(16384u32.into());
    Ok(config)
}
pub fn host_endpoint(
    store: &Store,
    peer: &str,
    listen: SocketAddr,
) -> Result<quinn::Endpoint, AuthError> {
    host_endpoint_with_transport(store, peer, listen, Arc::new(transport()?))
}
pub(crate) fn host_endpoint_with_transport(
    store: &Store,
    peer: &str,
    listen: SocketAddr,
    transport: Arc<quinn::TransportConfig>,
) -> Result<quinn::Endpoint, AuthError> {
    let crypto =
        quinn::crypto::rustls::QuicServerConfig::try_from(auth::server_for_peer(store, peer)?)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(transport);
    config.max_incoming(4);
    config.incoming_buffer_size(16 * 1024);
    config.incoming_buffer_size_total(64 * 1024);
    config.migration(false);
    Ok(quinn::Endpoint::server(config, listen)?)
}
pub(crate) fn client_endpoint(
    store: &Store,
    peer: &str,
    remote: SocketAddr,
) -> Result<quinn::Endpoint, AuthError> {
    client_endpoint_with_transport(store, peer, remote, Arc::new(transport()?))
}
pub(crate) fn client_endpoint_with_transport(
    store: &Store,
    peer: &str,
    remote: SocketAddr,
    transport: Arc<quinn::TransportConfig>,
) -> Result<quinn::Endpoint, AuthError> {
    let crypto =
        quinn::crypto::rustls::QuicClientConfig::try_from(auth::client_for_peer(store, peer)?)?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(transport);
    let bind = if remote.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let mut endpoint = quinn::Endpoint::client(bind.parse()?)?;
    endpoint.set_default_client_config(config);
    Ok(endpoint)
}

async fn receive_events(
    connection: &quinn::Connection,
    store: &Store,
    control: &mut ControlledSession<Recorder>,
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    max_payload: usize,
    silence: Duration,
) -> Result<HostReport, AuthError> {
    let mut report = HostReport {
        received_datagrams: 0,
        applied_transitions: 0,
        duplicate_transitions: 0,
        unchanged_transitions: 0,
        recording_injector_submissions: 0,
        owned_keys_before_cleanup: 0,
        owned_keys_after_cleanup: 0,
        cleanup_failures: 0,
    };
    let mut last_input = Instant::now();
    let session_start = Instant::now();
    let mut tick = interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let stop_request = async {
        let mut stop = [0; 4];
        recv.read_exact(&mut stop).await?;
        Ok::<_, quinn::ReadExactError>(stop)
    };
    tokio::pin!(stop_request);
    loop {
        tokio::select! {
            biased;
            _=connection.closed()=>return Err("probe disconnected; owned recording state will be released".into()),
            _=tick.tick()=>{
                if session_start.elapsed()>Duration::from_secs(60) {return Err("probe 60-second session budget expired".into());}
                // Filesystem work goes to a blocking worker. This probe still uses
                // one async actor; no dedicated hot-path performance is claimed.
                let refresh=store.clone(); timeout(silence,tokio::task::spawn_blocking(move||refresh.refresh_leases())).await???;
                control.poll_authorization().map_err(|_|"probe authorization revoked")?;
                if last_input.elapsed()>silence {return Err("probe input silence watchdog expired".into());}
            },
            result=&mut stop_request=>{
                let stop=result?; if &stop!=DONE {return Err("invalid stop control message".into());}
                report.owned_keys_before_cleanup=control.pressed_count();
                return Ok(report);
            },
            packet=connection.read_datagram()=>{
                let packet=packet?;
                if packet.len()>max_payload {return Err("datagram exceeds configured payload cap".into());}
                report.received_datagrams+=1;
                if report.received_datagrams>MAX_EVENTS {return Err("probe event budget exhausted".into());}
                let event=Transition::decode(&packet)?;
                let disposition=match control.apply(event).map_err(|e|format!("probe event rejected: {e:?}"))? {
                    Applied::Submitted=>{report.applied_transitions+=1;0},
                    Applied::Duplicate=>{report.duplicate_transitions+=1;1},
                    Applied::StateUnchanged=>{report.unchanged_transitions+=1;2},
                };
                last_input=Instant::now();
                let mut ack=[0;9];ack[..8].copy_from_slice(&event.id.to_le_bytes());ack[8]=disposition;
                timeout(silence.min(OP_TIMEOUT),send.write_all(&ack)).await??;
            }
        }
    }
}

/// One approved session. Errors close the connection and drop the guard, releasing
/// recorded held keys. Nothing here is an OS input injector.
pub async fn serve_connection(
    connection: quinn::Connection,
    store: &Store,
    approved: bool,
    max_payload: usize,
    silence: Duration,
) -> Result<HostReport, AuthError> {
    serve_recorded(
        connection,
        store,
        approved,
        max_payload,
        silence,
        Recorder::default(),
    )
    .await
}
async fn serve_recorded(
    connection: quinn::Connection,
    store: &Store,
    approved: bool,
    max_payload: usize,
    silence: Duration,
    recorder: Recorder,
) -> Result<HostReport, AuthError> {
    let result = async {
        let (mut send, mut recv) = timeout(OP_TIMEOUT, connection.accept_bi()).await??;
        let mut hello = [0; 8];
        timeout(OP_TIMEOUT, recv.read_exact(&mut hello)).await??;
        if &hello != HELLO {
            return Err("invalid probe handshake".into());
        }
        let mut control =
            ControlledSession::admit_quic(&connection, store, approved, recorder.clone())?;
        if connection
            .max_datagram_size()
            .is_none_or(|n| n < bricks_protocol::TRANSITION_BYTES)
        {
            return Err("peer lacks required datagram capability".into());
        }
        timeout(OP_TIMEOUT, send.write_all(&control.epoch().0)).await??;
        let mut report = receive_events(
            &connection,
            store,
            &mut control,
            &mut send,
            &mut recv,
            max_payload,
            silence,
        )
        .await?;
        report.cleanup_failures = control.stop().len();
        report.owned_keys_after_cleanup = control.pressed_count();
        report.recording_injector_submissions = recorder.0.load(Ordering::Relaxed);
        if report.cleanup_failures != 0 || report.owned_keys_after_cleanup != 0 {
            return Err("probe cleanup failed".into());
        }
        let summary = serde_json::to_vec(&report)?;
        timeout(OP_TIMEOUT, send.write_all(&summary)).await??;
        send.finish()?;
        let _ = timeout(OP_TIMEOUT, send.stopped()).await;
        Ok(report)
    }
    .await;
    connection.close(
        if result.is_ok() {
            0u32.into()
        } else {
            1u32.into()
        },
        b"probe finished",
    );
    result
}

pub async fn host_once(
    endpoint: quinn::Endpoint,
    store: &Store,
    approved: bool,
    max_payload: usize,
    silence: Duration,
) -> Result<HostReport, AuthError> {
    let result = async {
        let incoming = timeout(Duration::from_secs(30), endpoint.accept())
            .await?
            .ok_or("endpoint closed")?;
        let connection = timeout(OP_TIMEOUT, incoming).await??;
        serve_connection(connection, store, approved, max_payload, silence).await
    }
    .await;
    endpoint.close(0u32.into(), b"probe host stopped");
    let _ = timeout(Duration::from_secs(1), endpoint.wait_idle()).await;
    result
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientReport {
    pub report_schema_version: u16,
    pub measurement_class: String,
    pub metric: String,
    pub samples: usize,
    pub mean_ns: f64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub host: HostReport,
    pub limitations: Vec<String>,
}
pub async fn client_run(
    store: &Store,
    peer: &str,
    remote: SocketAddr,
    pairs: u32,
) -> Result<ClientReport, AuthError> {
    if pairs == 0 || u64::from(pairs) * 2 > MAX_EVENTS {
        return Err("pairs must be 1..=5000".into());
    }
    let endpoint = client_endpoint(store, peer, remote)?;
    let result=async {
        let connection=timeout(OP_TIMEOUT,endpoint.connect(remote,"bricks-host")?).await??;
        let (mut send,mut recv)=timeout(OP_TIMEOUT,connection.open_bi()).await??;
        send.write_all(HELLO).await?;
        let mut epoch=[0;16];timeout(OP_TIMEOUT,recv.read_exact(&mut epoch)).await??;
        let origin=Instant::now(); let mut samples=Vec::with_capacity(pairs as usize*2);
        for id in 1..=u64::from(pairs)*2 {
            let event=Transition {epoch:Epoch(epoch),id,captured_ns:origin.elapsed().as_nanos() as u64,
                key:PhysicalKey {usage_page:7,usage:4},down:id%2==1};
            let bytes=event.encode()?;
            let start=Instant::now();connection.send_datagram(bytes.to_vec().into())?;
            let mut ack=[0;9];timeout(OP_TIMEOUT,recv.read_exact(&mut ack)).await??;
            if u64::from_le_bytes(ack[..8].try_into().unwrap())!=id || ack[8]!=0 {return Err("unexpected application acknowledgment".into());}
            samples.push(start.elapsed().as_nanos() as u64);
        }
        send.write_all(DONE).await?;send.finish()?;
        let summary=timeout(OP_TIMEOUT,recv.read_to_end(4096)).await??;
        let host:HostReport=serde_json::from_slice(&summary)?;
        if host.applied_transitions!=u64::from(pairs)*2 || host.received_datagrams!=u64::from(pairs)*2
            || host.duplicate_transitions!=0 || host.unchanged_transitions!=0 || host.recording_injector_submissions!=u64::from(pairs)*2
            || host.owned_keys_before_cleanup!=0 || host.owned_keys_after_cleanup!=0 || host.cleanup_failures!=0 {
            return Err("host probe counters failed validation".into());
        }
        connection.close(0u32.into(),b"probe received final summary");
        samples.sort_unstable();let n=samples.len();let percentile=|percent:usize|samples[(n*percent).div_ceil(100).saturating_sub(1)];
        Ok(ClientReport {report_schema_version:1,measurement_class:if remote.ip().is_loopback(){"loopback_quic_synthetic"}else{"two_endpoint_quic_synthetic"}.into(),
            metric:"synthetic_datagram_to_application_ack_roundtrip".into(),samples:n,mean_ns:samples.iter().map(|&n|n as f64).sum::<f64>()/n as f64,
            p50_ns:percentile(50),p95_ns:percentile(95),p99_ns:percentile(99),host,
            limitations:vec!["Synthetic transitions, recording injector; no OS input capture or injection".into(),
                "Stop-and-wait probe; reliable control-stream ACKs; no application datagram retries".into(),
                "Uncalibrated single-client roundtrip; not capture-to-submit, one-way, physical, or a transport bake-off".into()]})
    }.await;
    endpoint.close(0u32.into(), b"probe client stopped");
    let _ = timeout(Duration::from_secs(1), endpoint.wait_idle()).await;
    result
}

pub fn run_cli_if_requested(host_role: bool) -> Result<bool, AuthError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("probe-selftest") {
        if args.len() > 2 {
            return Err("probe-selftest [pairs:1..5000]".into());
        }
        let pairs = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(100);
        let runtime = tokio::runtime::Runtime::new()?;
        let report = runtime.block_on(loopback_selftest(pairs))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(true);
    }
    let command = if host_role {
        "probe-host"
    } else {
        "probe-client"
    };
    if args.first().map(String::as_str) != Some(command) {
        return Ok(false);
    }
    if args.len() != 4 || (host_role && args[3] != "--approve-control") {
        return Err(format!(
            "{command} <config.toml> <peer_sha256> {}",
            if host_role {
                "--approve-control"
            } else {
                "<pairs:1..5000>"
            }
        )
        .into());
    }
    let config = bricks_config::resolve(&std::fs::read_to_string(&args[1])?, None)?;
    let role = if host_role {
        bricks_config::Role::Host
    } else {
        bricks_config::Role::Client
    };
    if config.role != role {
        return Err("configuration role mismatch".into());
    }
    let store = Store::from_reference(&config.security.identity_ref)?;
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        if host_role {
            let endpoint=host_endpoint(&store,&args[2],config.transport.listen.ok_or("listen missing")?)?;
            eprintln!("Recording-only QUIC probe listening on {}; no OS input will be injected",endpoint.local_addr()?);
            let report=host_once(endpoint,&store,true,usize::from(config.transport.max_datagram_bytes),Duration::from_millis(u64::from(config.recovery.silence_timeout_ms))).await?;
            println!("{}",serde_json::to_string_pretty(&report)?);
        }else {
            let report=client_run(&store,&args[2],config.transport.peer.ok_or("peer missing")?,args[3].parse()?).await?;
            println!("{}",serde_json::to_string_pretty(&serde_json::json!({"probe":report,"config_sha256":config.fingerprint()?,"resolved_config":config}))?);
        }
        Ok::<(),AuthError>(())
    })?;
    Ok(true)
}

/// Explicit local diagnostic: fresh ephemeral identities trusted only inside this
/// process, protected keys in an owned temporary directory, recording injector.
pub async fn loopback_selftest(pairs: u32) -> Result<ClientReport, AuthError> {
    if pairs == 0 || pairs > 5000 {
        return Err("pairs must be 1..=5000".into());
    }
    struct TempRoot(std::path::PathBuf);
    impl Drop for TempRoot {
        fn drop(&mut self) {
            if let (Ok(root), Ok(base)) =
                (self.0.canonicalize(), std::env::temp_dir().canonicalize())
                && root.parent() == Some(base.as_path())
                && root
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("bricks-quic-selftest-"))
            {
                let _ = std::fs::remove_dir_all(root);
            }
        }
    }
    let mut random = [0; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut random)
        .map_err(|_| "random generation failed")?;
    let suffix = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let root = TempRoot(std::env::temp_dir().join(format!("bricks-quic-selftest-{suffix}")));
    std::fs::create_dir(&root.0)?;
    let host = Store::new(root.0.join("host"));
    let client = Store::new(root.0.join("client"));
    let host_fp = host.init("bricks-host")?;
    let client_fp = client.init("bricks-client")?;
    host.trust(client.certificate()?, &client_fp, true)?;
    client.trust(host.certificate()?, &host_fp, false)?;
    let endpoint = host_endpoint(&host, &client_fp, "127.0.0.1:0".parse()?)?;
    let remote = endpoint.local_addr()?;
    let hs = host.clone();
    let server =
        tokio::spawn(
            async move { host_once(endpoint, &hs, true, 1200, Duration::from_secs(2)).await },
        );
    let result = client_run(&client, &host_fp, remote, pairs).await;
    if result.is_err() {
        server.abort();
        let _ = server.await;
        return result;
    }
    server.await??;
    result
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::identity::test_support::TestStore;
    fn setup(allow_control: bool) -> (TestStore, TestStore, String, String) {
        let host = TestStore::new();
        let client = TestStore::new();
        let hf = host.store.init("bricks-host").unwrap();
        let cf = client.store.init("bricks-client").unwrap();
        host.store
            .trust(client.store.certificate().unwrap(), &cf, allow_control)
            .unwrap();
        client
            .store
            .trust(host.store.certificate().unwrap(), &hf, false)
            .unwrap();
        (host, client, hf, cf)
    }
    #[tokio::test]
    async fn protected_identities_complete_real_quic_datagram_roundtrips() {
        let (host, client, hf, cf) = setup(true);
        let endpoint = host_endpoint(&host.store, &cf, "127.0.0.1:0".parse().unwrap()).unwrap();
        let remote = endpoint.local_addr().unwrap();
        let hs = host.store.clone();
        let server = tokio::spawn(async move {
            host_once(endpoint, &hs, true, 1200, Duration::from_secs(2)).await
        });
        let result = client_run(&client.store, &hf, remote, 100).await.unwrap();
        assert_eq!(result.samples, 200);
        assert_eq!(result.host.applied_transitions, 200);
        assert_eq!(result.host.owned_keys_after_cleanup, 0);
        assert_eq!(
            server
                .await
                .unwrap()
                .unwrap()
                .recording_injector_submissions,
            200
        );
    }
    #[tokio::test]
    async fn real_quic_does_not_grant_control_without_permission_or_approval() {
        for (permission, approval) in [(false, true), (true, false)] {
            let (host, client, hf, cf) = setup(permission);
            let endpoint = host_endpoint(&host.store, &cf, "127.0.0.1:0".parse().unwrap()).unwrap();
            let remote = endpoint.local_addr().unwrap();
            let hs = host.store.clone();
            let server = tokio::spawn(async move {
                host_once(endpoint, &hs, approval, 1200, Duration::from_secs(2)).await
            });
            assert!(client_run(&client.store, &hf, remote, 1).await.is_err());
            assert!(server.await.unwrap().is_err());
        }
    }
    #[tokio::test]
    async fn real_quic_rejects_a_client_not_in_the_hosts_tls_trust() {
        let (host, client, hf, _) = setup(true);
        let stranger = TestStore::new();
        let sf = stranger.store.init("bricks-client").unwrap();
        host.store
            .trust(stranger.store.certificate().unwrap(), &sf, true)
            .unwrap();
        let endpoint = host_endpoint(&host.store, &sf, "127.0.0.1:0".parse().unwrap()).unwrap();
        let remote = endpoint.local_addr().unwrap();
        let hs = host.store.clone();
        let server = tokio::spawn(async move {
            host_once(endpoint, &hs, true, 1200, Duration::from_secs(2)).await
        });
        assert!(client_run(&client.store, &hf, remote, 1).await.is_err());
        assert!(server.await.unwrap().is_err());
    }
    #[tokio::test]
    async fn held_recording_state_is_released_on_network_faults_and_idle_revocation() {
        for fault in ["disconnect", "malformed", "old_epoch", "revoke", "silence"] {
            let (host, client, hf, cf) = setup(true);
            let endpoint = host_endpoint(&host.store, &cf, "127.0.0.1:0".parse().unwrap()).unwrap();
            let remote = endpoint.local_addr().unwrap();
            let ce = client_endpoint(&client.store, &hf, remote).unwrap();
            let connecting = ce.connect(remote, "bricks-host").unwrap();
            let server_endpoint = endpoint.clone();
            let accept = async move { server_endpoint.accept().await.unwrap().await.unwrap() };
            let (connection, sc) = tokio::join!(connecting, accept);
            let connection = connection.unwrap();
            let recorder = Recorder::default();
            let tracked = recorder.clone();
            let hs = host.store.clone();
            let server = tokio::spawn(async move {
                serve_recorded(sc, &hs, true, 1200, Duration::from_millis(500), tracked).await
            });
            let (mut send, mut recv) = connection.open_bi().await.unwrap();
            send.write_all(HELLO).await.unwrap();
            let mut epoch = [0; 16];
            timeout(OP_TIMEOUT, recv.read_exact(&mut epoch))
                .await
                .unwrap()
                .unwrap();
            let event = Transition {
                epoch: Epoch(epoch),
                id: 1,
                captured_ns: 0,
                key: PhysicalKey {
                    usage_page: 7,
                    usage: 4,
                },
                down: true,
            };
            connection
                .send_datagram(event.encode().unwrap().to_vec().into())
                .unwrap();
            let mut ack = [0; 9];
            timeout(OP_TIMEOUT, recv.read_exact(&mut ack))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(recorder.0.load(Ordering::Relaxed), 1);
            match fault {
                "disconnect" => connection.close(0u32.into(), b"test disconnect"),
                "malformed" => connection.send_datagram(vec![0].into()).unwrap(),
                "old_epoch" => {
                    let mut stale = event;
                    stale.epoch = Epoch([0; 16]);
                    stale.id = 2;
                    connection
                        .send_datagram(stale.encode().unwrap().to_vec().into())
                        .unwrap();
                }
                "revoke" => host.store.revoke(&cf).unwrap(),
                "silence" => {}
                _ => unreachable!(),
            }
            assert!(
                timeout(Duration::from_secs(4), server)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err(),
                "{fault}"
            );
            assert_eq!(
                recorder.0.load(Ordering::Relaxed),
                2,
                "missing release after {fault}"
            );
            ce.close(0u32.into(), b"test cleanup");
            endpoint.close(0u32.into(), b"test cleanup");
        }
    }
}
