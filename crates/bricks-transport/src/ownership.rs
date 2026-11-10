//! Authenticated admission and explicit input ownership. No socket/UI loop here.
use crate::{
    auth::AuthError,
    identity::{ControlLease, Store, fingerprint},
};
use bricks_core::{Applied, ApplyError, Injector, Session};
use bricks_protocol::{Epoch, PhysicalKey, Transition};

#[derive(Debug)]
pub enum ControlError<E> {
    Revoked {
        cleanup_failures: Vec<(PhysicalKey, E)>,
    },
    Input(ApplyError<E>),
}
pub struct ControlledSession<I: Injector> {
    session: Session<I>,
    lease: ControlLease,
    epoch: Epoch,
}
impl<I: Injector> ControlledSession<I> {
    pub fn admit(
        connection: &rustls::ServerConnection,
        store: &Store,
        host_approved: bool,
        injector: I,
    ) -> Result<Self, AuthError> {
        if !host_approved {
            return Err("host has not approved control".into());
        }
        if connection.is_handshaking()
            || connection.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3)
            || connection.alpn_protocol() != Some(b"bricks-input/1")
        {
            return Err("completed Bricks TLS 1.3 authentication required".into());
        }
        let certificate = connection
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or("client certificate required")?;
        Self::from_verified_certificate(certificate, store, host_approved, injector)
    }
    pub(crate) fn admit_quic(
        connection: &quinn::Connection,
        store: &Store,
        host_approved: bool,
        injector: I,
    ) -> Result<Self, AuthError> {
        let (epoch, lease) = authorize_quic(connection, store, host_approved)?;
        Ok(Self {
            session: Session::new(epoch, injector, 64),
            lease,
            epoch,
        })
    }
    fn from_verified_certificate(
        certificate: &rustls::pki_types::CertificateDer<'static>,
        store: &Store,
        host_approved: bool,
        injector: I,
    ) -> Result<Self, AuthError> {
        let (epoch, lease) = authorize_certificate(certificate, store, host_approved)?;
        Ok(Self {
            session: Session::new(epoch, injector, 64),
            lease,
            epoch,
        })
    }
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }
    pub fn apply(&mut self, event: Transition) -> Result<Applied, ControlError<I::Error>> {
        if !self.lease.is_valid() {
            return Err(ControlError::Revoked {
                cleanup_failures: self.session.stop(),
            });
        }
        self.session.apply(event).map_err(ControlError::Input)
    }
    pub fn poll_authorization(&mut self) -> Result<(), Vec<(PhysicalKey, I::Error)>> {
        if self.lease.is_valid() {
            Ok(())
        } else {
            Err(self.session.stop())
        }
    }
    pub fn stop(&mut self) -> Vec<(PhysicalKey, I::Error)> {
        self.lease.invalidate();
        self.session.stop()
    }
    pub fn pressed_count(&self) -> usize {
        self.session.pressed_count()
    }
}

/// Shared authentication/admission path for both the legacy probe and recovery.
pub(crate) fn authorize_quic(
    connection: &quinn::Connection,
    store: &Store,
    host_approved: bool,
) -> Result<(Epoch, ControlLease), AuthError> {
    if connection.side() != quinn::Side::Server || connection.close_reason().is_some() {
        return Err("live server-side QUIC connection required".into());
    }
    let handshake = connection
        .handshake_data()
        .ok_or("QUIC handshake metadata missing")?
        .downcast::<quinn::crypto::rustls::HandshakeData>()
        .map_err(|_| "unexpected QUIC crypto provider")?;
    if handshake.protocol.as_deref() != Some(b"bricks-input/1") {
        return Err("wrong application protocol".into());
    }
    let chain = connection
        .peer_identity()
        .ok_or("authenticated client certificate required")?
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .map_err(|_| "unexpected QUIC peer identity")?;
    let certificate = chain.first().ok_or("empty client certificate chain")?;
    authorize_certificate(certificate, store, host_approved)
}
fn authorize_certificate(
    certificate: &rustls::pki_types::CertificateDer<'static>,
    store: &Store,
    host_approved: bool,
) -> Result<(Epoch, ControlLease), AuthError> {
    if !host_approved {
        return Err("host has not approved control".into());
    }
    let fp = fingerprint(certificate.as_ref());
    if store.trusted_certificate(&fp)?.as_ref() != certificate.as_ref() {
        return Err("peer certificate no longer trusted".into());
    }
    let lease = store.control_lease(&fp)?;
    let mut bytes = [0; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| "secure epoch generation failed")?;
    let epoch = Epoch(bytes);
    Ok((epoch, lease))
}

impl<I: Injector> Drop for ControlledSession<I> {
    fn drop(&mut self) {
        self.lease.invalidate();
        // Best effort only. Explicit stop exposes errors; process crashes still
        // require the future external watchdog/supervisor.
        let _ = self.session.stop();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::{auth, identity::test_support::TestStore};
    use std::{cell::RefCell, io::Cursor, rc::Rc, sync::Arc};
    type Events = Rc<RefCell<Vec<(PhysicalKey, bool)>>>;
    #[derive(Clone, Default)]
    struct Fixture(Events);
    impl Injector for Fixture {
        type Error = std::convert::Infallible;
        fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), Self::Error> {
            self.0.borrow_mut().push((key, down));
            Ok(())
        }
    }
    fn connections(
        host: &Store,
        client: &Store,
    ) -> (rustls::ClientConnection, rustls::ServerConnection) {
        let hc = host.certificate().unwrap();
        let cc = client.certificate().unwrap();
        let host_fp = fingerprint(hc.as_ref());
        client.trust(hc, &host_fp, false).unwrap();
        let client_config = auth::client_for_peer(client, &host_fp).unwrap();
        let server_config = auth::server_for_peer(host, &fingerprint(cc.as_ref())).unwrap();
        (
            rustls::ClientConnection::new(
                Arc::new(client_config),
                "bricks-host".try_into().unwrap(),
            )
            .unwrap(),
            rustls::ServerConnection::new(Arc::new(server_config)).unwrap(),
        )
    }
    fn handshake(c: &mut rustls::ClientConnection, s: &mut rustls::ServerConnection) {
        for _ in 0..32 {
            let mut wire = Vec::new();
            while c.wants_write() {
                c.write_tls(&mut wire).unwrap();
            }
            if !wire.is_empty() {
                s.read_tls(&mut Cursor::new(wire)).unwrap();
                s.process_new_packets().unwrap();
            }
            let mut wire = Vec::new();
            while s.wants_write() {
                s.write_tls(&mut wire).unwrap();
            }
            if !wire.is_empty() {
                c.read_tls(&mut Cursor::new(wire)).unwrap();
                c.process_new_packets().unwrap();
            }
            if !c.is_handshaking() && !s.is_handshaking() {
                return;
            }
        }
        panic!("handshake exceeded bounded progress");
    }
    #[test]
    fn protected_identities_authenticate_before_exclusive_control_and_revoke_releases_idle_keys() {
        let host = TestStore::new();
        let client = TestStore::new();
        host.store.init("bricks-host").unwrap();
        client.store.init("bricks-client").unwrap();
        let cert = client.store.certificate().unwrap();
        let fp = fingerprint(cert.as_ref());
        host.store.trust(cert.clone(), &fp, true).unwrap();
        let (mut c, mut s) = connections(&host.store, &client.store);
        assert!(ControlledSession::admit(&s, &host.store, true, Fixture::default()).is_err());
        handshake(&mut c, &mut s);
        assert!(ControlledSession::admit(&s, &host.store, false, Fixture::default()).is_err());
        let fixture = Fixture::default();
        let events = fixture.0.clone();
        let mut control = ControlledSession::admit(&s, &host.store, true, fixture).unwrap();
        assert!(ControlledSession::admit(&s, &host.store, true, Fixture::default()).is_err());
        let key = PhysicalKey {
            usage_page: 7,
            usage: 4,
        };
        let mut down = Transition {
            epoch: control.epoch(),
            id: 1,
            captured_ns: 100,
            key,
            down: true,
        };
        let mut stale = down;
        stale.epoch = Epoch([0; 16]);
        assert!(matches!(
            control.apply(stale),
            Err(ControlError::Input(ApplyError::Epoch))
        ));
        control.apply(down).unwrap();
        assert_eq!(control.pressed_count(), 1);
        host.store.revoke(&fp).unwrap();
        assert!(control.poll_authorization().unwrap_err().is_empty());
        assert_eq!(control.pressed_count(), 0);
        assert_eq!(*events.borrow(), vec![(key, true), (key, false)]);
        down.id = 2;
        assert!(matches!(
            control.apply(down),
            Err(ControlError::Revoked { .. })
        ));
        host.store.trust(cert, &fp, true).unwrap();
        assert!(matches!(
            control.apply(down),
            Err(ControlError::Revoked { .. })
        ));
    }
    #[test]
    fn authenticated_peer_without_control_permission_is_denied() {
        let host = TestStore::new();
        let client = TestStore::new();
        host.store.init("bricks-host").unwrap();
        client.store.init("bricks-client").unwrap();
        let cert = client.store.certificate().unwrap();
        let fp = fingerprint(cert.as_ref());
        host.store.trust(cert, &fp, false).unwrap();
        let (mut c, mut s) = connections(&host.store, &client.store);
        handshake(&mut c, &mut s);
        assert!(ControlledSession::admit(&s, &host.store, true, Fixture::default()).is_err());
    }
}
