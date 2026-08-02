//! Letting a verdict change on screen without letting the page move underneath the reader.
//!
//! A list ordered worst-first re-sorts every time any member's health changes, and health
//! near a threshold flickers: one lost packet crosses a line, the next window crosses back.
//! The row someone is reading then swaps places with its neighbour a second after they
//! started reading it, which is the single most effective way to make a page unusable.
//!
//! The fix is **not** to smooth the state. Smoothing would delay the answer, and the answers
//! this product exists to deliver are exactly the ones a user is waiting for. Instead the two
//! uses of a state are separated:
//!
//! * **What is shown changes immediately.** A badge that says "unreachable" says it the
//!   instant the measurement does. Nothing on screen is ever stale.
//! * **What the *ordering* uses lags**, and only adopts a new state once that state has held
//!   continuously for a stated period. A flicker back and forth therefore never moves a row,
//!   because the change never survives long enough to settle.
//!
//! Like everything in this crate it reads no clock: `now` is passed in.

use std::time::{Duration, Instant};

use crate::health::Health;

/// How long a health state must hold before the ordering adopts it.
///
/// Five seconds is several emissions at the rate the UI is fed, so a state that flickers
/// across a threshold from one window to the next never settles — while a genuine change,
/// which persists, moves the row about as fast as a reader can notice it did.
pub const ORDER_HOLD: Duration = Duration::from_secs(5);

/// A health state, and the possibly older one the ordering is still using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settling {
    shown: Health,
    settled: Health,
    /// When `shown` last became something other than what it was.
    since: Instant,
}

impl Settling {
    /// Starts from a first observation, with nothing yet to settle.
    #[must_use]
    pub const fn new(health: Health, now: Instant) -> Self {
        Self {
            shown: health,
            settled: health,
            since: now,
        }
    }

    /// Folds in the latest verdict.
    ///
    /// The shown state becomes `health` at once. The settled state follows only when the
    /// shown state has been the same thing for `hold` — and a state that returns to what the
    /// ordering already uses needs no settling at all, which is what makes a flicker free.
    pub fn observe(&mut self, health: Health, now: Instant, hold: Duration) {
        if health != self.shown {
            self.shown = health;
            self.since = now;
        }
        if self.shown == self.settled {
            return;
        }
        if now.saturating_duration_since(self.since) >= hold {
            self.settled = self.shown;
        }
    }

    /// What the badge says. Always the latest measurement.
    #[must_use]
    pub const fn shown(&self) -> Health {
        self.shown
    }

    /// What the ordering uses. Never newer than [`Settling::shown`].
    #[must_use]
    pub const fn settled(&self) -> Health {
        self.settled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_observation_settles_immediately() {
        // Nothing to hold still for: there is no order to disturb yet.
        let now = Instant::now();
        let settling = Settling::new(Health::Ok, now);
        assert_eq!(settling.shown(), Health::Ok);
        assert_eq!(settling.settled(), Health::Ok);
    }

    #[test]
    fn what_is_shown_changes_at_once_while_the_order_waits() {
        // The whole point. A user must never read a stale badge; a user must never have the
        // row they are reading move a second after they started.
        let now = Instant::now();
        let mut settling = Settling::new(Health::Ok, now);

        settling.observe(Health::Degraded, now, ORDER_HOLD);

        assert_eq!(
            settling.shown(),
            Health::Degraded,
            "the badge is never stale"
        );
        assert_eq!(settling.settled(), Health::Ok, "the row has not moved");
    }

    #[test]
    fn a_change_that_holds_is_adopted() {
        let now = Instant::now();
        let mut settling = Settling::new(Health::Ok, now);

        // The hold is counted from the moment the state changed, which is the first of
        // these — so five seconds of it is the sixth.
        for step in 1..=6 {
            settling.observe(
                Health::Degraded,
                now + Duration::from_secs(step),
                ORDER_HOLD,
            );
        }

        assert_eq!(settling.settled(), Health::Degraded);
    }

    #[test]
    fn a_flicker_never_moves_a_row() {
        // One lost packet crosses a threshold and the next window crosses back. Under a
        // plain sort that is a row swapping places with its neighbour every other second.
        let now = Instant::now();
        let mut settling = Settling::new(Health::Ok, now);

        for step in 0..60 {
            let health = if step % 2 == 0 {
                Health::Degraded
            } else {
                Health::Ok
            };
            settling.observe(health, now + Duration::from_secs(step), ORDER_HOLD);
            assert_eq!(
                settling.settled(),
                Health::Ok,
                "nothing held long enough to settle at step {step}"
            );
        }
    }

    #[test]
    fn returning_to_the_settled_state_costs_nothing() {
        // It left and came back before the hold elapsed, so there was never anything to
        // adopt — and the next genuine change must not inherit a head start from it.
        let now = Instant::now();
        let mut settling = Settling::new(Health::Ok, now);

        settling.observe(Health::Degraded, now + Duration::from_secs(1), ORDER_HOLD);
        settling.observe(Health::Ok, now + Duration::from_secs(2), ORDER_HOLD);
        settling.observe(
            Health::Unreachable,
            now + Duration::from_secs(3),
            ORDER_HOLD,
        );
        settling.observe(
            Health::Unreachable,
            now + Duration::from_secs(7),
            ORDER_HOLD,
        );

        assert_eq!(
            settling.settled(),
            Health::Ok,
            "four seconds of the new state is not five"
        );

        settling.observe(
            Health::Unreachable,
            now + Duration::from_secs(8),
            ORDER_HOLD,
        );
        assert_eq!(settling.settled(), Health::Unreachable);
    }

    #[test]
    fn a_clock_that_appears_to_move_backwards_settles_nothing_early() {
        let earlier = Instant::now();
        let now = earlier + Duration::from_secs(60);
        let mut settling = Settling::new(Health::Ok, now);

        settling.observe(Health::Unreachable, now, ORDER_HOLD);
        settling.observe(Health::Unreachable, earlier, ORDER_HOLD);

        assert_eq!(settling.settled(), Health::Ok);
    }

    #[test]
    fn a_zero_hold_adopts_everything_at_once() {
        // The degenerate case a caller could configure. It must behave like no hysteresis at
        // all rather than like an ordering that never changes again.
        let now = Instant::now();
        let mut settling = Settling::new(Health::Ok, now);

        settling.observe(Health::Blocked, now, Duration::ZERO);

        assert_eq!(settling.settled(), Health::Blocked);
    }
}
