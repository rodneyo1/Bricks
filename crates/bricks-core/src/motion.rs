//! Cumulative relative motion with critical barriers. Only impaired-path future
//! segments allocate; healthy critical barriers use a single inline slot.
use crate::Injector;
use bricks_protocol::{
    Epoch, PhysicalKey, Transition,
    live::{Critical, Motion, Totals},
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

pub trait MotionInjector: Injector {
    fn submit_motion(&mut self, delta: [i32; 4]) -> Result<(), Self::Error>;
}
#[derive(Debug, thiserror::Error)]
pub enum Error<E> {
    #[error("invalid, conflicting or unbounded motion history")]
    History,
    #[error("motion/critical backend failed: {0}")]
    Backend(E),
}
pub struct State<I: MotionInjector> {
    pub injector: I,
    epoch: Epoch,
    segment: u64,
    totals: Totals,
    capacity: usize,
    next: Option<Critical>,
    barriers: BTreeMap<u64, Critical>,
    future: BTreeMap<u64, Motion>,
    pub submitted_totals: [i64; 4],
    pub motion_submissions: u64,
}
impl<I: MotionInjector> State<I> {
    pub fn new(epoch: Epoch, injector: I, capacity: usize) -> Self {
        assert!((1..=1024).contains(&capacity));
        Self {
            injector,
            epoch,
            segment: 0,
            totals: Totals::default(),
            capacity,
            next: None,
            barriers: BTreeMap::new(),
            future: BTreeMap::new(),
            submitted_totals: [0; 4],
            motion_submissions: 0,
        }
    }
    pub fn stage(&mut self, event: Critical, applied: u64) -> Result<(), Error<I::Error>> {
        if event.transition.epoch != self.epoch {
            return Err(Error::History);
        }
        let id = event.transition.id;
        if id <= applied {
            return Ok(());
        }
        if id - applied > self.capacity as u64 + 1 {
            return Err(Error::History);
        }
        if let Some(previous) = self.barriers.get(&id) {
            return if *previous == event {
                Ok(())
            } else {
                Err(Error::History)
            };
        }
        if id == applied + 1 {
            if let Some(previous) = self.next {
                if previous != event {
                    return Err(Error::History);
                }
            } else {
                self.next = Some(event);
            }
        } else if let Some(previous) = self.barriers.get(&id) {
            if *previous != event {
                return Err(Error::History);
            }
        } else {
            if self.barriers.len() >= self.capacity {
                return Err(Error::History);
            }
            self.barriers.insert(id, event);
        }
        Ok(())
    }
    pub fn motion(&mut self, event: Motion) -> Result<(), Error<I::Error>> {
        if event.epoch != self.epoch {
            return Err(Error::History);
        }
        if event.segment < self.segment {
            return Ok(());
        }
        if event.segment == self.segment {
            if event.totals.sample < self.totals.sample {
                return Ok(());
            }
            return self.apply(event.totals);
        }
        if event.segment - self.segment > self.capacity as u64 + 1 {
            return Err(Error::History);
        }
        if let Some(previous) = self.future.get(&event.segment) {
            if previous.totals.sample > event.totals.sample {
                return Ok(());
            }
            if previous.totals.sample == event.totals.sample && previous.totals != event.totals {
                return Err(Error::History);
            }
        } else if self.future.len() > self.capacity {
            return Err(Error::History);
        }
        self.future.insert(event.segment, event);
        Ok(())
    }
    fn apply(&mut self, totals: Totals) -> Result<(), Error<I::Error>> {
        if totals.sample < self.totals.sample
            || (totals.sample == 0 && totals.axes != [0; 4])
            || (totals.sample == self.totals.sample && totals != self.totals)
        {
            return Err(Error::History);
        }
        let mut delta = [0; 4];
        for (i, value) in delta.iter_mut().enumerate() {
            *value = totals.axes[i]
                .checked_sub(self.totals.axes[i])
                .and_then(|value| i32::try_from(value).ok())
                .ok_or(Error::History)?;
        }
        let mut submitted = self.submitted_totals;
        for (sum, value) in submitted.iter_mut().zip(delta) {
            *sum = sum.checked_add(i64::from(value)).ok_or(Error::History)?;
        }
        if delta != [0; 4] {
            self.injector.submit_motion(delta).map_err(Error::Backend)?;
            self.motion_submissions += 1;
        }
        self.submitted_totals = submitted;
        self.totals = totals;
        Ok(())
    }
}
pub struct Adapter<I: MotionInjector>(pub Rc<RefCell<State<I>>>);
impl<I: MotionInjector> Injector for Adapter<I> {
    type Error = Error<I::Error>;
    fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), Self::Error> {
        self.0
            .borrow_mut()
            .injector
            .submit(key, down)
            .map_err(Error::Backend)
    }
    fn before_transition(&mut self, event: Transition) -> Result<(), Self::Error> {
        let mut state = self.0.borrow_mut();
        if event.id.checked_sub(1) != Some(state.segment) {
            return Err(Error::History);
        }
        let critical = if state
            .next
            .as_ref()
            .is_some_and(|critical| critical.transition.id == event.id)
        {
            state.next.take()
        } else {
            state.barriers.remove(&event.id)
        }
        .ok_or(Error::History)?;
        if critical.transition != event {
            return Err(Error::History);
        }
        state.apply(critical.barrier)
    }
    fn after_transition(&mut self, event: Transition) -> Result<(), Self::Error> {
        let mut state = self.0.borrow_mut();
        state.segment = event.id;
        state.totals = Totals::default();
        if let Some(motion) = state.future.remove(&event.id) {
            state.apply(motion.totals)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Record(Vec<String>);
    impl Injector for Record {
        type Error = &'static str;
        fn submit(&mut self, _: PhysicalKey, down: bool) -> Result<(), Self::Error> {
            self.0.push(format!("key:{down}"));
            Ok(())
        }
    }
    impl MotionInjector for Record {
        fn submit_motion(&mut self, delta: [i32; 4]) -> Result<(), Self::Error> {
            self.0.push(format!("motion:{delta:?}"));
            Ok(())
        }
    }
    fn critical(id: u64, down: bool, barrier: Totals) -> Critical {
        Critical {
            transition: Transition {
                epoch: Epoch([1; 16]),
                id,
                captured_ns: id,
                key: PhysicalKey {
                    usage_page: 9,
                    usage: 1,
                },
                down,
            },
            barrier,
        }
    }
    fn motion(segment: u64, sample: u64, x: i64) -> Motion {
        Motion {
            epoch: Epoch([1; 16]),
            segment,
            captured_ns: 0,
            totals: Totals {
                sample,
                axes: [x, 0, 0, 0],
            },
        }
    }
    #[test]
    fn lost_motion_is_repaired_before_click_and_future_motion_waits_until_after_click() {
        let state = Rc::new(RefCell::new(State::new(
            Epoch([1; 16]),
            Record::default(),
            4,
        )));
        let mut session = crate::Session::new(Epoch([1; 16]), Adapter(state.clone()), 8);
        state.borrow_mut().motion(motion(0, 1, 2)).unwrap();
        state.borrow_mut().motion(motion(1, 2, 7)).unwrap(); // reordered beyond click
        let event = critical(
            1,
            true,
            Totals {
                sample: 3,
                axes: [9, 0, 0, 0],
            },
        );
        state.borrow_mut().stage(event, 0).unwrap();
        session.apply(event.transition).unwrap();
        assert_eq!(
            state.borrow().injector.0,
            vec![
                "motion:[2, 0, 0, 0]",
                "motion:[7, 0, 0, 0]",
                "key:true",
                "motion:[7, 0, 0, 0]"
            ]
        );
        state.borrow_mut().motion(motion(0, 4, 123)).unwrap(); // old segment cannot cross click
        state.borrow_mut().motion(motion(1, 1, 3)).unwrap(); // older sample cannot undo motion
        assert_eq!(state.borrow().submitted_totals, [16, 0, 0, 0]);
        assert!(session.stop().is_empty());
    }
    #[test]
    fn critical_recovery_drains_barriers_exactly_once_and_rejects_conflicts() {
        let epoch = Epoch([1; 16]);
        let state = Rc::new(RefCell::new(State::new(epoch, Record::default(), 4)));
        let policy = crate::recovery::Policy {
            send_capacity: 8,
            receive_capacity: 4,
            retry_ms: 2,
            recovery_ms: 30,
            silence_ms: 250,
        };
        let mut receiver =
            crate::recovery::Receiver::new(epoch, Adapter(state.clone()), 8, policy, 0).unwrap();
        let second = critical(
            2,
            false,
            Totals {
                sample: 1,
                axes: [4, 0, 0, 0],
            },
        );
        let first = critical(1, true, Totals::default());
        state.borrow_mut().stage(second, 0).unwrap();
        receiver.receive(second.transition, 0).unwrap();
        state.borrow_mut().stage(first, 0).unwrap();
        receiver.receive(first.transition, 1).unwrap();
        assert_eq!(receiver.ack().through, 2);
        state.borrow_mut().stage(second, 2).unwrap();
        receiver.receive(second.transition, 2).unwrap();
        assert_eq!(
            state.borrow().injector.0,
            vec!["key:true", "motion:[4, 0, 0, 0]", "key:false"]
        );
        assert!(state.borrow_mut().motion(motion(2, 1, i64::MAX)).is_err());
        assert!(state.borrow_mut().motion(motion(8, 1, 1)).is_err());
        state.borrow_mut().motion(motion(2, 1, 1)).unwrap();
        assert!(state.borrow_mut().motion(motion(2, 1, 2)).is_err());
    }
}
