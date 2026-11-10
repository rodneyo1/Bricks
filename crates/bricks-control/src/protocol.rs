//! Versioned local control protocol; separate from the remote input protocol.
use crate::{Capabilities, Diagnostic, Error, Result, Snapshot};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub const VERSION: u16 = 2;
pub const MAX_FRAME: usize = 65536;
pub const EVENT_CAPACITY: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", deny_unknown_fields)]
pub enum Command {
    Peers,
    TrustPeer {
        certificate_file: String,
        expected_sha256: String,
        incoming_client: bool,
    },
    RevokePeer {
        fingerprint: String,
        incoming_client: bool,
    },
    StartLive {
        device_id: Option<u64>,
        peer_fingerprint: String,
        bind_ip: String,
    },
    SaveDevice {
        revision: u64,
        id: Option<u64>,
        name: String,
        ip: String,
        port: String,
    },
    Forget {
        revision: u64,
        id: u64,
    },
    Settings {
        revision: u64,
        name: String,
        port: String,
    },
    Setup,
    Check {
        kind: Diagnostic,
    },
    Export,
    StopAll,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub peers: Option<TrustedPeers>,
    pub message: String,
    pub snapshot: Option<Snapshot>,
    pub selection: Option<String>,
    pub close_dialog: bool,
    pub identities: Option<String>,
    pub report_summary: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedPeers {
    pub hosts: Vec<String>,
    pub clients: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub code: String,
    pub message: String,
}
impl Failure {
    pub fn new(code: &str, message: impl ToString) -> Self {
        Self {
            code: code.into(),
            message: message.to_string(),
        }
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Failure {}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentState {
    pub live_session: Option<String>,
    pub instance: String,
    pub sequence: u64,
    pub snapshot: Snapshot,
    pub capabilities: Capabilities,
    pub active_operations: Vec<u64>,
    pub cleanup: String,
    pub status: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Request {
    Hello { version: u16 },
    Execute { id: u64, command: Command },
    Snapshot { id: u64 },
    Cancel { operation: u64 },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Frame {
    Welcome {
        version: u16,
        state: Box<AgentState>,
    },
    Accepted {
        id: u64,
        operation: u64,
    },
    Completed {
        id: u64,
        state: Box<AgentState>,
        result: std::result::Result<Outcome, Failure>,
    },
    State {
        state: Box<AgentState>,
    },
    Rejected {
        id: u64,
        error: Failure,
    },
    Cancelled {
        operation: u64,
    },
}
pub async fn read_frame<T: DeserializeOwned>(reader: &mut (impl AsyncRead + Unpin)) -> Result<T> {
    let mut header = [0; 4];
    reader.read_exact(&mut header[..1]).await?;
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        reader.read_exact(&mut header[1..]),
    )
    .await??;
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err("invalid local control frame length".into());
    }
    let mut bytes = vec![0; length];
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        reader.read_exact(&mut bytes),
    )
    .await??;
    Ok(serde_json::from_slice(&bytes)?)
}
pub async fn write_frame<T: Serialize>(
    writer: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("local control frame exceeds 64 KiB".into());
    }
    writer.write_u32_le(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}
pub fn unavailable(message: impl ToString) -> Error {
    Box::new(Failure::new("AgentUnavailable", message))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_bad_versions_shapes_sizes_and_accepts_fragmented_frames() {
        assert!(
            serde_json::from_str::<Request>(
                r#"{"type":"Execute","id":1,"command":{"command":"SomethingElse"}}"#
            )
            .is_err()
        );
        let (mut w, mut r) = tokio::io::duplex(32);
        let writer = tokio::spawn(async move {
            let data = serde_json::to_vec(&Request::Hello { version: VERSION }).unwrap();
            w.write_u32_le(data.len() as u32).await.unwrap();
            for b in data {
                w.write_all(&[b]).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        assert!(matches!(
            read_frame::<Request>(&mut r).await.unwrap(),
            Request::Hello { version: VERSION }
        ));
        writer.await.unwrap();
        let (mut w, mut r) = tokio::io::duplex(8);
        w.write_u32_le(MAX_FRAME as u32 + 1).await.unwrap();
        assert!(read_frame::<Request>(&mut r).await.is_err());
        let (mut w, mut r) = tokio::io::duplex(8);
        w.write_u32_le(0).await.unwrap();
        assert!(read_frame::<Request>(&mut r).await.is_err());
    }
}
