//! Authenticated critical-transition recovery adapter. No UI, filesystem reads,
//! sleep-on-arrival, or OS-specific injector is required on the data path.
use crate::{
    auth::AuthError,
    identity::{ControlLease, Store},
    ownership::authorize_quic,
};
use bricks_core::{
    Injector,
    recovery::{AppliedAck, Delivery, Fault, Policy, Receiver},
};
use bricks_protocol::{Epoch, Transition};

pub const ACK_BYTES: usize = 28;
pub async fn grant_host(
    connection: &quinn::Connection,
    epoch: Epoch,
    policy: Policy,
) -> Result<(), AuthError> {
    grant_host_protocol(connection, epoch, policy, b"BRKREC01").await
}
pub(crate) async fn grant_host_protocol(
    connection: &quinn::Connection,
    epoch: Epoch,
    policy: Policy,
    protocol: &[u8; 8],
) -> Result<(), AuthError> {
    let policy = policy.validate()?;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let mut hello = [0; 8];
        recv.read_exact(&mut hello).await?;
        if &hello != protocol {
            return Err("wrong recovery protocol handshake".into());
        }
        let mut grant = [0; 32];
        grant[..4].copy_from_slice(b"BRG1");
        grant[4..20].copy_from_slice(&epoch.0);
        for (index, value) in [
            policy.send_capacity as u16,
            policy.receive_capacity as u16,
            policy.retry_ms as u16,
            policy.recovery_ms as u16,
            policy.silence_ms as u16,
        ]
        .iter()
        .enumerate()
        {
            grant[20 + index * 2..22 + index * 2].copy_from_slice(&value.to_le_bytes());
        }
        send.write_all(&grant).await?;
        send.finish()?;
        Ok::<(), AuthError>(())
    })
    .await?
}
pub async fn negotiate_client(
    connection: &quinn::Connection,
) -> Result<(Epoch, Policy), AuthError> {
    negotiate_client_protocol(connection, b"BRKREC01").await
}
pub(crate) async fn negotiate_client_protocol(
    connection: &quinn::Connection,
    protocol: &[u8; 8],
) -> Result<(Epoch, Policy), AuthError> {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let (mut send, mut recv) = connection.open_bi().await?;
        send.write_all(protocol).await?;
        send.finish()?;
        let grant = recv.read_to_end(32).await?;
        if grant.len() != 32 || &grant[..4] != b"BRG1" || grant[30..] != [0, 0] {
            return Err("invalid recovery ownership grant".into());
        }
        let word = |index: usize| {
            u16::from_le_bytes(grant[20 + index * 2..22 + index * 2].try_into().unwrap()) as u64
        };
        Ok((
            Epoch(grant[4..20].try_into()?),
            Policy {
                send_capacity: word(0) as usize,
                receive_capacity: word(1) as usize,
                retry_ms: word(2),
                recovery_ms: word(3),
                silence_ms: word(4),
            }
            .validate()?,
        ))
    })
    .await?
}
pub fn encode_heartbeat(epoch: Epoch, last_produced: u64) -> [u8; ACK_BYTES] {
    let mut bytes = encode_ack(AppliedAck {
        epoch,
        through: last_produced,
    });
    bytes[..4].copy_from_slice(b"BHB1");
    bytes
}
pub fn encode_ack(ack: AppliedAck) -> [u8; ACK_BYTES] {
    let mut bytes = [0; ACK_BYTES];
    bytes[..4].copy_from_slice(b"BAK1");
    bytes[4..20].copy_from_slice(&ack.epoch.0);
    bytes[20..].copy_from_slice(&ack.through.to_le_bytes());
    bytes
}
pub fn decode_ack(bytes: &[u8]) -> Result<AppliedAck, AuthError> {
    if bytes.len() != ACK_BYTES || &bytes[..4] != b"BAK1" {
        return Err("invalid recovery ACK envelope".into());
    }
    Ok(AppliedAck {
        epoch: Epoch(bytes[4..20].try_into()?),
        through: u64::from_le_bytes(bytes[20..].try_into()?),
    })
}

pub fn policy(config: &bricks_config::Recovery, srtt_ms: u64) -> Result<Policy, Fault> {
    Policy {
        send_capacity: config.send_history_capacity.into(),
        receive_capacity: config.receive_buffer_capacity.into(),
        retry_ms: (srtt_ms / 4).clamp(config.retry_min_ms.into(), config.retry_max_ms.into()),
        recovery_ms: srtt_ms
            .saturating_mul(2)
            .saturating_add(10)
            .clamp(config.deadline_min_ms.into(), config.deadline_max_ms.into()),
        silence_ms: config.silence_timeout_ms.into(),
    }
    .validate()
}

pub struct RecoveryHost<I: Injector> {
    receiver: Receiver<I>,
    lease: ControlLease,
    pub stale_discards: u64,
    pub cleanup_failures_observed: usize,
}
impl<I: Injector> RecoveryHost<I> {
    pub fn admit(
        connection: &quinn::Connection,
        store: &Store,
        approved: bool,
        injector: I,
        policy: Policy,
    ) -> Result<Self, AuthError> {
        Self::admit_with(connection, store, approved, policy, |_| Ok(injector))
    }
    pub fn admit_with(
        connection: &quinn::Connection,
        store: &Store,
        approved: bool,
        policy: Policy,
        make_injector: impl FnOnce(Epoch) -> Result<I, AuthError>,
    ) -> Result<Self, AuthError> {
        let (epoch, lease) = authorize_quic(connection, store, approved)?;
        let injector = make_injector(epoch)?;
        Ok(Self {
            receiver: Receiver::new(epoch, injector, 64, policy, 0)?,
            lease,
            stale_discards: 0,
            cleanup_failures_observed: 0,
        })
    }
    pub fn epoch(&self) -> Epoch {
        self.receiver.ack().epoch
    }
    pub fn ack(&self) -> AppliedAck {
        self.receiver.ack()
    }
    pub fn tick(&mut self, now_ms: u64) -> Result<(), AuthError> {
        if !self.lease.is_valid() {
            let failures = self.receiver.stop().len();
            self.cleanup_failures_observed += failures;
            return Err(format!("authorization revoked; cleanup failures: {failures}").into());
        }
        self.receiver.tick(now_ms).map_err(|e| {
            self.cleanup_failures_observed += e.cleanup_failures.len();
            format!(
                "{}; cleanup failures: {}",
                e.fault,
                e.cleanup_failures.len()
            )
            .into()
        })
    }
    pub fn receive(&mut self, bytes: &[u8], now_ms: u64) -> Result<Delivery, AuthError> {
        self.tick(now_ms)?;
        let event = Transition::decode(bytes)?;
        self.receive_event(event, now_ms)
    }
    pub fn receive_event(&mut self, event: Transition, now_ms: u64) -> Result<Delivery, AuthError> {
        self.tick(now_ms)?;
        if event.epoch != self.epoch() {
            self.stale_discards += 1;
            return Ok(Delivery {
                ack: self.ack(),
                submitted: 0,
                unchanged: 0,
                buffered: false,
                duplicate: false,
            });
        }
        self.receiver.receive(event, now_ms).map_err(|e| {
            self.cleanup_failures_observed += e.cleanup_failures.len();
            format!(
                "{}; cleanup failures: {}",
                e.fault,
                e.cleanup_failures.len()
            )
            .into()
        })
    }
    pub fn heartbeat(&mut self, bytes: &[u8], now_ms: u64) -> Result<AppliedAck, AuthError> {
        self.tick(now_ms)?;
        if bytes.len() != ACK_BYTES || &bytes[..4] != b"BHB1" {
            return Err("invalid heartbeat envelope".into());
        }
        let epoch = Epoch(bytes[4..20].try_into()?);
        let produced = u64::from_le_bytes(bytes[20..].try_into()?);
        if epoch != self.epoch() {
            self.stale_discards += 1;
            return Ok(self.ack());
        }
        self.receiver
            .keep_alive(epoch, produced, now_ms)
            .map_err(|e| {
                self.cleanup_failures_observed += e.cleanup_failures.len();
                format!(
                    "{}; cleanup failures: {}",
                    e.fault,
                    e.cleanup_failures.len()
                )
                .into()
            })
    }
    pub fn pending_count(&self) -> usize {
        self.receiver.pending_count()
    }
    pub fn next_wake_ms(&self) -> Option<u64> {
        self.receiver.next_wake_ms()
    }
    pub fn pressed_count(&self) -> usize {
        self.receiver.pressed_count()
    }
    pub fn stop(&mut self) -> usize {
        self.lease.invalidate();
        let failures = self.receiver.stop().len();
        self.cleanup_failures_observed += failures;
        failures
    }
}
impl<I: Injector> Drop for RecoveryHost<I> {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ack_has_fixed_versioned_layout_and_rejects_malformed_packets() {
        let ack = AppliedAck {
            epoch: Epoch([7; 16]),
            through: 0x1020304050607080,
        };
        let bytes = encode_ack(ack);
        assert_eq!(&bytes[..4], b"BAK1");
        assert_eq!(
            &bytes[20..],
            &[0x80, 0x70, 0x60, 0x50, 0x40, 0x30, 0x20, 0x10]
        );
        assert_eq!(decode_ack(&bytes).unwrap(), ack);
        assert!(decode_ack(&bytes[..27]).is_err());
        assert!(decode_ack(&[0; 28]).is_err());
        assert!(decode_ack(&[0; 29]).is_err());
    }
}
