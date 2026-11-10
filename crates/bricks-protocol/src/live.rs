//! V1 live-input envelopes. The live grant is distinct from the recording grant.
//! Critical barriers carry final motion totals so a lost motion sample cannot
//! move input across a click/key boundary. No Rust memory layout is sent.
use crate::{Epoch, Transition};
use thiserror::Error;

pub const MOTION_BYTES: usize = 80;
pub const CRITICAL_BYTES: usize = 88;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Totals {
    pub sample: u64,
    /// Relative X, Y, vertical wheel, horizontal wheel; native wheel units.
    pub axes: [i64; 4],
}
impl Totals {
    pub fn advance(&mut self, delta: [i32; 4]) -> Result<(), LiveError> {
        let sample = self.sample.checked_add(1).ok_or(LiveError::Counter)?;
        let mut axes = self.axes;
        for (total, increment) in axes.iter_mut().zip(delta) {
            *total = total
                .checked_add(i64::from(increment))
                .ok_or(LiveError::Counter)?;
        }
        self.axes = axes;
        self.sample = sample;
        Ok(())
    }
    fn valid(self) -> bool {
        self.sample != 0 || self.axes == [0; 4]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Motion {
    pub epoch: Epoch,
    /// Number of critical transitions preceding this motion segment.
    pub segment: u64,
    pub captured_ns: u64,
    pub totals: Totals,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Critical {
    pub transition: Transition,
    pub barrier: Totals,
}
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum LiveError {
    #[error("invalid live-input envelope, reserved bytes or totals")]
    Packet,
    #[error("live-input counter or cumulative displacement overflow")]
    Counter,
}
fn write_totals(out: &mut [u8], totals: Totals) {
    out[..8].copy_from_slice(&totals.sample.to_le_bytes());
    for (bytes, value) in out[8..].as_chunks_mut::<8>().0.iter_mut().zip(totals.axes) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }
}
fn read_totals(bytes: &[u8]) -> Result<Totals, LiveError> {
    let totals = Totals {
        sample: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
        axes: std::array::from_fn(|i| {
            i64::from_le_bytes(bytes[8 + i * 8..16 + i * 8].try_into().unwrap())
        }),
    };
    totals.valid().then_some(totals).ok_or(LiveError::Packet)
}
impl Critical {
    pub fn encode(self) -> Result<[u8; CRITICAL_BYTES], LiveError> {
        if !self.barrier.valid() {
            return Err(LiveError::Packet);
        }
        let mut bytes = [0; CRITICAL_BYTES];
        bytes[..48].copy_from_slice(&self.transition.encode().map_err(|_| LiveError::Packet)?);
        write_totals(&mut bytes[48..], self.barrier);
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, LiveError> {
        if bytes.len() != CRITICAL_BYTES {
            return Err(LiveError::Packet);
        }
        Ok(Self {
            transition: Transition::decode(&bytes[..48]).map_err(|_| LiveError::Packet)?,
            barrier: read_totals(&bytes[48..])?,
        })
    }
}
impl Motion {
    pub fn encode(self, heartbeat: bool) -> Result<[u8; MOTION_BYTES], LiveError> {
        if !self.totals.valid() {
            return Err(LiveError::Packet);
        }
        let mut bytes = [0; MOTION_BYTES];
        bytes[..4].copy_from_slice(if heartbeat { b"BLH1" } else { b"BLM1" });
        bytes[4..20].copy_from_slice(&self.epoch.0);
        bytes[20..28].copy_from_slice(&self.segment.to_le_bytes());
        bytes[28..36].copy_from_slice(&self.captured_ns.to_le_bytes());
        write_totals(&mut bytes[36..76], self.totals);
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<(Self, bool), LiveError> {
        if bytes.len() != MOTION_BYTES
            || (&bytes[..4] != b"BLH1" && &bytes[..4] != b"BLM1")
            || bytes[76..] != [0; 4]
        {
            return Err(LiveError::Packet);
        }
        Ok((
            Self {
                epoch: Epoch(bytes[4..20].try_into().unwrap()),
                segment: u64::from_le_bytes(bytes[20..28].try_into().unwrap()),
                captured_ns: u64::from_le_bytes(bytes[28..36].try_into().unwrap()),
                totals: read_totals(&bytes[36..76])?,
            },
            &bytes[..4] == b"BLH1",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PhysicalKey;
    #[test]
    fn fixed_live_layouts_reject_truncation_reserved_bits_and_impossible_zero_sample() {
        let motion = Motion {
            epoch: Epoch([3; 16]),
            segment: 2,
            captured_ns: 99,
            totals: Totals {
                sample: 7,
                axes: [-123, 456, -120, 120],
            },
        };
        for heartbeat in [false, true] {
            let bytes = motion.encode(heartbeat).unwrap();
            assert_eq!(&bytes[20..28], &2u64.to_le_bytes());
            assert_eq!(&bytes[44..52], &(-123i64).to_le_bytes());
            assert_eq!(Motion::decode(&bytes).unwrap(), (motion, heartbeat));
            for size in 0..MOTION_BYTES {
                assert!(Motion::decode(&bytes[..size]).is_err());
            }
            let mut bad = bytes;
            bad[79] = 1;
            assert!(Motion::decode(&bad).is_err());
            let mut bad = bytes;
            bad[36..44].fill(0);
            assert!(Motion::decode(&bad).is_err());
        }
        let critical = Critical {
            transition: Transition {
                epoch: motion.epoch,
                id: 3,
                captured_ns: 100,
                key: PhysicalKey {
                    usage_page: 9,
                    usage: 1,
                },
                down: true,
            },
            barrier: motion.totals,
        };
        let bytes = critical.encode().unwrap();
        assert_eq!(Critical::decode(&bytes).unwrap(), critical);
        for size in 0..CRITICAL_BYTES {
            assert!(Critical::decode(&bytes[..size]).is_err());
        }
        let mut totals = Totals {
            sample: u64::MAX,
            axes: [0; 4],
        };
        assert_eq!(totals.advance([1; 4]), Err(LiveError::Counter));
        assert_eq!(totals.axes, [0; 4]);
    }
}
