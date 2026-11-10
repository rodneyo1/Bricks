//! TLS 1.3 mutual authentication, reusable by the future QUIC endpoint.
//! Trust anchors must be provisioned through an independently verified pairing path.
//! This module does not issue control ownership or persist private keys.
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig, server::WebPkiClientVerifier};
use std::sync::Arc;

pub type AuthError = Box<dyn std::error::Error + Send + Sync>;
const ALPN: &[u8] = b"bricks-input/1";

pub fn client_for_peer(
    store: &crate::identity::Store,
    peer_fingerprint: &str,
) -> Result<ClientConfig, AuthError> {
    let (certificate, key) = store.credentials()?;
    client_config(
        vec![certificate],
        key,
        store.trusted_certificate(peer_fingerprint)?,
    )
}
pub fn server_for_peer(
    store: &crate::identity::Store,
    peer_fingerprint: &str,
) -> Result<ServerConfig, AuthError> {
    let (certificate, key) = store.credentials()?;
    server_config(
        vec![certificate],
        key,
        store.trusted_certificate(peer_fingerprint)?,
    )
}

/// Uses only explicitly supplied peer trust, never the public Web PKI root set.
pub fn client_config(
    own_chain: Vec<CertificateDer<'static>>,
    own_key: PrivateKeyDer<'static>,
    trusted_host: CertificateDer<'static>,
) -> Result<ClientConfig, AuthError> {
    let mut roots = RootCertStore::empty();
    roots.add(trusted_host)?;
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_root_certificates(roots)
            .with_client_auth_cert(own_chain, own_key)?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    config.enable_early_data = false;
    Ok(config)
}

pub fn server_config(
    own_chain: Vec<CertificateDer<'static>>,
    own_key: PrivateKeyDer<'static>,
    trusted_client: CertificateDer<'static>,
) -> Result<ServerConfig, AuthError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = RootCertStore::empty();
    roots.add(trusted_client)?;
    let verifier =
        WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone()).build()?;
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(own_chain, own_key)?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    config.max_early_data_size = 0;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{ClientConnection, ServerConnection, pki_types::PrivatePkcs8KeyDer};
    use std::io::{Cursor, Read, Write};

    struct Identity {
        cert: CertificateDer<'static>,
        key: Vec<u8>,
    }
    fn identity(name: &str) -> Identity {
        let generated = rcgen::generate_simple_self_signed(vec![name.into()]).unwrap();
        Identity {
            cert: generated.cert.der().clone(),
            key: generated.signing_key.serialize_der(),
        }
    }
    fn key(id: &Identity) -> PrivateKeyDer<'static> {
        PrivatePkcs8KeyDer::from(id.key.clone()).into()
    }
    fn connections(
        host: &Identity,
        client: &Identity,
        trusted_host: &Identity,
        trusted_client: &Identity,
        name: &str,
    ) -> (ClientConnection, ServerConnection) {
        let cc = client_config(
            vec![client.cert.clone()],
            key(client),
            trusted_host.cert.clone(),
        )
        .unwrap();
        let sc = server_config(
            vec![host.cert.clone()],
            key(host),
            trusted_client.cert.clone(),
        )
        .unwrap();
        (
            ClientConnection::new(Arc::new(cc), name.to_owned().try_into().unwrap()).unwrap(),
            ServerConnection::new(Arc::new(sc)).unwrap(),
        )
    }
    fn handshake(
        client: &mut ClientConnection,
        server: &mut ServerConnection,
    ) -> Result<(), AuthError> {
        for _ in 0..32 {
            let mut wire = Vec::new();
            while client.wants_write() {
                client.write_tls(&mut wire)?;
            }
            if !wire.is_empty() {
                server.read_tls(&mut Cursor::new(wire))?;
                server.process_new_packets()?;
            }
            let mut wire = Vec::new();
            while server.wants_write() {
                server.write_tls(&mut wire)?;
            }
            if !wire.is_empty() {
                client.read_tls(&mut Cursor::new(wire))?;
                client.process_new_packets()?;
            }
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok(());
            }
        }
        Err("TLS handshake made no bounded progress".into())
    }
    #[test]
    fn verified_peers_complete_tls13_and_exchange_protected_data() {
        let host = identity("bricks-host");
        let client = identity("bricks-client");
        let (mut c, mut s) = connections(&host, &client, &host, &client, "bricks-host");
        handshake(&mut c, &mut s).unwrap();
        assert_eq!(c.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
        assert_eq!(c.alpn_protocol(), Some(ALPN));
        assert_eq!(s.peer_certificates().unwrap()[0], client.cert);
        c.writer().write_all(b"authenticated fixture data").unwrap();
        let mut wire = Vec::new();
        c.write_tls(&mut wire).unwrap();
        s.read_tls(&mut Cursor::new(wire)).unwrap();
        s.process_new_packets().unwrap();
        let mut received = [0; 26];
        s.reader().read_exact(&mut received).unwrap();
        assert_eq!(&received, b"authenticated fixture data");
    }
    #[test]
    fn missing_client_certificate_is_rejected_and_early_data_is_disabled() {
        let host = identity("bricks-host");
        let client = identity("bricks-client");
        let sc = server_config(vec![host.cert.clone()], key(&host), client.cert.clone()).unwrap();
        assert_eq!(sc.max_early_data_size, 0);
        let cc = client_config(vec![client.cert.clone()], key(&client), host.cert.clone()).unwrap();
        assert!(!cc.enable_early_data);
        let mut roots = RootCertStore::empty();
        roots.add(host.cert.clone()).unwrap();
        let mut anonymous =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth();
        anonymous.alpn_protocols = vec![ALPN.to_vec()];
        let mut c =
            ClientConnection::new(Arc::new(anonymous), "bricks-host".try_into().unwrap()).unwrap();
        let mut s = ServerConnection::new(Arc::new(sc)).unwrap();
        assert!(handshake(&mut c, &mut s).is_err());
    }

    #[test]
    fn rejects_wrong_host_wrong_client_and_wrong_hostname() {
        let host = identity("bricks-host");
        let client = identity("bricks-client");
        let stranger = identity("stranger");
        for (th, tc, name) in [
            (&stranger, &client, "bricks-host"),
            (&host, &stranger, "bricks-host"),
            (&host, &client, "wrong-host"),
        ] {
            let (mut c, mut s) = connections(&host, &client, th, tc, name);
            assert!(handshake(&mut c, &mut s).is_err());
        }
    }
}
