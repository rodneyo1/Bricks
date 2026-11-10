//! Versioned reference codec for critical transitions. Not the frozen network protocol.
//! No authentication is provided by this codec.

use thiserror::Error;

pub mod live;

pub const VERSION: u8 = 1;
pub const TRANSITION_BYTES: usize = 48;
const MAGIC: [u8; 4] = *b"BRK0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Epoch(pub [u8; 16]);

/// Physical key usage; text/layout conversion belongs to the host OS.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PhysicalKey {
    pub usage_page: u16,
    pub usage: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transition {
    pub epoch: Epoch,
    /// Starts at one in each epoch. Zero and wrap are prohibited.
    pub id: u64,
    /// Client application receipt time relative to its local monotonic origin.
    pub captured_ns: u64,
    pub key: PhysicalKey,
    pub down: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CodecError {
    #[error("expected exactly {TRANSITION_BYTES} bytes, received {0}")]
    Length(usize),
    #[error("invalid magic or protocol version")]
    Version,
    #[error("unknown message type, flags, or nonzero reserved byte")]
    Flags,
    #[error("transition id must be nonzero")]
    Id,
    #[error("physical usage page and usage must be nonzero")]
    Key,
}

impl Transition {
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.id == 0 {
            return Err(CodecError::Id);
        }
        if self.key.usage_page == 0 || self.key.usage == 0 {
            return Err(CodecError::Key);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<[u8; TRANSITION_BYTES], CodecError> {
        self.validate()?;
        let mut out = [0; TRANSITION_BYTES];
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[5] = 1; // critical key transition
        out[6] = u8::from(self.down);
        out[8..24].copy_from_slice(&self.epoch.0);
        out[24..32].copy_from_slice(&self.id.to_le_bytes());
        out[32..40].copy_from_slice(&self.captured_ns.to_le_bytes());
        out[40..42].copy_from_slice(&self.key.usage_page.to_le_bytes());
        out[42..44].copy_from_slice(&self.key.usage.to_le_bytes());
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() != TRANSITION_BYTES {
            return Err(CodecError::Length(bytes.len()));
        }
        if bytes[..4] != MAGIC || bytes[4] != VERSION {
            return Err(CodecError::Version);
        }
        if bytes[5] != 1 || bytes[6] > 1 || bytes[7] != 0 || bytes[44..48] != [0; 4] {
            return Err(CodecError::Flags);
        }
        // Exact-length validation above proves these fixed ranges have the right length.
        let event = Self {
            epoch: Epoch(bytes[8..24].try_into().unwrap()),
            id: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            captured_ns: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            key: PhysicalKey {
                usage_page: u16::from_le_bytes(bytes[40..42].try_into().unwrap()),
                usage: u16::from_le_bytes(bytes[42..44].try_into().unwrap()),
            },
            down: bytes[6] == 1,
        };
        event.validate()?;
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn event() -> Transition {
        Transition {
            epoch: Epoch([0xa5; 16]),
            id: 1,
            captured_ns: 0x0102_0304_0506_0708,
            key: PhysicalKey {
                usage_page: 7,
                usage: 4,
            },
            down: true,
        }
    }

    #[test]
    fn golden_wire_vector() {
        let expected = [
            66, 82, 75, 48, 1, 1, 1, 0, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165,
            165, 165, 165, 165, 1, 0, 0, 0, 0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 7, 0, 4, 0, 0, 0,
            0, 0,
        ];
        assert_eq!(event().encode().unwrap(), expected);
        assert_eq!(Transition::decode(&expected).unwrap(), event());
    }

    #[test]
    fn rejects_truncation_trailing_bytes_and_invalid_fields() {
        let wire = event().encode().unwrap();
        for size in 0..TRANSITION_BYTES {
            assert_eq!(
                Transition::decode(&wire[..size]),
                Err(CodecError::Length(size))
            );
        }
        assert!(Transition::decode(&[0; TRANSITION_BYTES + 1]).is_err());
        for index in [0, 4, 5, 6, 7, 44, 47] {
            let mut bad = wire;
            bad[index] = 255;
            assert!(
                Transition::decode(&bad).is_err(),
                "accepted bad byte {index}"
            );
        }
        let mut bad = wire;
        bad[24..32].fill(0);
        assert_eq!(Transition::decode(&bad), Err(CodecError::Id));
        bad = wire;
        bad[42..44].fill(0);
        assert_eq!(Transition::decode(&bad), Err(CodecError::Key));
    }
}
