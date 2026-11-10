//! Versioned, strictly typed configuration shared by both development binaries.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, net::SocketAddr};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("{0}")]
    Validation(String),
    #[error("could not serialize resolved configuration: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Client,
    Host,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TransportBackend {
    #[default]
    Quic,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transport {
    pub backend: TransportBackend,
    pub listen: Option<SocketAddr>,
    pub peer: Option<SocketAddr>,
    pub max_datagram_bytes: u16,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            backend: TransportBackend::Quic,
            listen: None,
            peer: None,
            max_datagram_bytes: 1200,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Security {
    pub identity_ref: String,
    pub require_paired_peer: bool,
}

impl Default for Security {
    fn default() -> Self {
        Self {
            identity_ref: String::new(),
            require_paired_peer: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    #[default]
    WindowsRawInput,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionBackend {
    #[default]
    WindowsSendInput,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Capture {
    pub backend: CaptureBackend,
    pub escape_chord: String,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            backend: CaptureBackend::WindowsRawInput,
            escape_chord: "Ctrl+Alt+Shift+F12".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Injection {
    pub backend: InjectionBackend,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub capture: Option<Capture>,
    pub injection: Option<Injection>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Forwarding {
    pub motion_aggregation_us: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Recovery {
    pub snapshot_interval_ms: u16,
    pub silence_timeout_ms: u16,
    pub send_history_capacity: u16,
    pub receive_buffer_capacity: u16,
    pub retry_min_ms: u16,
    pub retry_max_ms: u16,
    pub deadline_min_ms: u16,
    pub deadline_max_ms: u16,
}

impl Default for Recovery {
    fn default() -> Self {
        Self {
            snapshot_interval_ms: 20,
            silence_timeout_ms: 250,
            send_history_capacity: 256,
            receive_buffer_capacity: 64,
            retry_min_ms: 2,
            retry_max_ms: 20,
            deadline_min_ms: 30,
            deadline_max_ms: 150,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Diagnostics {
    pub event_trace: bool,
}

/// Keep packet I/O independent of the root input actor. Zero is retained for
/// explicitly labelled single-thread benchmark comparisons.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Execution {
    pub io_workers: u8,
    pub games_scheduling: bool,
}
impl Default for Execution {
    fn default() -> Self {
        Self {
            io_workers: 2,
            games_scheduling: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ForwardingOverride {
    motion_aggregation_us: Option<u16>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Profile {
    forwarding: ForwardingOverride,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u16,
    role: Role,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    transport: Transport,
    #[serde(default)]
    security: Security,
    #[serde(default)]
    input: Input,
    #[serde(default)]
    forwarding: Forwarding,
    #[serde(default)]
    recovery: Recovery,
    #[serde(default)]
    diagnostics: Diagnostics,
    #[serde(default)]
    execution: Execution,
    #[serde(default)]
    profiles: BTreeMap<String, Profile>,
}

/// Kept immutable by callers for the lifetime of a future live session.
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedConfig {
    pub schema_version: u16,
    pub role: Role,
    pub selected_profile: Option<String>,
    pub transport: Transport,
    pub security: Security,
    pub input: Input,
    pub forwarding: Forwarding,
    pub recovery: Recovery,
    pub diagnostics: Diagnostics,
    pub execution: Execution,
}

impl ResolvedConfig {
    pub fn json(&self) -> Result<String, ConfigError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Hash the unredacted, deterministically serialized effective settings, not the
    /// source file's whitespace or unused profile definitions. No secret material is allowed.
    pub fn fingerprint(&self) -> Result<String, ConfigError> {
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(self)?)))
    }
}

pub fn resolve(
    source: &str,
    profile_override: Option<&str>,
) -> Result<ResolvedConfig, ConfigError> {
    let mut doc: Document = toml::from_str(source)?;
    let invalid = |message: &str| ConfigError::Validation(message.into());
    if doc.schema_version != 1 {
        return Err(invalid("schema_version: only version 1 is supported"));
    }
    if doc.execution.io_workers > 2 {
        return Err(invalid("execution.io_workers: expected 0..=2"));
    }
    let selected = profile_override.map(str::to_owned).or(doc.profile);
    if let Some(name) = &selected {
        // `gaming` is a compiled built-in profile. A file may override its fields.
        let profile = doc.profiles.get(name);
        if profile.is_none() && name != "gaming" {
            return Err(invalid(&format!("profile: unknown profile {name:?}")));
        }
        if let Some(value) = profile.and_then(|p| p.forwarding.motion_aggregation_us) {
            doc.forwarding.motion_aggregation_us = value;
        }
    }
    if !(256..=1200).contains(&doc.transport.max_datagram_bytes) {
        return Err(invalid("transport.max_datagram_bytes: expected 256..=1200"));
    }
    if !doc.security.require_paired_peer {
        return Err(invalid(
            "security.require_paired_peer: authentication cannot be disabled",
        ));
    }
    if !doc.security.identity_ref.starts_with("os-store:") || doc.security.identity_ref.len() <= 9 {
        return Err(invalid(
            "security.identity_ref: expected a nonempty os-store: reference, never a secret",
        ));
    }
    if doc.forwarding.motion_aggregation_us > 1000 {
        return Err(invalid(
            "forwarding.motion_aggregation_us: expected 0..=1000",
        ));
    }
    if !(5..=100).contains(&doc.recovery.snapshot_interval_ms) {
        return Err(invalid("recovery.snapshot_interval_ms: expected 5..=100"));
    }
    if !(100..=1000).contains(&doc.recovery.silence_timeout_ms)
        || u32::from(doc.recovery.silence_timeout_ms)
            < 3 * u32::from(doc.recovery.snapshot_interval_ms)
    {
        return Err(invalid(
            "recovery.silence_timeout_ms: expected 100..=1000 and at least three snapshot intervals",
        ));
    }
    let recovery = &doc.recovery;
    if !(1..=4096).contains(&recovery.send_history_capacity)
        || !(1..=1024).contains(&recovery.receive_buffer_capacity)
    {
        return Err(invalid(
            "recovery: send history must be 1..=4096 and receive buffer 1..=1024",
        ));
    }
    if recovery.retry_min_ms == 0
        || recovery.retry_min_ms > recovery.retry_max_ms
        || recovery.retry_max_ms >= recovery.deadline_min_ms
        || recovery.deadline_min_ms > recovery.deadline_max_ms
        || recovery.deadline_max_ms > recovery.silence_timeout_ms
    {
        return Err(invalid(
            "recovery: require 0 < retry_min <= retry_max < deadline_min <= deadline_max <= silence_timeout (milliseconds)",
        ));
    }
    for (field, endpoint) in [
        ("listen", doc.transport.listen),
        ("peer", doc.transport.peer),
    ] {
        if let Some(address) = endpoint
            && (address.port() == 0 || address.ip().is_multicast() || address.ip().is_unspecified())
        {
            return Err(invalid(&format!(
                "transport.{field}: use a concrete unicast address and nonzero port"
            )));
        }
    }
    match doc.role {
        Role::Client => {
            if doc.transport.peer.is_none() || doc.transport.listen.is_some() {
                return Err(invalid(
                    "transport: client requires peer and must not set listen",
                ));
            }
            if doc.input.injection.is_some() {
                return Err(invalid("input.injection: not valid for the client role"));
            }
            let capture = doc.input.capture.get_or_insert_with(Capture::default);
            // Only this well-defined chord is supported in the initial schema.
            if capture.escape_chord != "Ctrl+Alt+Shift+F12" {
                return Err(invalid(
                    "input.capture.escape_chord: initial implementation supports Ctrl+Alt+Shift+F12 only",
                ));
            }
        }
        Role::Host => {
            if doc.transport.listen.is_none() || doc.transport.peer.is_some() {
                return Err(invalid(
                    "transport: host requires listen and must not set peer",
                ));
            }
            if doc.input.capture.is_some() {
                return Err(invalid("input.capture: not valid for the host role"));
            }
            doc.input.injection.get_or_insert_with(Injection::default);
        }
    }
    Ok(ResolvedConfig {
        schema_version: doc.schema_version,
        role: doc.role,
        selected_profile: selected,
        transport: doc.transport,
        security: doc.security,
        input: doc.input,
        forwarding: doc.forwarding,
        recovery: doc.recovery,
        diagnostics: doc.diagnostics,
        execution: doc.execution,
    })
}

/// Development CLI shared by the two binaries. It never opens a listener or
/// captures/injects input. Authentication and backend startup land in M1/M2.
pub fn run_config_cli(role: Role) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        println!("Bricks development foundation. Usage: config <path.toml> [--profile <name>]");
        println!(
            "Configuration inspection only. Identity and recording-only QUIC probes use separate commands."
        );
        return Ok(());
    };
    if command == "--help" || command == "-h" {
        println!("Usage: config <path.toml> [--profile <name>]");
        return Ok(());
    }
    if command != "config" {
        return Err(format!("unknown command {command:?}; expected config").into());
    }
    let path = args.next().ok_or("config requires a TOML path")?;
    let profile = match args.next().as_deref() {
        None => None,
        Some("--profile") => Some(args.next().ok_or("--profile requires a name")?),
        Some(other) => return Err(format!("unknown argument {other:?}").into()),
    };
    if args.next().is_some() {
        return Err("unexpected trailing arguments".into());
    }
    let config = resolve(&std::fs::read_to_string(path)?, profile.as_deref())?;
    if config.role != role {
        return Err("configuration role does not match this binary".into());
    }
    println!("{}", config.json()?);
    println!("config_sha256={}", config.fingerprint()?);
    println!(
        "status=validated_only; identity and recording-only QUIC probes use separate commands"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const CLIENT: &str = include_str!("../../../docs/examples/client.toml");
    const HOST: &str = include_str!("../../../docs/examples/host.toml");

    #[test]
    fn actual_examples_resolve_with_stable_fingerprints() {
        let client = resolve(CLIENT, None).unwrap();
        let host = resolve(HOST, None).unwrap();
        assert_eq!(client.role, Role::Client);
        assert_eq!(host.role, Role::Host);
        assert_eq!(client.fingerprint().unwrap().len(), 64);
        assert_eq!(
            client.fingerprint().unwrap(),
            resolve(&format!("{CLIENT}\n# comment"), None)
                .unwrap()
                .fingerprint()
                .unwrap()
        );
    }

    #[test]
    fn profile_override_wins_and_unknown_profile_fails() {
        let source = format!("{CLIENT}\n[profiles.test.forwarding]\nmotion_aggregation_us = 500\n");
        assert_eq!(
            resolve(&source, Some("test"))
                .unwrap()
                .forwarding
                .motion_aggregation_us,
            500
        );
        assert_eq!(
            resolve(&source, None)
                .unwrap()
                .forwarding
                .motion_aggregation_us,
            0
        );
        assert!(resolve(CLIENT, Some("missing")).is_err());
    }

    #[test]
    fn rejects_unknown_fields_backends_and_future_schema() {
        for source in [
            CLIENT.replace("schema_version = 1", "schema_version = 2"),
            CLIENT.replace("max_datagram_bytes", "max_datagram_byte"),
            CLIENT.replace("backend = \"quic\"", "backend = \"invented\""),
            format!("unexpected = true\n{CLIENT}"),
        ] {
            assert!(resolve(&source, None).is_err());
        }
    }

    #[test]
    fn rejects_disabling_auth_invalid_endpoints_and_incompatible_roles() {
        for source in [
            CLIENT.replace("require_paired_peer = true", "require_paired_peer = false"),
            CLIENT.replace("127.0.0.1:47990", "0.0.0.0:47990"),
            CLIENT.replace("127.0.0.1:47990", "127.0.0.1:0"),
            CLIENT.replace("role = \"client\"", "role = \"host\""),
            HOST.replace("silence_timeout_ms = 250", "silence_timeout_ms = 10"),
            CLIENT.replace("motion_aggregation_us = 0", "motion_aggregation_us = 8000"),
        ] {
            assert!(resolve(&source, None).is_err());
        }
    }

    #[test]
    fn recovery_limits_reject_unbounded_queues_and_inconsistent_timers() {
        for (before, after) in [
            ("send_history_capacity = 256", "send_history_capacity = 0"),
            (
                "send_history_capacity = 256",
                "send_history_capacity = 4097",
            ),
            (
                "receive_buffer_capacity = 64",
                "receive_buffer_capacity = 0",
            ),
            (
                "receive_buffer_capacity = 64",
                "receive_buffer_capacity = 1025",
            ),
            ("retry_min_ms = 2", "retry_min_ms = 0"),
            ("retry_min_ms = 2", "retry_min_ms = 21"),
            ("retry_max_ms = 20", "retry_max_ms = 30"),
            ("deadline_min_ms = 30", "deadline_min_ms = 151"),
            ("deadline_max_ms = 150", "deadline_max_ms = 251"),
        ] {
            assert!(
                resolve(&HOST.replace(before, after), None).is_err(),
                "accepted {after}"
            );
        }
        // Optional additions preserve old schema-v1 files; defaults hash exactly
        // like the same explicitly written limits, rather than the file text.
        let legacy = HOST
            .lines()
            .filter(|line| {
                ![
                    "send_history_capacity",
                    "receive_buffer_capacity",
                    "retry_min_ms",
                    "retry_max_ms",
                    "deadline_min_ms",
                    "deadline_max_ms",
                ]
                .iter()
                .any(|field| line.starts_with(field))
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            resolve(HOST, None).unwrap().fingerprint().unwrap(),
            resolve(&legacy, None).unwrap().fingerprint().unwrap()
        );
    }
}
