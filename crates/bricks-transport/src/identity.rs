//! Development identity provisioning. Windows current-user DPAPI protects private keys.
//! Public peer certificates must be fingerprint-verified out of band before trust.
use crate::auth::{self, AuthError};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityRecord {
    schema_version: u16,
    name: String,
    certificate: Vec<u8>,
    protected_key: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerRecord {
    schema_version: u16,
    certificate: Vec<u8>,
    allow_control: bool,
}

pub fn fingerprint(certificate: &[u8]) -> String {
    format!("{:x}", Sha256::digest(certificate))
}
fn valid_fingerprint(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A lease is invalidated by in-process revoke/refresh. Revocation never revives
/// existing leases; reauthorization requires a new completed handshake.
#[derive(Clone)]
pub struct ControlLease(Arc<AtomicBool>);
impl ControlLease {
    pub fn is_valid(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub(crate) fn invalidate(&self) {
        self.0.store(false, Ordering::Release);
    }
}
type Leases = Arc<Mutex<Vec<(String, Weak<AtomicBool>)>>>;
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    leases: Leases,
}
impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            leases: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub fn from_reference(reference: &str) -> Result<Self, AuthError> {
        let name = reference
            .strip_prefix("os-store:")
            .ok_or("identity reference must start with os-store:")?;
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("invalid identity reference name".into());
        }
        let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
        Ok(Self::new(
            PathBuf::from(base)
                .join("Bricks")
                .join("identities")
                .join(name),
        ))
    }
    pub fn init(&self, name: &str) -> Result<String, AuthError> {
        if !["bricks-host", "bricks-client"].contains(&name) {
            return Err("unsupported development identity name".into());
        }
        if self.root.join("identity.json").exists() {
            return Err("identity already exists; refusing to replace it".into());
        }
        let generated = rcgen::generate_simple_self_signed(vec![name.into()])?;
        let key = Zeroizing::new(generated.signing_key.serialize_der());
        let record = IdentityRecord {
            schema_version: 1,
            name: name.into(),
            certificate: generated.cert.der().to_vec(),
            protected_key: protect(&key)?,
        };
        fs::create_dir_all(&self.root)?;
        write_new(
            &self.root.join("identity.json"),
            &serde_json::to_vec(&record)?,
        )?;
        Ok(fingerprint(&record.certificate))
    }
    pub fn credentials(
        &self,
    ) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), AuthError> {
        let record: IdentityRecord =
            serde_json::from_slice(&read_bounded(&self.root.join("identity.json"))?)?;
        if record.schema_version != 1 {
            return Err("unsupported identity schema".into());
        }
        let plain = unprotect(&record.protected_key)?;
        let key: PrivateKeyDer<'static> = PrivatePkcs8KeyDer::from(plain.to_vec()).into();
        let certificate = CertificateDer::from(record.certificate);
        // Also validate DER and matching key: corrupted/unrelated public metadata
        // must not turn DPAPI success into a valid credential.
        auth::server_config(
            vec![certificate.clone()],
            key.clone_key(),
            certificate.clone(),
        )?;
        Ok((certificate, key))
    }
    pub fn certificate(&self) -> Result<CertificateDer<'static>, AuthError> {
        Ok(self.credentials()?.0)
    }
    fn peer_path(&self, fp: &str) -> Result<PathBuf, AuthError> {
        if !valid_fingerprint(fp) {
            return Err("expected 64 lowercase SHA-256 hex digits".into());
        }
        Ok(self.root.join("peers").join(format!("{fp}.json")))
    }
    pub fn trust(
        &self,
        certificate: CertificateDer<'static>,
        expected: &str,
        allow_control: bool,
    ) -> Result<(), AuthError> {
        if fingerprint(certificate.as_ref()) != expected {
            return Err("certificate fingerprint mismatch; trust not written".into());
        }
        let path = self.peer_path(expected)?;
        // Root-store parsing rejects malformed certificates; actual handshake also
        // validates name/time/signature/usage before ownership can be granted.
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certificate.clone())?;
        let record = PeerRecord {
            schema_version: 1,
            certificate: certificate.to_vec(),
            allow_control,
        };
        fs::create_dir_all(self.root.join("peers"))?;
        write_new(&path, &serde_json::to_vec(&record)?)
    }
    fn peer(&self, fp: &str) -> Result<PeerRecord, AuthError> {
        let record: PeerRecord = serde_json::from_slice(&read_bounded(&self.peer_path(fp)?)?)?;
        if record.schema_version != 1 || fingerprint(&record.certificate) != fp {
            return Err("invalid peer trust record".into());
        }
        Ok(record)
    }
    pub fn trusted_certificate(&self, fp: &str) -> Result<CertificateDer<'static>, AuthError> {
        Ok(CertificateDer::from(self.peer(fp)?.certificate))
    }
    pub fn trusted_peers(&self, control_only: bool) -> Result<Vec<String>, AuthError> {
        let directory = self.root.join("peers");
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut peers = Vec::new();
        for entry in entries.take(65) {
            let entry = entry?;
            let name = entry.file_name();
            let Some(fp) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
                continue;
            };
            if !valid_fingerprint(fp) {
                continue;
            }
            let peer = self.peer(fp)?;
            if !control_only || peer.allow_control {
                peers.push(fp.to_owned());
            }
            if peers.len() > 64 {
                return Err("too many trusted peers".into());
            }
        }
        peers.sort();
        Ok(peers)
    }
    pub(crate) fn control_lease(&self, fp: &str) -> Result<ControlLease, AuthError> {
        let mut leases = self.leases.lock().map_err(|_| "trust lock poisoned")?;
        if !self.peer(fp)?.allow_control {
            return Err("peer is trusted but has no control permission".into());
        }
        leases.retain(|(_, lease)| lease.strong_count() > 0);
        if leases
            .iter()
            .any(|(_, lease)| lease.upgrade().is_some_and(|v| v.load(Ordering::Acquire)))
        {
            return Err("this host store already has an active controller".into());
        }
        if leases.len() >= 64 {
            return Err("control lease capacity exceeded".into());
        }
        let valid = Arc::new(AtomicBool::new(true));
        leases.push((fp.into(), Arc::downgrade(&valid)));
        Ok(ControlLease(valid))
    }
    pub fn revoke(&self, fp: &str) -> Result<(), AuthError> {
        let path = self.peer_path(fp)?;
        let leases = self.leases.lock().map_err(|_| "trust lock poisoned")?;
        for (peer, lease) in leases.iter() {
            if peer == fp
                && let Some(valid) = lease.upgrade()
            {
                valid.store(false, Ordering::Release);
            }
        }
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
    /// The future control actor must call this off the input hot path and then
    /// notify its injection actor. Detects revocation by another process/store.
    pub fn refresh_leases(&self) -> Result<(), AuthError> {
        let leases = self.leases.lock().map_err(|_| "trust lock poisoned")?;
        for (fp, lease) in leases.iter() {
            if !self.peer(fp).is_ok_and(|peer| peer.allow_control)
                && let Some(valid) = lease.upgrade()
            {
                valid.store(false, Ordering::Release);
            }
        }
        Ok(())
    }
}
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, AuthError> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("identity/certificate file exceeds 64 KiB".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), AuthError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn crypt(bytes: &[u8], decrypt: bool) -> Result<Zeroizing<Vec<u8>>, AuthError> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    let size = u32::try_from(bytes.len())?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: size,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input/output blobs point to valid buffers. DPAPI output is copied,
    // zeroed with volatile stores, and released using its documented allocator.
    unsafe {
        let result = if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if result == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if output.pbData.is_null() || output.cbData == 0 {
            LocalFree(output.pbData.cast());
            return Err("DPAPI returned an empty blob".into());
        }
        let plain = Zeroizing::new(
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec(),
        );
        for i in 0..output.cbData as usize {
            std::ptr::write_volatile(output.pbData.add(i), 0);
        }
        LocalFree(output.pbData.cast());
        Ok(plain)
    }
}
#[cfg(not(windows))]
fn crypt(_: &[u8], _: bool) -> Result<Zeroizing<Vec<u8>>, AuthError> {
    Err("identity protection requires Windows DPAPI".into())
}
fn protect(bytes: &[u8]) -> Result<Vec<u8>, AuthError> {
    Ok(crypt(bytes, false)?.to_vec())
}
fn unprotect(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, AuthError> {
    crypt(bytes, true)
}

/// Returns false when the caller should dispatch its ordinary configuration CLI.
pub fn run_cli_if_requested(name: &str) -> Result<bool, AuthError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("identity") {
        return Ok(false);
    }
    let usage = "identity <init|show|export|trust|revoke> <os-store:name> [public.der expected_sha256 [--allow-control] | fingerprint]";
    if args.len() < 3 {
        return Err(usage.into());
    }
    let store = Store::from_reference(&args[2])?;
    match args[1].as_str() {
        "init" if args.len() == 3 => println!("identity_sha256={}", store.init(name)?),
        "show" if args.len() == 3 => println!(
            "identity_sha256={}",
            fingerprint(store.certificate()?.as_ref())
        ),
        "export" if args.len() == 4 => {
            write_new(Path::new(&args[3]), store.certificate()?.as_ref())?;
            println!("Public certificate exported; private key remains DPAPI protected.");
        }
        "trust" if args.len() == 5 || (args.len() == 6 && args[5] == "--allow-control") => {
            store.trust(
                CertificateDer::from(read_bounded(Path::new(&args[3]))?),
                &args[4],
                args.len() == 6,
            )?;
            println!(
                "Peer trust saved with control_permission={}",
                args.len() == 6
            );
        }
        "revoke" if args.len() == 4 => {
            store.revoke(&args[3])?;
            println!("Peer trust revoked.");
        }
        _ => return Err(usage.into()),
    }
    Ok(true)
}

#[cfg(all(test, windows))]
pub(crate) mod test_support {
    use super::*;
    pub struct TestStore {
        pub store: Store,
    }
    impl TestStore {
        pub fn new() -> Self {
            static INDEX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "bricks-identity-test-{}-{stamp}-{}",
                std::process::id(),
                INDEX.fetch_add(1, Ordering::Relaxed)
            ));
            Self {
                store: Store::new(root),
            }
        }
    }
    impl Drop for TestStore {
        fn drop(&mut self) {
            let root = &self.store.root;
            if root.starts_with(std::env::temp_dir())
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("bricks-identity-test-")
            {
                let _ = fs::remove_dir_all(root);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::test_support::TestStore;
    use super::*;
    #[test]
    fn dpapi_identity_roundtrip_refuses_overwrite_and_detects_key_mismatch() {
        let test = TestStore::new();
        let store = &test.store;
        let fp = store.init("bricks-host").unwrap();
        let (cert, key) = store.credentials().unwrap();
        assert_eq!(fingerprint(cert.as_ref()), fp);
        let path = store.root.join("identity.json");
        let mut record: IdentityRecord =
            serde_json::from_slice(&read_bounded(&path).unwrap()).unwrap();
        assert!(record.protected_key.as_slice() != key.secret_der());
        assert!(store.init("bricks-host").is_err());
        let other = rcgen::generate_simple_self_signed(vec!["bricks-host".into()]).unwrap();
        record.certificate = other.cert.der().to_vec();
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(store.credentials().is_err());
    }
    #[test]
    fn trust_is_verified_permission_is_separate_and_revocation_never_revives_leases() {
        let test = TestStore::new();
        let store = &test.store;
        let peer = rcgen::generate_simple_self_signed(vec!["bricks-client".into()]).unwrap();
        let cert = peer.cert.der().clone();
        let fp = fingerprint(cert.as_ref());
        assert!(store.trust(cert.clone(), &"0".repeat(64), true).is_err());
        store.trust(cert.clone(), &fp, false).unwrap();
        assert!(store.control_lease(&fp).is_err());
        store.revoke(&fp).unwrap();
        store.trust(cert.clone(), &fp, true).unwrap();
        let lease = store.control_lease(&fp).unwrap();
        assert!(store.control_lease(&fp).is_err());
        store.clone().revoke(&fp).unwrap();
        assert!(!lease.is_valid());
        assert!(store.trusted_certificate(&fp).is_err());
        store.trust(cert, &fp, true).unwrap();
        assert!(!lease.is_valid());
        assert!(store.control_lease(&fp).unwrap().is_valid());
        assert!(store.revoke("../identity").is_err());
    }
    #[test]
    fn independent_store_revocation_is_detected_on_control_refresh() {
        let test = TestStore::new();
        let store = &test.store;
        let peer = rcgen::generate_simple_self_signed(vec!["bricks-client".into()]).unwrap();
        let cert = peer.cert.der().clone();
        let fp = fingerprint(cert.as_ref());
        store.trust(cert, &fp, true).unwrap();
        let lease = store.control_lease(&fp).unwrap();
        Store::new(store.root.clone()).revoke(&fp).unwrap();
        store.refresh_leases().unwrap();
        assert!(!lease.is_valid());
    }
}
