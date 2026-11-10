use crate::{
    Controller, Diagnostic, Result, Settings,
    protocol::{self, AgentState, Command, Failure, Frame, Outcome},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, oneshot};

struct Operation {
    owner: u64,
    cancel: Arc<AtomicBool>,
    cancellable: bool,
    live: bool,
}
struct Shared {
    state: AgentState,
    operations: BTreeMap<u64, Operation>,
}
struct Job {
    operation: u64,
    command: Command,
    reply: oneshot::Sender<std::result::Result<Outcome, Failure>>,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone)]
pub struct Service {
    root: std::path::PathBuf,
    shared: Arc<Mutex<Shared>>,
    events: broadcast::Sender<Frame>,
    jobs: SyncSender<Job>,
    next: Arc<AtomicU64>,
}
impl Service {
    pub fn new(mut control: Controller) -> Result<Self> {
        let marker = control.root().join("agent-running.json");
        let unclean = marker.try_exists()?;
        std::fs::write(&marker, b"{\"schema_version\":1,\"live_input\":false}")?;
        let instance = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        );
        let state=AgentState { live_session:None,instance, sequence:0, snapshot:control.snapshot(), capabilities:crate::CAPABILITIES, active_operations:Vec::new(), cleanup:if unclean {"unknown_local"}else{"not_needed"}.into(), status:if unclean {"Agent restarted after an unexpected exit. Saved state reconciled; prior cleanup is unconfirmed. Sharing is off."}else{"Agent ready. Not connected. Sharing is off."}.into() };
        let (events, _) = broadcast::channel(protocol::EVENT_CAPACITY);
        let (jobs, receiver) = sync_channel::<Job>(16);
        let service = Self {
            root: control.root().to_owned(),
            shared: Arc::new(Mutex::new(Shared {
                state,
                operations: BTreeMap::new(),
            })),
            events,
            jobs,
            next: Arc::new(AtomicU64::new(1)),
        };
        let shared = service.shared.clone();
        let events = service.events.clone();
        std::thread::Builder::new()
            .name("bricks-agent-storage".into())
            .spawn(move || {
                let mut report = None;
                while let Ok(job) = receiver.recv() {
                    let result = perform(&mut control, &mut report, job.command, job.cancel)
                        .map_err(classify);
                    let mut shared = shared.lock().unwrap();
                    shared.operations.remove(&job.operation);
                    shared.state.snapshot = control.snapshot();
                    shared.state.active_operations = shared.operations.keys().copied().collect();
                    shared.state.sequence += 1;
                    shared.state.status = match &result {
                        Ok(outcome) => outcome.message.clone(),
                        Err(error) => error.to_string(),
                    };
                    let _ = events.send(Frame::State {
                        state: Box::new(shared.state.clone()),
                    });
                    let _ = job.reply.send(result);
                }
                let _ = std::fs::remove_file(marker);
            })?;
        Ok(service)
    }
    pub fn attach(&self) -> (AgentState, broadcast::Receiver<Frame>) {
        let shared = self.shared.lock().unwrap();
        let receiver = self.events.subscribe();
        (shared.state.clone(), receiver)
    }
    pub fn snapshot(&self) -> AgentState {
        self.shared.lock().unwrap().state.clone()
    }
    pub fn execute(
        &self,
        owner: u64,
        command: Command,
    ) -> std::result::Result<
        (
            u64,
            oneshot::Receiver<std::result::Result<Outcome, Failure>>,
        ),
        Failure,
    > {
        if matches!(command, Command::StopAll) {
            self.cancel_owner(owner);
            let (tx, rx) = oneshot::channel();
            let _=tx.send(Ok(Outcome {message:"Stop requested for this connection's owned operations. Waiting for cleanup confirmation.".into(),..Default::default()}));
            return Ok((0, rx));
        }
        if matches!(command, Command::StartLive { .. }) {
            return self.start_live(owner, command);
        }
        let mut shared = self.shared.lock().unwrap();
        if shared.operations.len() >= 16 {
            return Err(Failure::new("Busy", "Agent operation queue is full."));
        }
        let operation = self.next.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancellable = matches!(command, Command::Check { .. });
        let (reply, receive) = oneshot::channel();
        self.jobs
            .try_send(Job {
                operation,
                command,
                reply,
                cancel: cancel.clone(),
            })
            .map_err(|_| Failure::new("Busy", "Agent operation queue is full or unavailable."))?;
        shared.operations.insert(
            operation,
            Operation {
                owner,
                cancel,
                cancellable,
                live: false,
            },
        );
        shared.state.active_operations = shared.operations.keys().copied().collect();
        shared.state.sequence += 1;
        shared.state.status = "Agent operation accepted; waiting for completion.".into();
        let _ = self.events.send(Frame::State {
            state: Box::new(shared.state.clone()),
        });
        Ok((operation, receive))
    }
    fn start_live(
        &self,
        owner: u64,
        command: Command,
    ) -> std::result::Result<
        (
            u64,
            oneshot::Receiver<std::result::Result<Outcome, Failure>>,
        ),
        Failure,
    > {
        let mut shared = self.shared.lock().unwrap();
        if shared.operations.len() >= 16
            || shared.operations.values().any(|operation| operation.live)
        {
            return Err(Failure::new(
                "Busy",
                "Stop the existing live session before starting another.",
            ));
        }
        let operation = self.next.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let snapshot = shared.state.snapshot.clone();
        let (reply, receive) = oneshot::channel();
        shared.operations.insert(
            operation,
            Operation {
                owner,
                cancel: cancel.clone(),
                cancellable: true,
                live: true,
            },
        );
        shared.state.live_session = Some(
            if matches!(
                command,
                Command::StartLive {
                    device_id: None,
                    ..
                }
            ) {
                "host"
            } else {
                "client"
            }
            .into(),
        );
        shared.state.active_operations = shared.operations.keys().copied().collect();
        shared.state.sequence += 1;
        shared.state.status = "Starting the dedicated live-input worker...".into();
        let _ = self.events.send(Frame::State {
            state: Box::new(shared.state.clone()),
        });
        drop(shared);
        let service = self.clone();
        let root = self.root.clone();
        let spawn = std::thread::Builder::new()
            .name("bricks-session-owner".into())
            .spawn(move || {
                let progress_service = service.clone();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::session::run(&root, snapshot, command, cancel, move |message| {
                        let mut shared = progress_service.shared.lock().unwrap();
                        shared.state.sequence += 1;
                        shared.state.status = message;
                        let _ = progress_service.events.send(Frame::State {
                            state: Box::new(shared.state.clone()),
                        });
                    })
                }))
                .unwrap_or_else(|_| {
                    Err("live-session owner failed; cleanup is unconfirmed".into())
                });
                let mut shared = service.shared.lock().unwrap();
                shared.operations.remove(&operation);
                shared.state.live_session = None;
                shared.state.active_operations = shared.operations.keys().copied().collect();
                shared.state.sequence += 1;
                let result = match result {
                    Ok((outcome, cleanup)) => {
                        shared.state.cleanup = cleanup;
                        shared.state.status = outcome.message.clone();
                        Ok(outcome)
                    }
                    Err(error) => {
                        shared.state.cleanup = "unknown_local".into();
                        let error = classify(error);
                        shared.state.status = error.to_string();
                        Err(error)
                    }
                };
                let _ = service.events.send(Frame::State {
                    state: Box::new(shared.state.clone()),
                });
                let _ = reply.send(result);
            });
        if let Err(error) = spawn {
            let mut shared = self.shared.lock().unwrap();
            shared.operations.remove(&operation);
            shared.state.live_session = None;
            shared.state.active_operations = shared.operations.keys().copied().collect();
            return Err(Failure::new("Unavailable", error));
        }
        Ok((operation, receive))
    }
    pub fn cancel(&self, owner: u64, operation: u64) -> std::result::Result<(), Failure> {
        let shared = self.shared.lock().unwrap();
        if let Some(active) = shared.operations.get(&operation) {
            if active.owner != owner {
                return Err(Failure::new(
                    "PermissionDenied",
                    "That operation belongs to another connection.",
                ));
            }
            if !active.cancellable {
                return Err(Failure::new(
                    "UnsupportedCapability",
                    "A committed storage operation cannot be cancelled.",
                ));
            }
            active.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn cancel_owner(&self, owner: u64) {
        for active in self.shared.lock().unwrap().operations.values() {
            if active.owner == owner && active.cancellable {
                active.cancel.store(true, Ordering::Release);
            }
        }
    }
}
fn classify(error: crate::Error) -> Failure {
    if let Some(failure) = error.downcast_ref::<Failure>() {
        return failure.clone();
    }
    let text = error.to_string();
    let code = if text.contains("changed") {
        "RevisionConflict"
    } else if text.contains("cancelled") {
        "Cancelled"
    } else {
        "InvalidSettings"
    };
    Failure::new(code, text)
}
fn perform(
    control: &mut Controller,
    last_report: &mut Option<serde_json::Value>,
    command: Command,
    cancel: Arc<AtomicBool>,
) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    match command {
        Command::Peers => {
            outcome.peers = Some(control.peers()?);
            outcome.message = "Verified peer identities refreshed.".into();
        }
        Command::TrustPeer {
            certificate_file,
            expected_sha256,
            incoming_client,
        } => {
            control.trust_peer(&certificate_file, &expected_sha256, incoming_client)?;
            outcome.peers = Some(control.peers()?);
            outcome.message = "Verified peer certificate imported. Sharing remains off.".into();
        }
        Command::RevokePeer {
            fingerprint,
            incoming_client,
        } => {
            control
                .identity_store(incoming_client)
                .revoke(&fingerprint)?;
            outcome.peers = Some(control.peers()?);
            outcome.message = "Peer trust revoked.".into();
        }
        Command::StartLive { .. } => unreachable!("live sessions use a separate owner worker"),
        Command::SaveDevice {
            revision,
            id,
            name,
            ip,
            port,
        } => {
            let address = crate::parse_address(&ip, &port)?;
            let id = control.save_device(revision, id, name.trim().into(), address)?;
            outcome.message = "Computer saved. Availability has not been checked.".into();
            outcome.selection = Some(id.to_string());
            outcome.close_dialog = true;
            outcome.snapshot = Some(control.snapshot());
        }
        Command::Forget { revision, id } => {
            control.forget_device(revision, id)?;
            outcome.message = "Computer forgotten. Peer trust was not changed.".into();
            outcome.selection = Some(String::new());
            outcome.close_dialog = true;
            outcome.snapshot = Some(control.snapshot());
        }
        Command::Settings {
            revision,
            name,
            port,
        } => {
            let default_port = port
                .trim()
                .parse()
                .map_err(|_| "Port must be a number from 1 to 65535.")?;
            control.update_settings(
                revision,
                Settings {
                    computer_name: name.trim().into(),
                    default_port,
                },
            )?;
            outcome.message = "Settings saved.".into();
            outcome.snapshot = Some(control.snapshot());
        }
        Command::Setup => {
            outcome.identities = Some(control.setup_identities()?);
            outcome.peers = Some(control.peers()?);
            outcome.message =
                "This PC is set up. Sharing remains off; no peer trust was granted.".into();
        }
        Command::Check { kind } => {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            let report = match kind {
                Diagnostic::RecoveryNetwork => crate::run_network_diagnostic(cancel.clone())?,
                _ => crate::run_diagnostic(kind)?,
            };
            if cancel.load(Ordering::Acquire) {
                return Err(
                    "cancelled; partial diagnostic report was not presented as complete".into(),
                );
            }
            let summary = match kind {
                Diagnostic::Recovery => format!(
                    "Recovery: {} / {} virtual-time scenarios passed. No OS input or network involved.",
                    report["scenarios_passed"], report["scenarios_total"]
                ),
                Diagnostic::Loopback => format!(
                    "Local QUIC: {} synthetic round trips; median {:.3} ms, p99 {:.3} ms. Recording only.",
                    report["samples"],
                    report["p50_ns"].as_u64().unwrap_or_default() as f64 / 1e6,
                    report["p99_ns"].as_u64().unwrap_or_default() as f64 / 1e6
                ),
                Diagnostic::RecoveryNetwork => format!(
                    "Network recovery: {}; {} / {} recorded transitions applied. Not gaming latency.",
                    report["status"], report["applied"], report["generated"]
                ),
            };
            *last_report = Some(report);
            outcome.message = summary.clone();
            outcome.report_summary = Some(summary);
        }
        Command::Export => {
            let path = crate::export_report(
                control.root(),
                last_report
                    .as_ref()
                    .ok_or("Run a check before exporting.")?,
            )?;
            outcome.message = format!("Report saved: {}", path.display());
        }
        Command::StopAll => unreachable!(),
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "bricks-service-test-{}-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    async fn finish(service: Service, path: std::path::PathBuf) {
        drop(service);
        for _ in 0..100 {
            if !path.join("agent-running.json").exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        std::fs::remove_dir_all(path).unwrap();
    }
    #[tokio::test]
    async fn live_owner_is_exclusive_and_other_connections_cannot_cancel_it() {
        let path = root();
        let service = Service::new(Controller::open(path.clone()).unwrap()).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        service.shared.lock().unwrap().operations.insert(
            42,
            Operation {
                owner: 7,
                cancel: cancel.clone(),
                cancellable: true,
                live: true,
            },
        );
        assert_eq!(
            service
                .execute(
                    8,
                    Command::StartLive {
                        device_id: None,
                        peer_fingerprint: "0".repeat(64),
                        bind_ip: "127.0.0.1".into()
                    }
                )
                .unwrap_err()
                .code,
            "Busy"
        );
        assert_eq!(service.cancel(8, 42).unwrap_err().code, "PermissionDenied");
        assert!(!cancel.load(Ordering::Acquire));
        service.cancel_owner(7);
        assert!(cancel.load(Ordering::Acquire));
        service.shared.lock().unwrap().operations.remove(&42);
        finish(service, path).await;
    }
    #[tokio::test]
    async fn event_overflow_is_explicit_and_atomic_reattach_returns_current_snapshot() {
        let path = root();
        let service = Service::new(Controller::open(path.clone()).unwrap()).unwrap();
        let (initial, mut events) = service.attach();
        assert_eq!(initial.sequence, 0);
        for revision in 0..80 {
            let (_, reply) = service
                .execute(
                    1,
                    Command::Settings {
                        revision,
                        name: "Snapshot test".into(),
                        port: "47990".into(),
                    },
                )
                .unwrap();
            reply.await.unwrap().unwrap();
        }
        assert!(matches!(
            events.recv().await,
            Err(broadcast::error::RecvError::Lagged(_))
        ));
        let (state, _) = service.attach();
        assert_eq!(state.snapshot.revision, 80);
        assert_eq!(state.sequence, 160);
        assert!(state.active_operations.is_empty());
        finish(service, path).await;
    }
    #[tokio::test]
    async fn operation_cancellation_is_owned_and_terminal() {
        let path = root();
        let service = Service::new(Controller::open(path.clone()).unwrap()).unwrap();
        let (operation, reply) = service
            .execute(
                7,
                Command::Check {
                    kind: Diagnostic::RecoveryNetwork,
                },
            )
            .unwrap();
        assert_eq!(
            service.cancel(8, operation).unwrap_err().code,
            "PermissionDenied"
        );
        service.cancel_owner(7);
        let result = reply.await.unwrap().unwrap_err();
        assert_eq!(result.code, "Cancelled");
        assert!(service.snapshot().active_operations.is_empty());
        service.cancel(7, operation).unwrap();
        finish(service, path).await;
    }
}
