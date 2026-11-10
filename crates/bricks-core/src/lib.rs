//! Deterministic host state. Arrival is eligible immediately, never at a sampled time.
//! A missing transition is rejected for the later recovery layer to resolve.

use bricks_protocol::{CodecError, Epoch, PhysicalKey, Transition};
use thiserror::Error;

pub mod motion;
pub mod recovery;

#[derive(Debug, Error)]
pub enum ApplyError<E> {
    #[error("transition has invalid fields: {0}")]
    Invalid(CodecError),
    #[error("no active ownership grant")]
    Inactive,
    #[error("wrong session epoch")]
    Epoch,
    #[error("missing transition: expected {expected}, received {received}")]
    Gap { expected: u64, received: u64 },
    #[error("owned key capacity reached")]
    Capacity,
    #[error("injection failed: {0}")]
    Injection(E),
}

pub trait Injector {
    type Error;
    /// Success means the backend accepted the submission, not that a game consumed it.
    fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), Self::Error>;
    /// Optional causal motion barrier, only for a contiguous new transition.
    fn before_transition(&mut self, _event: Transition) -> Result<(), Self::Error> {
        Ok(())
    }
    fn after_transition(&mut self, _event: Transition) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Applied {
    Submitted,
    /// An OS repeat down or unmatched release changes no owned state.
    StateUnchanged,
    Duplicate,
}

pub struct Session<I: Injector> {
    epoch: Epoch,
    next_id: Option<u64>,
    pressed: Vec<PhysicalKey>,
    max_pressed: usize,
    active: bool,
    injector: I,
}

impl<I: Injector> Session<I> {
    /// The caller must authenticate and grant ownership before creating a session.
    pub fn new(epoch: Epoch, injector: I, max_pressed: usize) -> Self {
        Self {
            epoch,
            next_id: Some(1),
            pressed: Vec::with_capacity(max_pressed),
            max_pressed,
            active: true,
            injector,
        }
    }

    pub fn apply(&mut self, event: Transition) -> Result<Applied, ApplyError<I::Error>> {
        event.validate().map_err(ApplyError::Invalid)?;
        if !self.active {
            return Err(ApplyError::Inactive);
        }
        if event.epoch != self.epoch {
            return Err(ApplyError::Epoch);
        }
        let Some(next) = self.next_id else {
            return Ok(Applied::Duplicate);
        };
        if event.id < next {
            return Ok(Applied::Duplicate);
        }
        if event.id != next {
            return Err(ApplyError::Gap {
                expected: next,
                received: event.id,
            });
        }
        let was_down = self.pressed.contains(&event.key);
        if event.down && !was_down && self.pressed.len() >= self.max_pressed {
            return Err(ApplyError::Capacity);
        }
        if let Err(error) = self.injector.before_transition(event) {
            self.active = false;
            return Err(ApplyError::Injection(error));
        }
        if event.down == was_down {
            if let Err(error) = self.injector.after_transition(event) {
                self.active = false;
                return Err(ApplyError::Injection(error));
            }
            self.next_id = event.id.checked_add(1);
            return Ok(Applied::StateUnchanged);
        }
        // Track a possibly submitted down even on backend failure. Cleanup must attempt
        // its release, including when an OS API reported ambiguous/partial success.
        if event.down {
            let index = self.pressed.binary_search(&event.key).unwrap_err();
            self.pressed.insert(index, event.key);
        }
        if let Err(error) = self.injector.submit(event.key, event.down) {
            self.active = false;
            return Err(ApplyError::Injection(error));
        }
        if !event.down {
            let index = self
                .pressed
                .binary_search(&event.key)
                .expect("owned release");
            self.pressed.remove(index);
        }
        if let Err(error) = self.injector.after_transition(event) {
            self.active = false;
            return Err(ApplyError::Injection(error));
        }
        self.next_id = event.id.checked_add(1);
        Ok(Applied::Submitted)
    }

    /// Invalidate first; attempt every owned release even if one fails. Failed
    /// releases remain tracked so a supervising caller can retry cleanup.
    pub fn stop(&mut self) -> Vec<(PhysicalKey, I::Error)> {
        self.active = false;
        let mut failures = Vec::new();
        let injector = &mut self.injector;
        self.pressed
            .retain(|key| match injector.submit(*key, false) {
                Ok(()) => false,
                Err(error) => {
                    failures.push((*key, error));
                    true
                }
            });
        failures
    }

    pub fn pressed_count(&self) -> usize {
        self.pressed.len()
    }
    pub fn injector(&self) -> &I {
        &self.injector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fixture {
        events: Vec<(PhysicalKey, bool)>,
        fail_next: bool,
    }

    impl Injector for Fixture {
        type Error = &'static str;
        fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), Self::Error> {
            if std::mem::take(&mut self.fail_next) {
                return Err("fixture failure");
            }
            self.events.push((key, down));
            Ok(())
        }
    }

    fn event(id: u64, usage: u16, down: bool) -> Transition {
        Transition {
            epoch: Epoch([1; 16]),
            id,
            captured_ns: id * 100,
            key: PhysicalKey {
                usage_page: 7,
                usage,
            },
            down,
        }
    }

    #[test]
    fn preserves_order_deduplicates_and_leaves_repeat_to_host() {
        let mut session = Session::new(Epoch([1; 16]), Fixture::default(), 16);
        assert_eq!(
            session.apply(event(1, 4, true)).unwrap(),
            Applied::Submitted
        );
        assert_eq!(
            session.apply(event(1, 4, true)).unwrap(),
            Applied::Duplicate
        );
        assert_eq!(
            session.apply(event(2, 4, true)).unwrap(),
            Applied::StateUnchanged
        );
        assert!(matches!(
            session.apply(event(4, 4, false)),
            Err(ApplyError::Gap { expected: 3, .. })
        ));
        assert_eq!(session.pressed_count(), 1);
        session.apply(event(3, 5, true)).unwrap();
        session.apply(event(4, 4, false)).unwrap();
        assert_eq!(session.injector().events.len(), 3);
        assert!(session.stop().is_empty());
        assert_eq!(session.pressed_count(), 0);
        assert!(matches!(
            session.apply(event(5, 5, false)),
            Err(ApplyError::Inactive)
        ));
    }

    #[test]
    fn wrong_epoch_does_not_change_state() {
        let mut session = Session::new(Epoch([2; 16]), Fixture::default(), 16);
        assert!(matches!(
            session.apply(event(1, 4, true)),
            Err(ApplyError::Epoch)
        ));
        assert_eq!(session.pressed_count(), 0);
        assert!(session.injector().events.is_empty());
    }

    #[test]
    fn ambiguous_injection_failure_is_tracked_and_stops_session() {
        let fixture = Fixture {
            fail_next: true,
            ..Fixture::default()
        };
        let mut session = Session::new(Epoch([1; 16]), fixture, 16);
        assert!(matches!(
            session.apply(event(1, 4, true)),
            Err(ApplyError::Injection(_))
        ));
        assert_eq!(session.pressed_count(), 1);
        assert!(matches!(
            session.apply(event(2, 5, true)),
            Err(ApplyError::Inactive)
        ));
        assert!(session.stop().is_empty());
        assert_eq!(session.pressed_count(), 0);
    }

    #[test]
    fn cleanup_continues_after_failure_and_can_be_retried() {
        let mut session = Session::new(Epoch([1; 16]), Fixture::default(), 16);
        session.apply(event(1, 4, true)).unwrap();
        session.apply(event(2, 5, true)).unwrap();
        session.injector.fail_next = true;
        assert_eq!(session.stop().len(), 1);
        assert_eq!(session.pressed_count(), 1);
        assert!(session.stop().is_empty());
        assert_eq!(session.pressed_count(), 0);
    }

    #[test]
    fn capacity_is_bounded_without_consuming_the_transition() {
        let mut session = Session::new(Epoch([1; 16]), Fixture::default(), 1);
        session.apply(event(1, 4, true)).unwrap();
        assert!(matches!(
            session.apply(event(2, 5, true)),
            Err(ApplyError::Capacity)
        ));
        assert_eq!(
            session.apply(event(2, 4, false)).unwrap(),
            Applied::Submitted
        );
    }
}
