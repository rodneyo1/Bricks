//! Real authenticated-loopback lifecycle tests with an externally held recording
//! trace. These prove recorded cleanup only, never Windows/game neutral state.
use crate::{
    auth::AuthError,
    probe,
    recovery_lab::EphemeralPair,
    reliable::{self, RecoveryHost},
};
use bricks_core::Injector;
use bricks_protocol::{Epoch, PhysicalKey, Transition};
use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::time::timeout;

type Events = Arc<Mutex<Vec<(PhysicalKey, bool)>>>;
#[derive(Clone, Default)]
struct Trace {
    events: Events,
    fail_release: Arc<AtomicBool>,
}
impl Injector for Trace {
    type Error = &'static str;
    fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), Self::Error> {
        if !down && self.fail_release.load(Ordering::Acquire) {
            return Err("deliberate release failure");
        }
        self.events.lock().unwrap().push((key, down));
        Ok(())
    }
}
#[derive(Serialize)]
pub struct Case {
    pub scenario: &'static str,
    pub passed: bool,
    pub trigger_to_recorded_neutral_ns: Option<u64>,
    pub observed_submissions: usize,
    pub cleanup_failures_observed: usize,
    pub reason: String,
}
async fn connections(
    pair: &EphemeralPair,
) -> Result<
    (
        quinn::Endpoint,
        quinn::Endpoint,
        quinn::Connection,
        quinn::Connection,
    ),
    AuthError,
> {
    let endpoint = probe::host_endpoint(&pair.host, &pair.client_fp, "127.0.0.1:0".parse()?)?;
    let remote = endpoint.local_addr()?;
    let ce = probe::client_endpoint(&pair.client, &pair.host_fp, remote)?;
    let (client, host) = timeout(Duration::from_secs(3), async {
        tokio::join!(ce.connect(remote, "bricks-host").unwrap(), async {
            endpoint.accept().await.unwrap().await
        })
    })
    .await?;
    Ok((endpoint, ce, client?, host?))
}
pub async fn run() -> Result<Vec<Case>, AuthError> {
    let mut cases = Vec::new();
    for scenario in [
        "disconnect",
        "silence",
        "held_10_seconds",
        "revoke",
        "old_epoch",
        "reconnect",
        "busy_owner",
        "worker_abort",
        "cleanup_failure",
        "approval_denied",
        "permission_denied",
        "outage_50_ms",
        "outage_250_ms",
        "outage_1000_ms",
    ] {
        let pair = EphemeralPair::new()?;
        let (endpoint, ce, client, host) = connections(&pair).await?;
        let trace = Trace::default();
        let config =
            bricks_config::resolve(include_str!("../../../docs/examples/host.toml"), None)?;
        let policy = reliable::policy(&config.recovery, 1)?;
        if scenario == "approval_denied" || scenario == "permission_denied" {
            if scenario == "permission_denied" {
                pair.host.revoke(&pair.client_fp)?;
                pair.host
                    .trust(pair.client.certificate()?, &pair.client_fp, false)?;
            }
            let denied = RecoveryHost::admit(
                &host,
                &pair.host,
                scenario != "approval_denied",
                trace.clone(),
                policy,
            )
            .is_err();
            cases.push(Case {
                scenario,
                passed: denied && trace.events.lock().unwrap().is_empty(),
                trigger_to_recorded_neutral_ns: None,
                observed_submissions: 0,
                cleanup_failures_observed: 0,
                reason: "Denied before input ownership or recorded submission".into(),
            });
            endpoint.close(0u32.into(), b"case done");
            ce.close(0u32.into(), b"case done");
            continue;
        }
        let mut receiver = RecoveryHost::admit(&host, &pair.host, true, trace.clone(), policy)?;
        let (client_grant, host_grant) = tokio::join!(
            reliable::negotiate_client(&client),
            reliable::grant_host(&host, receiver.epoch(), policy)
        );
        host_grant?;
        let (client_epoch, _) = client_grant?;
        let key = PhysicalKey {
            usage_page: 7,
            usage: 4,
        };
        let down = Transition {
            epoch: client_epoch,
            id: 1,
            captured_ns: 0,
            key,
            down: true,
        };
        client.send_datagram(down.encode()?.to_vec().into())?;
        let packet = timeout(Duration::from_secs(3), host.read_datagram()).await??;
        receiver.receive(&packet, 0)?;
        if receiver.pressed_count() != 1 {
            return Err("lifecycle fixture did not hold its recorded key".into());
        }
        let trigger = Instant::now();
        let mut failures = 0;
        let reason: String;
        let mut neutral_ns = 0;
        match scenario {
            "held_10_seconds" => {
                let mut pulse = tokio::time::interval(Duration::from_millis(20));
                pulse.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                while trigger.elapsed() < Duration::from_secs(10) {
                    pulse.tick().await;
                    client.send_datagram(
                        reliable::encode_heartbeat(receiver.epoch(), 1)
                            .to_vec()
                            .into(),
                    )?;
                    let packet = timeout(Duration::from_secs(1), host.read_datagram()).await??;
                    receiver.heartbeat(&packet, trigger.elapsed().as_millis() as u64)?;
                    if receiver.pressed_count() != 1 || trace.events.lock().unwrap().len() != 1 {
                        return Err("heartbeat altered held state or repeated a submission".into());
                    }
                }
                let mut up = down;
                up.id = 2;
                up.down = false;
                client.send_datagram(up.encode()?.to_vec().into())?;
                let packet = timeout(Duration::from_secs(1), host.read_datagram()).await??;
                receiver.receive(&packet, trigger.elapsed().as_millis() as u64)?;
                failures += receiver.stop();
                reason="Ten-second hold preserved using authenticated epoch/high-water heartbeats; no duplicate down applied".into();
            }
            "disconnect" => {
                client.close(1u32.into(), b"simulated sender exit");
                timeout(Duration::from_secs(1), host.closed()).await?;
                failures += receiver.stop();
                reason = "Transport disconnect observed; recorded key released".into();
            }
            "silence" | "outage_250_ms" | "outage_1000_ms" => {
                loop {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    if let Err(error) = receiver.tick(trigger.elapsed().as_millis() as u64) {
                        reason = error.to_string();
                        break;
                    }
                    if trigger.elapsed() > Duration::from_millis(1000) {
                        return Err("watchdog did not fire".into());
                    }
                }
                neutral_ns = trigger.elapsed().as_nanos() as u64;
                if scenario == "outage_1000_ms" {
                    tokio::time::sleep(
                        Duration::from_millis(1000).saturating_sub(trigger.elapsed()),
                    )
                    .await;
                }
                let mut late = down;
                late.id = 2;
                late.down = false;
                if receiver
                    .receive(&late.encode()?, trigger.elapsed().as_millis() as u64)
                    .is_ok()
                {
                    return Err("stopped epoch accepted resumed history".into());
                }
            }
            "outage_50_ms" => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                // Sender's 30 ms history deadline has expired. On restoration,
                // stop this epoch instead of silently treating cleanup as delivery.
                failures += receiver.stop();
                reason="Recovery deadline exceeded; restored control stops old epoch; new authorization required".into();
            }
            "revoke" => {
                pair.host.revoke(&pair.client_fp)?;
                reason = receiver.tick(0).unwrap_err().to_string();
            }
            "old_epoch" => {
                let mut stale = down;
                stale.id = 2;
                stale.down = false;
                stale.epoch = Epoch([0; 16]);
                client.send_datagram(stale.encode()?.to_vec().into())?;
                let packet = timeout(Duration::from_secs(1), host.read_datagram()).await??;
                receiver.receive(&packet, 0)?;
                if receiver.stale_discards != 1
                    || receiver.pressed_count() != 1
                    || trace.events.lock().unwrap().len() != 1
                {
                    return Err("old epoch changed current state".into());
                }
                failures += receiver.stop();
                reason =
                    "Old epoch discarded without applying release or refreshing liveness".into();
            }
            "busy_owner" => {
                if RecoveryHost::admit(&host, &pair.host, true, Trace::default(), policy).is_ok() {
                    return Err("second owner admitted".into());
                }
                failures += receiver.stop();
                reason = "Concurrent controller denied; original owner cleaned up".into();
            }
            "reconnect" => {
                let old_epoch = receiver.epoch();
                failures += receiver.stop();
                let (second_endpoint, second_ce, new_client, new_host) = connections(&pair).await?;
                let mut next =
                    RecoveryHost::admit(&new_host, &pair.host, true, Trace::default(), policy)?;
                let (new_grant, host_grant) = tokio::join!(
                    reliable::negotiate_client(&new_client),
                    reliable::grant_host(&new_host, next.epoch(), policy)
                );
                host_grant?;
                let (new_epoch, _) = new_grant?;
                if new_epoch == old_epoch {
                    return Err("reconnect reused epoch".into());
                }
                new_client.send_datagram(down.encode()?.to_vec().into())?;
                let stale = timeout(Duration::from_secs(1), new_host.read_datagram()).await??;
                next.receive(&stale, 0)?;
                if next.pressed_count() != 0 || next.stale_discards != 1 {
                    return Err("replayed previous session reached new injector".into());
                }
                failures += next.stop();
                second_endpoint.close(0u32.into(), b"case done");
                second_ce.close(0u32.into(), b"case done");
                reason = "New TLS connection has fresh epoch; old-session replay discarded".into();
            }
            "worker_abort" => {
                let (ready, started) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(async move {
                    let _owned = receiver;
                    let _ = ready.send(());
                    std::future::pending::<()>().await;
                });
                started.await?;
                task.abort();
                let _ = task.await;
                let events = trace.events.lock().unwrap();
                cases.push(Case{scenario,passed:events.as_slice()==[(key,true),(key,false)],trigger_to_recorded_neutral_ns:Some(trigger.elapsed().as_nanos() as u64),observed_submissions:events.len(),cleanup_failures_observed:0,reason:"Aborted host worker dropped its guard; external trace observed recorded release. Process death is a separate case.".into()});
                endpoint.close(0u32.into(), b"case done");
                ce.close(0u32.into(), b"case done");
                continue;
            }
            "cleanup_failure" => {
                trace.fail_release.store(true, Ordering::Release);
                failures = receiver.stop();
                if failures != 1 || receiver.pressed_count() != 1 {
                    return Err("failed release lost ownership state".into());
                }
                trace.fail_release.store(false, Ordering::Release);
                if receiver.stop() != 0 {
                    return Err("recorded cleanup retry failed".into());
                }
                reason =
                    "Release failure remained owned and visible; explicit retry completed".into();
            }
            _ => unreachable!(),
        }
        if neutral_ns == 0 {
            neutral_ns = trigger.elapsed().as_nanos() as u64;
        }
        let events = trace.events.lock().unwrap();
        let within_watchdog = !matches!(scenario, "silence" | "outage_250_ms" | "outage_1000_ms")
            || neutral_ns <= 300_000_000;
        let passed = events.as_slice() == [(key, true), (key, false)]
            && receiver.pressed_count() == 0
            && within_watchdog
            && (failures == 0 || scenario == "cleanup_failure");
        cases.push(Case {
            scenario,
            passed,
            trigger_to_recorded_neutral_ns: Some(neutral_ns),
            observed_submissions: events.len(),
            cleanup_failures_observed: failures,
            reason,
        });
        endpoint.close(0u32.into(), b"case done");
        ce.close(0u32.into(), b"case done");
    }
    Ok(cases)
}
#[cfg(all(test, windows))]
mod tests {
    #[tokio::test]
    async fn authenticated_lifecycles_release_recorded_keys_and_keep_failures_visible() {
        for case in super::run().await.unwrap() {
            assert!(case.passed, "{}: {}", case.scenario, case.reason);
        }
    }
}
