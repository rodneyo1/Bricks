//! Laboratory reference transport, not an authenticated remote-control backend.

pub mod auth;
pub mod identity;
pub mod lifecycle;
#[cfg(all(windows, target_pointer_width = "64"))]
pub mod live;
#[cfg(all(windows, target_pointer_width = "64"))]
pub mod live_verification;
pub mod ownership;
pub mod power;
pub mod probe;
pub mod recovery_check;
pub mod recovery_lab;
pub mod reliable;
pub mod resources;
pub mod timing;

use bricks_protocol::{TRANSITION_BYTES, Transition};
use std::{io, net::UdpSocket, time::Duration};

/// Both addresses are created internally on IPv4 loopback with ephemeral ports.
/// No externally supplied bind/peer is accepted. This cannot inject OS input.
pub struct LoopbackReference {
    sender: UdpSocket,
    echo: UdpSocket,
}

impl LoopbackReference {
    pub fn new() -> io::Result<Self> {
        let sender = UdpSocket::bind("127.0.0.1:0")?;
        let echo = UdpSocket::bind("127.0.0.1:0")?;
        sender.connect(echo.local_addr()?)?;
        echo.connect(sender.local_addr()?)?;
        for socket in [&sender, &echo] {
            socket.set_read_timeout(Some(Duration::from_secs(2)))?;
            socket.set_write_timeout(Some(Duration::from_secs(2)))?;
        }
        Ok(Self { sender, echo })
    }

    /// One sequential request/echo with codec validation at both endpoints.
    /// Timings include both directions and this same-thread echo implementation.
    pub fn roundtrip(&self, event: Transition) -> io::Result<Transition> {
        let invalid = |error| io::Error::new(io::ErrorKind::InvalidData, error);
        let wire = event.encode().map_err(invalid)?;
        if self.sender.send(&wire)? != TRANSITION_BYTES {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "partial UDP send"));
        }
        let mut buffer = [0; TRANSITION_BYTES + 1];
        let count = self.echo.recv(&mut buffer)?;
        let decoded = Transition::decode(&buffer[..count]).map_err(invalid)?;
        let reply = decoded.encode().map_err(invalid)?;
        if self.echo.send(&reply)? != TRANSITION_BYTES {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "partial UDP echo"));
        }
        let count = self.sender.recv(&mut buffer)?;
        let result = Transition::decode(&buffer[..count]).map_err(invalid)?;
        if result != event {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "echo identity mismatch",
            ));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bricks_protocol::{Epoch, PhysicalKey};

    #[test]
    fn real_loopback_datagrams_preserve_event_identity() {
        let transport = LoopbackReference::new().unwrap();
        assert!(transport.sender.local_addr().unwrap().ip().is_loopback());
        assert!(transport.echo.local_addr().unwrap().ip().is_loopback());
        for id in 1..=32 {
            let event = Transition {
                epoch: Epoch([0x42; 16]),
                id,
                captured_ns: id * 10,
                key: PhysicalKey {
                    usage_page: 7,
                    usage: 4,
                },
                down: id % 2 == 1,
            };
            assert_eq!(transport.roundtrip(event).unwrap(), event);
        }
    }
}
