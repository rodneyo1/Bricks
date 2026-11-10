//! Transport-independent critical-key recovery. Callers authenticate/grant the epoch
//! before construction, drive timers, and serialize delivery, revocation and stop.
//! Milliseconds here are local monotonic time, never cross-machine timestamps.

use crate::{Applied, ApplyError, Injector, Session};
use bricks_protocol::{Epoch, PhysicalKey, Transition};
use std::collections::{BTreeMap, VecDeque};
use thiserror::Error;

#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub send_capacity: usize,
    pub receive_capacity: usize,
    pub retry_ms: u64,
    pub recovery_ms: u64,
    pub silence_ms: u64,
}

impl Policy {
    pub fn validate(self) -> Result<Self, Fault> {
        if !(1..=4096).contains(&self.send_capacity)
            || !(1..=1024).contains(&self.receive_capacity)
            || self.retry_ms == 0
            || self.retry_ms >= self.recovery_ms
            || self.recovery_ms > self.silence_ms
            || self.silence_ms > 1000
        {
            return Err(Fault::Policy);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum Fault {
    #[error("invalid recovery limits or timer relationships")]
    Policy,
    #[error("session is stopped; a newly authorized epoch is required")]
    Inactive,
    #[error("wrong session epoch")]
    Epoch,
    #[error("invalid transition")]
    Invalid,
    #[error("local monotonic clock went backwards")]
    Clock,
    #[error("bounded send history is full")]
    SendOverflow,
    #[error("bounded receive reorder window is full or exceeded")]
    ReceiveOverflow,
    #[error("critical history recovery deadline expired")]
    Deadline,
    #[error("authenticated input silence deadline expired")]
    Silence,
    #[error("ACK claims a transition never sent in this epoch")]
    AckAhead,
    #[error("conflicting copies of a buffered transition")]
    Conflict,
    #[error("transition counter exhausted")]
    Exhausted,
    #[error("host state rejected a transition or injection failed")]
    Application,
}

/// Highest contiguous ID accepted by the injector/state core, not game consumption.
/// The future wire envelope must bind feedback to the authenticated session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppliedAck {
    pub epoch: Epoch,
    pub through: u64,
}

struct Pending {
    event: Transition,
    first_ms: u64,
    last_ms: u64,
}

/// Retains exact transitions (including capture timestamps) until cumulative ACK.
/// `enqueue` returns the initial immediate send; `retry` returns at most one oldest
/// due transition per call. The caller must pace calls and report send failures.
pub struct Sender {
    epoch: Epoch,
    policy: Policy,
    pending: VecDeque<Pending>,
    last_id: u64,
    last_ms: u64,
    active: bool,
}

impl Sender {
    pub fn new(epoch: Epoch, policy: Policy, now_ms: u64) -> Result<Self, Fault> {
        Ok(Self {
            epoch,
            policy: policy.validate()?,
            pending: VecDeque::with_capacity(policy.send_capacity),
            last_id: 0,
            last_ms: now_ms,
            active: true,
        })
    }

    fn check(&mut self, now_ms: u64) -> Result<(), Fault> {
        if !self.active {
            return Err(Fault::Inactive);
        }
        let fault = if now_ms < self.last_ms {
            Some(Fault::Clock)
        } else if self
            .pending
            .front()
            .is_some_and(|p| now_ms - p.first_ms >= self.policy.recovery_ms)
        {
            Some(Fault::Deadline)
        } else {
            None
        };
        if let Some(fault) = fault {
            self.stop();
            return Err(fault);
        }
        self.last_ms = now_ms;
        Ok(())
    }

    pub fn enqueue(
        &mut self,
        key: PhysicalKey,
        down: bool,
        captured_ns: u64,
        now_ms: u64,
    ) -> Result<Transition, Fault> {
        self.check(now_ms)?;
        let Some(id) = self.last_id.checked_add(1) else {
            self.stop();
            return Err(Fault::Exhausted);
        };
        let event = Transition {
            epoch: self.epoch,
            id,
            captured_ns,
            key,
            down,
        };
        // Malformed local capture is an error but does not consume an ID.
        event.validate().map_err(|_| Fault::Invalid)?;
        if self.pending.len() == self.policy.send_capacity {
            self.stop();
            return Err(Fault::SendOverflow);
        }
        self.last_id = id;
        self.pending.push_back(Pending {
            event,
            first_ms: now_ms,
            last_ms: now_ms,
        });
        Ok(event)
    }

    pub fn acknowledge(&mut self, ack: AppliedAck, now_ms: u64) -> Result<(), Fault> {
        self.check(now_ms)?;
        if ack.epoch != self.epoch {
            return Err(Fault::Epoch);
        }
        if ack.through > self.last_id {
            self.stop();
            return Err(Fault::AckAhead);
        }
        while self
            .pending
            .front()
            .is_some_and(|p| p.event.id <= ack.through)
        {
            self.pending.pop_front();
        }
        Ok(())
    }

    pub fn retry(&mut self, now_ms: u64) -> Result<Option<Transition>, Fault> {
        self.check(now_ms)?;
        if let Some(pending) = self
            .pending
            .iter_mut()
            .find(|p| now_ms - p.last_ms >= self.policy.retry_ms)
        {
            pending.last_ms = now_ms;
            Ok(Some(pending.event))
        } else {
            Ok(None)
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Nearest actionable deadline. An event-driven adapter need not poll at
    /// 1 kHz when neither feedback nor recovery work is due.
    pub fn next_wake_ms(&self) -> Option<u64> {
        if !self.active {
            return None;
        }
        self.pending
            .iter()
            .map(|pending| {
                pending
                    .last_ms
                    .saturating_add(self.policy.retry_ms)
                    .min(pending.first_ms.saturating_add(self.policy.recovery_ms))
            })
            .min()
    }

    /// Authenticated idle liveness must advertise produced history, so a lost
    /// final release cannot be hidden by continuing heartbeats.
    pub fn heartbeat(&mut self, now_ms: u64) -> Result<(Epoch, u64), Fault> {
        self.check(now_ms)?;
        Ok((self.epoch, self.last_id))
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.pending.clear();
    }
}

/// State is invalidated before cleanup. Unreleased keys stay owned for retry;
/// callers must inspect this result and retain the receiver when cleanup fails.
#[derive(Debug)]
pub struct ReceiveError<E> {
    pub fault: Fault,
    pub application_error: Option<ApplyError<E>>,
    pub cleanup_failures: Vec<(PhysicalKey, E)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Delivery {
    pub ack: AppliedAck,
    pub submitted: usize,
    pub unchanged: usize,
    pub buffered: bool,
    pub duplicate: bool,
}

pub struct Receiver<I: Injector> {
    epoch: Epoch,
    policy: Policy,
    session: Session<I>,
    pending: BTreeMap<u64, Transition>,
    applied: u64,
    known_produced_through: u64,
    // Increasing produced watermarks and when each new range first became known.
    // At most receive_capacity + 1 ranges can lie ahead of contiguous application.
    noticed_ranges: VecDeque<(u64, u64)>,
    gap_since: Option<u64>,
    last_valid_ms: u64,
    last_ms: u64,
    active: bool,
}

impl<I: Injector> Receiver<I> {
    pub fn new(
        epoch: Epoch,
        injector: I,
        max_pressed: usize,
        policy: Policy,
        now_ms: u64,
    ) -> Result<Self, Fault> {
        Ok(Self {
            epoch,
            policy: policy.validate()?,
            session: Session::new(epoch, injector, max_pressed),
            pending: BTreeMap::new(),
            applied: 0,
            known_produced_through: 0,
            noticed_ranges: VecDeque::with_capacity(policy.receive_capacity + 1),
            gap_since: None,
            last_valid_ms: now_ms,
            last_ms: now_ms,
            active: true,
        })
    }

    fn fail(&mut self, fault: Fault) -> ReceiveError<I::Error> {
        ReceiveError {
            fault,
            application_error: None,
            cleanup_failures: self.stop(),
        }
    }

    /// Timers must be polled while idle too; invalid/stale packets never reset them.
    pub fn tick(&mut self, now_ms: u64) -> Result<(), ReceiveError<I::Error>> {
        if !self.active {
            return Err(ReceiveError {
                fault: Fault::Inactive,
                application_error: None,
                cleanup_failures: vec![],
            });
        }
        let fault = if now_ms < self.last_ms {
            Some(Fault::Clock)
        } else if self
            .gap_since
            .is_some_and(|t| now_ms - t >= self.policy.recovery_ms)
        {
            Some(Fault::Deadline)
        } else if now_ms - self.last_valid_ms >= self.policy.silence_ms {
            Some(Fault::Silence)
        } else {
            None
        };
        if let Some(fault) = fault {
            return Err(self.fail(fault));
        }
        self.last_ms = now_ms;
        Ok(())
    }

    pub fn receive(
        &mut self,
        event: Transition,
        now_ms: u64,
    ) -> Result<Delivery, ReceiveError<I::Error>> {
        self.tick(now_ms)?;
        if event.epoch != self.epoch {
            // A delayed previous-epoch record is discarded, without a liveness refresh.
            return Err(ReceiveError {
                fault: Fault::Epoch,
                application_error: None,
                cleanup_failures: vec![],
            });
        }
        if event.validate().is_err() {
            return Err(self.fail(Fault::Invalid));
        }
        let mut delivery = Delivery {
            ack: self.ack(),
            submitted: 0,
            unchanged: 0,
            buffered: false,
            duplicate: false,
        };
        if event.id <= self.applied {
            self.last_valid_ms = now_ms;
            delivery.duplicate = true;
            return Ok(delivery);
        }
        let expected = self.applied + 1; // An applied u64::MAX is handled by the duplicate branch.
        if event.id > expected {
            if let Some(previous) = self.pending.get(&event.id) {
                if *previous != event {
                    return Err(self.fail(Fault::Conflict));
                }
                delivery.duplicate = true;
            } else {
                if event.id - expected > self.policy.receive_capacity as u64
                    || self.pending.len() == self.policy.receive_capacity
                {
                    return Err(self.fail(Fault::ReceiveOverflow));
                }
                self.pending.insert(event.id, event);
            }
            self.notice_history(event.id, now_ms);
            self.last_valid_ms = now_ms;
            delivery.buffered = true;
            return Ok(delivery);
        }
        if self.known_produced_through == self.applied {
            // Healthy in-order delivery needs no gap bookkeeping or allocation.
            self.known_produced_through = event.id;
        } else {
            self.notice_history(event.id, now_ms);
        }
        let mut next = Some(event);
        while let Some(event) = next {
            match self.session.apply(event) {
                Ok(Applied::Submitted) => delivery.submitted += 1,
                Ok(Applied::StateUnchanged) => delivery.unchanged += 1,
                Ok(Applied::Duplicate) => unreachable!("receiver only submits contiguous new IDs"),
                Err(error) => {
                    let mut failure = self.fail(Fault::Application);
                    failure.application_error = Some(error);
                    return Err(failure);
                }
            }
            self.applied = event.id;
            next = self
                .applied
                .checked_add(1)
                .and_then(|id| self.pending.remove(&id));
        }
        // Retain the age of still-missing old history, but retire fully repaired
        // ranges so a newer hole cannot inherit an already resolved gap's age.
        while self
            .noticed_ranges
            .front()
            .is_some_and(|&(through, _)| through <= self.applied)
        {
            self.noticed_ranges.pop_front();
        }
        self.gap_since = self.noticed_ranges.front().map(|&(_, first_ms)| first_ms);
        self.last_valid_ms = now_ms;
        delivery.ack = self.ack();
        Ok(delivery)
    }

    fn notice_history(&mut self, through: u64, now_ms: u64) {
        if through <= self.known_produced_through {
            return;
        }
        // Callers have checked the bounded receive window. Each increasing
        // watermark occupies at least one ID in that window, so this never grows
        // the preallocated deque, including for adversarial valid heartbeats.
        debug_assert!(self.noticed_ranges.len() <= self.policy.receive_capacity);
        self.noticed_ranges.push_back((through, now_ms));
        self.known_produced_through = through;
        self.gap_since = self.noticed_ranges.front().map(|&(_, first_ms)| first_ms);
    }

    pub fn ack(&self) -> AppliedAck {
        AppliedAck {
            epoch: self.epoch,
            through: self.applied,
        }
    }
    pub fn next_wake_ms(&self) -> Option<u64> {
        self.active.then(|| {
            self.gap_since
                .map(|gap| gap.saturating_add(self.policy.recovery_ms))
                .unwrap_or(u64::MAX)
                .min(self.last_valid_ms.saturating_add(self.policy.silence_ms))
        })
    }
    pub fn keep_alive(
        &mut self,
        epoch: Epoch,
        produced_through: u64,
        now_ms: u64,
    ) -> Result<AppliedAck, ReceiveError<I::Error>> {
        self.tick(now_ms)?;
        if epoch != self.epoch {
            return Err(ReceiveError {
                fault: Fault::Epoch,
                application_error: None,
                cleanup_failures: vec![],
            });
        }
        if produced_through < self.applied {
            return Ok(self.ack());
        }
        if produced_through - self.applied > self.policy.receive_capacity as u64 {
            return Err(self.fail(Fault::ReceiveOverflow));
        }
        if produced_through > self.applied {
            self.notice_history(produced_through, now_ms);
        }
        self.last_valid_ms = now_ms;
        Ok(self.ack())
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn pressed_count(&self) -> usize {
        self.session.pressed_count()
    }
    pub fn injector(&self) -> &I {
        self.session.injector()
    }

    /// Call on disconnect, revocation, escape and remote sender failure. This
    /// never reactivates the epoch; retries only attempt outstanding releases.
    pub fn stop(&mut self) -> Vec<(PhysicalKey, I::Error)> {
        self.active = false;
        self.pending.clear();
        self.noticed_ranges.clear();
        self.gap_since = None;
        self.session.stop()
    }
}

impl<I: Injector> Drop for Receiver<I> {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default)]
    struct Record {
        events: Vec<(PhysicalKey, bool)>,
        fail_down: bool,
        fail_release: bool,
    }
    #[derive(Clone, Default)]
    struct Recorder(Rc<RefCell<Record>>);
    impl Injector for Recorder {
        type Error = &'static str;
        fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), Self::Error> {
            let mut record = self.0.borrow_mut();
            if (down && record.fail_down) || (!down && record.fail_release) {
                return Err("injected failure");
            }
            record.events.push((key, down));
            Ok(())
        }
    }
    fn policy() -> Policy {
        Policy {
            send_capacity: 8,
            receive_capacity: 4,
            retry_ms: 2,
            recovery_ms: 30,
            silence_ms: 250,
        }
    }
    fn key(usage: u16) -> PhysicalKey {
        PhysicalKey {
            usage_page: 7,
            usage,
        }
    }
    fn event(id: u64, down: bool) -> Transition {
        Transition {
            epoch: Epoch([1; 16]),
            id,
            captured_ns: id * 100,
            key: key(4),
            down,
        }
    }
    fn receiver() -> Receiver<Recorder> {
        Receiver::new(Epoch([1; 16]), Recorder::default(), 16, policy(), 0).unwrap()
    }

    #[test]
    fn healthy_events_have_no_intentional_delay_and_ack_after_submission() {
        let mut rx = receiver();
        let delivery = rx.receive(event(1, true), 0).unwrap();
        assert_eq!(
            (delivery.submitted, delivery.ack.through, delivery.buffered),
            (1, 1, false)
        );
        assert_eq!(rx.injector().0.borrow().events, vec![(key(4), true)]);
        assert_eq!(rx.receive(event(2, false), 0).unwrap().ack.through, 2);
        assert_eq!(rx.pressed_count(), 0);
    }

    #[test]
    fn heartbeats_preserve_long_holds_but_cannot_hide_missing_release_or_old_epochs() {
        let epoch = Epoch([1; 16]);
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        for now in (20..=10000).step_by(20) {
            rx.keep_alive(epoch, 1, now).unwrap();
        }
        assert_eq!(rx.pressed_count(), 1);
        assert_eq!(rx.injector().0.borrow().events.len(), 1);
        rx.keep_alive(epoch, 2, 10001).unwrap();
        for now in [10010, 10020, 10030] {
            rx.keep_alive(epoch, 2, now).unwrap();
        }
        assert_eq!(rx.tick(10031).unwrap_err().fault, Fault::Deadline);
        assert_eq!(rx.pressed_count(), 0);
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        assert_eq!(
            rx.keep_alive(Epoch([2; 16]), 1, 249).unwrap_err().fault,
            Fault::Epoch
        );
        assert_eq!(rx.tick(250).unwrap_err().fault, Fault::Silence);
        let mut tx = Sender::new(epoch, policy(), 0).unwrap();
        tx.enqueue(key(4), true, 0, 0).unwrap();
        assert_eq!(tx.heartbeat(1).unwrap(), (epoch, 1));
        assert_eq!(tx.heartbeat(30).unwrap_err(), Fault::Deadline);
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        rx.keep_alive(epoch, 3, 1).unwrap();
        rx.receive(event(2, false), 10).unwrap();
        assert_eq!(
            rx.tick(31).unwrap_err().fault,
            Fault::Deadline,
            "Partial history must not renew a heartbeat-discovered gap"
        );
    }
    #[test]
    fn recovers_a_lost_final_release_without_new_capture_or_ack() {
        let mut tx = Sender::new(Epoch([1; 16]), policy(), 0).unwrap();
        let mut rx = receiver();
        let down = tx.enqueue(key(4), true, 123, 0).unwrap();
        tx.acknowledge(rx.receive(down, 0).unwrap().ack, 0).unwrap();
        let lost_up = tx.enqueue(key(4), false, 456, 1).unwrap();
        assert_eq!(tx.retry(2).unwrap(), None);
        let retry = tx.retry(3).unwrap().unwrap();
        assert_eq!(retry, lost_up);
        tx.acknowledge(rx.receive(retry, 3).unwrap().ack, 3)
            .unwrap();
        assert_eq!((tx.pending_count(), rx.pressed_count()), (0, 0));
    }

    #[test]
    fn all_small_permutations_and_initial_drop_patterns_preserve_exact_history() {
        fn permutations(ids: &mut [usize], offset: usize, out: &mut Vec<Vec<usize>>) {
            if offset == ids.len() {
                out.push(ids.to_vec());
                return;
            }
            for i in offset..ids.len() {
                ids.swap(offset, i);
                permutations(ids, offset + 1, out);
                ids.swap(offset, i);
            }
        }
        let mut orders = vec![];
        permutations(&mut [0, 1, 2, 3], 0, &mut orders);
        for order in orders {
            for dropped in 0..16 {
                let mut tx = Sender::new(Epoch([1; 16]), policy(), 0).unwrap();
                let mut rx = receiver();
                let events: Vec<_> = (0..4)
                    .map(|i| {
                        tx.enqueue(key(4 + i / 2), i % 2 == 0, u64::from(i), 0)
                            .unwrap()
                    })
                    .collect();
                for &i in &order {
                    if dropped & (1 << i) == 0 {
                        let ack = rx.receive(events[i], 0).unwrap().ack;
                        // Deliver each packet twice, including ones still buffered.
                        assert!(rx.receive(events[i], 0).unwrap().duplicate);
                        tx.acknowledge(ack, 0).unwrap();
                    }
                }
                for now in 2..20 {
                    while let Some(retry) = tx.retry(now).unwrap() {
                        let ack = rx.receive(retry, now).unwrap().ack;
                        tx.acknowledge(ack, now).unwrap();
                    }
                    if tx.pending_count() == 0 {
                        break;
                    }
                }
                assert_eq!(
                    tx.pending_count(),
                    0,
                    "drop mask {dropped}, order {order:?}"
                );
                assert_eq!(
                    rx.injector().0.borrow().events,
                    vec![
                        (key(4), true),
                        (key(4), false),
                        (key(5), true),
                        (key(5), false)
                    ]
                );
                assert_eq!(rx.pressed_count(), 0);
            }
        }
    }

    #[test]
    fn lost_and_reordered_ack_feedback_does_not_repeat_injection() {
        let mut tx = Sender::new(Epoch([1; 16]), policy(), 0).unwrap();
        let mut rx = receiver();
        let down = tx.enqueue(key(4), true, 0, 0).unwrap();
        let ack1 = rx.receive(down, 0).unwrap().ack; // dropped
        let retry = tx.retry(2).unwrap().unwrap();
        assert!(rx.receive(retry, 2).unwrap().duplicate);
        let up = tx.enqueue(key(4), false, 0, 2).unwrap();
        let ack2 = rx.receive(up, 2).unwrap().ack;
        tx.acknowledge(ack2, 2).unwrap();
        tx.acknowledge(ack1, 3).unwrap();
        assert_eq!(tx.pending_count(), 0);
        assert_eq!(rx.injector().0.borrow().events.len(), 2);
    }

    #[test]
    fn event_driven_deadlines_recover_idle_release_without_extending_a_gap() {
        let mut tx = Sender::new(Epoch([1; 16]), policy(), 0).unwrap();
        assert_eq!(tx.next_wake_ms(), None);
        let down = tx.enqueue(key(4), true, 0, 0).unwrap();
        let mut rx = receiver();
        tx.acknowledge(rx.receive(down, 0).unwrap().ack, 0).unwrap();
        tx.enqueue(key(4), false, 1, 1).unwrap(); // final release lost
        assert_eq!(tx.next_wake_ms(), Some(3));
        let retry = tx.retry(tx.next_wake_ms().unwrap()).unwrap().unwrap();
        tx.acknowledge(rx.receive(retry, 3).unwrap().ack, 3)
            .unwrap();
        assert_eq!(tx.next_wake_ms(), None);
        assert_eq!(rx.pressed_count(), 0);
        assert_eq!(rx.next_wake_ms(), Some(253));
        rx.receive(event(5, true), 4).unwrap();
        assert_eq!(rx.next_wake_ms(), Some(34));
        rx.receive(event(5, true), 30).unwrap();
        assert_eq!(rx.next_wake_ms(), Some(34));
        assert_eq!(rx.tick(34).unwrap_err().fault, Fault::Deadline);
        assert_eq!(rx.next_wake_ms(), None);
        tx.stop();
        assert_eq!(tx.next_wake_ms(), None);
    }

    #[test]
    fn resolved_gap_does_not_age_newly_missing_history() {
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        rx.receive(event(3, true), 1).unwrap(); // missing 2, noticed at 1
        rx.receive(event(5, true), 20).unwrap(); // missing 4, noticed at 20
        assert_eq!(rx.next_wake_ms(), Some(31));
        assert_eq!(rx.receive(event(2, false), 25).unwrap().ack.through, 3);
        assert_eq!(rx.next_wake_ms(), Some(50));
        rx.tick(31).unwrap(); // remaining missing history is only 11 ms old
        rx.receive(event(5, true), 40).unwrap(); // duplicates cannot renew it
        assert_eq!(rx.next_wake_ms(), Some(50));
        assert_eq!(rx.receive(event(4, false), 49).unwrap().ack.through, 5);
        assert!(rx.stop().is_empty());
    }

    #[test]
    fn resolved_heartbeat_gap_does_not_age_newly_missing_history() {
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        rx.keep_alive(Epoch([1; 16]), 3, 1).unwrap();
        rx.receive(event(5, true), 20).unwrap();
        rx.receive(event(2, false), 25).unwrap();
        assert_eq!(rx.next_wake_ms(), Some(31)); // missing 3 is still old
        rx.receive(event(3, true), 29).unwrap();
        assert_eq!(rx.next_wake_ms(), Some(50)); // only newer missing 4 remains
        rx.tick(31).unwrap();
        rx.keep_alive(Epoch([1; 16]), 5, 40).unwrap();
        assert_eq!(rx.next_wake_ms(), Some(50));
        assert_eq!(rx.tick(50).unwrap_err().fault, Fault::Deadline);
        assert_eq!(rx.pressed_count(), 0);
    }

    #[test]
    fn gap_deadline_is_not_extended_by_duplicates_or_partial_progress() {
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        rx.receive(event(5, false), 1).unwrap();
        rx.receive(event(5, false), 20).unwrap();
        rx.receive(event(2, true), 29).unwrap();
        let error = rx.receive(event(3, true), 31).unwrap_err();
        assert_eq!(error.fault, Fault::Deadline);
        assert_eq!(rx.pressed_count(), 0);
        assert_eq!(rx.pending_count(), 0);
        assert_eq!(rx.ack().through, 2); // discarded history was never ACKed as applied
        assert_eq!(
            rx.receive(event(3, true), 32).unwrap_err().fault,
            Fault::Inactive
        );
    }

    #[test]
    fn outage_watchdog_and_drop_attempt_recorded_key_release() {
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        assert!(rx.tick(249).is_ok());
        assert_eq!(rx.tick(250).unwrap_err().fault, Fault::Silence);
        assert_eq!(rx.pressed_count(), 0);
        let recorder = Recorder::default();
        {
            let mut rx = Receiver::new(Epoch([1; 16]), recorder.clone(), 16, policy(), 0).unwrap();
            rx.receive(event(1, true), 0).unwrap();
        }
        assert_eq!(
            recorder.0.borrow().events,
            vec![(key(4), true), (key(4), false)]
        );
    }

    #[test]
    fn overflow_stops_history_instead_of_silently_dropping_oldest() {
        let mut tx = Sender::new(
            Epoch([1; 16]),
            Policy {
                send_capacity: 1,
                ..policy()
            },
            0,
        )
        .unwrap();
        tx.enqueue(key(4), true, 0, 0).unwrap();
        assert_eq!(tx.enqueue(key(4), false, 0, 0), Err(Fault::SendOverflow));
        assert_eq!(tx.pending_count(), 0);
        assert_eq!(tx.retry(2), Err(Fault::Inactive));
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        assert_eq!(
            rx.receive(event(7, false), 1).unwrap_err().fault,
            Fault::ReceiveOverflow
        );
        assert_eq!(rx.pressed_count(), 0);
    }

    #[test]
    fn stale_epochs_neither_refresh_watchdog_nor_clear_new_send_history() {
        let mut tx = Sender::new(Epoch([2; 16]), policy(), 0).unwrap();
        tx.enqueue(key(4), true, 0, 0).unwrap();
        assert_eq!(
            tx.acknowledge(
                AppliedAck {
                    epoch: Epoch([1; 16]),
                    through: 1
                },
                1
            ),
            Err(Fault::Epoch)
        );
        assert_eq!(tx.pending_count(), 1);
        let mut rx = Receiver::new(Epoch([2; 16]), Recorder::default(), 16, policy(), 0).unwrap();
        assert_eq!(
            rx.receive(event(1, true), 249).unwrap_err().fault,
            Fault::Epoch
        );
        assert_eq!(rx.tick(250).unwrap_err().fault, Fault::Silence);
        assert!(rx.injector().0.borrow().events.is_empty());
    }

    #[test]
    fn contradictory_buffered_history_and_impossible_ack_are_rejected() {
        let mut rx = receiver();
        rx.receive(event(2, false), 0).unwrap();
        assert_eq!(
            rx.receive(event(2, true), 0).unwrap_err().fault,
            Fault::Conflict
        );
        let mut tx = Sender::new(Epoch([1; 16]), policy(), 0).unwrap();
        assert_eq!(
            tx.acknowledge(
                AppliedAck {
                    epoch: Epoch([1; 16]),
                    through: 1
                },
                0
            ),
            Err(Fault::AckAhead)
        );
        assert_eq!(tx.retry(1), Err(Fault::Inactive));
    }

    #[test]
    fn ambiguous_injection_and_failed_cleanup_remain_visible_and_retryable() {
        let recorder = Recorder::default();
        recorder.0.borrow_mut().fail_down = true;
        recorder.0.borrow_mut().fail_release = true;
        let mut rx = Receiver::new(Epoch([1; 16]), recorder.clone(), 16, policy(), 0).unwrap();
        let error = rx.receive(event(1, true), 0).unwrap_err();
        assert_eq!(error.fault, Fault::Application);
        assert!(matches!(
            error.application_error,
            Some(ApplyError::Injection(_))
        ));
        assert_eq!(error.cleanup_failures.len(), 1);
        assert_eq!(rx.pressed_count(), 1);
        assert_eq!(rx.ack().through, 0);
        recorder.0.borrow_mut().fail_release = false;
        assert!(rx.stop().is_empty());
        assert_eq!(rx.pressed_count(), 0);
    }

    #[test]
    fn timers_fail_closed_at_deadlines_and_on_clock_regression() {
        let mut tx = Sender::new(Epoch([1; 16]), policy(), 0).unwrap();
        tx.enqueue(key(4), true, 0, 0).unwrap();
        assert!(tx.retry(29).unwrap().is_some());
        assert_eq!(tx.retry(30), Err(Fault::Deadline));
        let mut rx = receiver();
        rx.receive(event(1, true), 10).unwrap();
        assert_eq!(rx.tick(9).unwrap_err().fault, Fault::Clock);
        assert_eq!(rx.pressed_count(), 0);
        let mut tx = Sender::new(Epoch([1; 16]), policy(), 10).unwrap();
        assert_eq!(tx.retry(9), Err(Fault::Clock));
    }

    #[test]
    fn invalid_fields_and_counter_wrap_cannot_create_new_history() {
        let mut tx = Sender::new(Epoch([1; 16]), policy(), 0).unwrap();
        assert_eq!(tx.enqueue(key(0), true, 0, 0), Err(Fault::Invalid));
        assert_eq!(tx.enqueue(key(4), true, 0, 0).unwrap().id, 1);
        tx.stop();
        let mut tx = Sender::new(Epoch([1; 16]), policy(), 0).unwrap();
        tx.last_id = u64::MAX;
        assert_eq!(tx.enqueue(key(4), true, 0, 0), Err(Fault::Exhausted));
        let mut rx = receiver();
        rx.receive(event(1, true), 0).unwrap();
        assert_eq!(
            rx.receive(event(0, false), 0).unwrap_err().fault,
            Fault::Invalid
        );
        assert_eq!(rx.pressed_count(), 0);
    }
}
