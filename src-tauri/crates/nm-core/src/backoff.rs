//! Stretching the interval of a target that has stopped saying anything new.
//!
//! Backing off a *failing* endpoint sounds like the wrong instinct for a diagnostic tool —
//! the failure is what the user came to see. The rule that makes it right is that only
//! *unbroken* failure stretches the interval. A link losing half its packets keeps returning
//! successes, and every success puts the target back to full rate, so the measurement the
//! user is watching stays sharp. Backoff engages only where the answer has been identical
//! for several probes running, and there another probe a second later adds nothing while
//! spending budget that a measurable endpoint could use.
//!
//! Recovery is still noticed, just later — bounded by the maximum interval, which is why that
//! bound is part of the type rather than a convention.

use std::time::Duration;

use crate::sample::ProbeOutcome;
use crate::Error;

/// How many consecutive failures pass before the interval starts stretching.
///
/// A short burst of loss is ordinary and must not coarsen the very measurement being taken;
/// three in a row is long enough that the next probe is unlikely to say anything new.
pub const FAILURES_BEFORE_STRETCHING: u32 = 3;

/// Per-target interval that grows while a target keeps giving the same non-answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    consecutive_failures: u32,
}

impl Backoff {
    /// Creates a backoff that starts at `base` and never exceeds `max`.
    ///
    /// A `max` below `base` is raised to `base`: a ceiling under the floor is a configuration
    /// mistake, and honouring it literally would silently probe faster than asked.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroInterval`] if `base` is zero, which would make a target
    /// permanently due and starve every other one.
    pub fn new(base: Duration, max: Duration) -> Result<Self, Error> {
        if base.is_zero() {
            return Err(Error::ZeroInterval);
        }
        Ok(Self {
            base,
            max: max.max(base),
            consecutive_failures: 0,
        })
    }

    /// The interval to use for the next probe.
    #[must_use]
    pub fn interval(&self) -> Duration {
        let stretching = self
            .consecutive_failures
            .saturating_sub(FAILURES_BEFORE_STRETCHING);
        if stretching == 0 {
            return self.base;
        }
        // Saturating on both the shift and the multiply: an endpoint silent for hours must
        // land on `max`, not wrap around to probing furiously.
        let factor = 1_u32.checked_shl(stretching).unwrap_or(u32::MAX);
        self.base
            .checked_mul(factor)
            .unwrap_or(self.max)
            .min(self.max)
    }

    /// How many probes in a row have failed to say anything new.
    #[must_use]
    pub const fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Whether the interval has been stretched beyond its base.
    #[must_use]
    pub fn is_stretched(&self) -> bool {
        self.interval() > self.base
    }

    /// Folds one probe result in.
    ///
    /// A success is the only outcome that restores full rate. The other three are stable
    /// states — silence, a standing refusal, a filtered probe kind — and repeating them
    /// faster produces no new knowledge.
    pub fn record(&mut self, outcome: ProbeOutcome) {
        match outcome {
            ProbeOutcome::Success(_) => self.consecutive_failures = 0,
            ProbeOutcome::Timeout | ProbeOutcome::Unreachable | ProbeOutcome::Blocked => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            }
        }
    }

    /// Returns to full rate.
    ///
    /// Used when something has changed that makes the previous failures uninformative — a
    /// different probe kind taking over, for instance. The new attempt deserves the same
    /// chance the first one had.
    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::Rtt;

    const BASE: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(30);

    fn backoff() -> Backoff {
        Backoff::new(BASE, MAX).unwrap()
    }

    fn success() -> ProbeOutcome {
        ProbeOutcome::Success(Rtt::from_micros(9_000))
    }

    fn fail(backoff: &mut Backoff, times: u32) {
        for _ in 0..times {
            backoff.record(ProbeOutcome::Timeout);
        }
    }

    #[test]
    fn a_fresh_target_probes_at_the_base_interval() {
        let backoff = backoff();
        assert_eq!(backoff.interval(), BASE);
        assert_eq!(backoff.consecutive_failures(), 0);
        assert!(!backoff.is_stretched());
    }

    #[test]
    fn a_short_burst_of_loss_does_not_coarsen_the_measurement() {
        let mut backoff = backoff();
        fail(&mut backoff, FAILURES_BEFORE_STRETCHING);
        assert_eq!(
            backoff.interval(),
            BASE,
            "loss is what the user is watching; it must be sampled at full rate"
        );
    }

    #[test]
    fn sustained_failure_doubles_the_interval() {
        let mut backoff = backoff();
        fail(&mut backoff, FAILURES_BEFORE_STRETCHING + 1);
        assert_eq!(backoff.interval(), BASE * 2);
        backoff.record(ProbeOutcome::Timeout);
        assert_eq!(backoff.interval(), BASE * 4);
        assert!(backoff.is_stretched());
    }

    #[test]
    fn the_interval_never_exceeds_its_ceiling() {
        let mut backoff = backoff();
        fail(&mut backoff, 1_000);
        assert_eq!(backoff.interval(), MAX);
    }

    #[test]
    fn an_endpoint_silent_for_a_very_long_time_does_not_wrap_around() {
        // The failure mode this guards: an overflowing shift or multiply turning hours of
        // silence into a target that is due constantly. Forty failures is past the point
        // where doubling exceeds a 32-bit shift, which is where the overflow would happen.
        let mut backoff = Backoff::new(BASE, Duration::from_secs(3_600)).unwrap();
        fail(&mut backoff, 40);
        assert_eq!(backoff.interval(), Duration::from_secs(3_600));

        fail(&mut backoff, 200);
        assert_eq!(backoff.interval(), Duration::from_secs(3_600));
    }

    #[test]
    fn a_single_success_restores_full_rate() {
        let mut backoff = backoff();
        fail(&mut backoff, 20);
        assert!(backoff.is_stretched());

        backoff.record(success());
        assert_eq!(backoff.interval(), BASE);
        assert_eq!(backoff.consecutive_failures(), 0);
    }

    #[test]
    fn an_endpoint_losing_every_other_packet_is_never_stretched() {
        // The case the whole design turns on: partial loss is a measurement, not a reason to
        // stop measuring.
        let mut backoff = backoff();
        for _ in 0..100 {
            backoff.record(ProbeOutcome::Timeout);
            backoff.record(success());
        }
        assert_eq!(backoff.interval(), BASE);
    }

    #[test]
    fn every_kind_of_standing_non_answer_counts() {
        for outcome in [
            ProbeOutcome::Timeout,
            ProbeOutcome::Unreachable,
            ProbeOutcome::Blocked,
        ] {
            let mut backoff = backoff();
            for _ in 0..=FAILURES_BEFORE_STRETCHING {
                backoff.record(outcome);
            }
            assert!(backoff.is_stretched(), "{outcome:?}");
        }
    }

    #[test]
    fn resetting_gives_a_new_attempt_the_same_chance_as_the_first() {
        let mut backoff = backoff();
        fail(&mut backoff, 20);
        backoff.reset();
        assert_eq!(backoff.interval(), BASE);
        assert_eq!(backoff.consecutive_failures(), 0);
    }

    #[test]
    fn a_zero_base_interval_is_refused() {
        assert_eq!(
            Backoff::new(Duration::ZERO, MAX).unwrap_err(),
            Error::ZeroInterval
        );
    }

    #[test]
    fn a_ceiling_below_the_floor_is_raised_to_it() {
        let mut backoff = Backoff::new(Duration::from_secs(5), Duration::from_secs(1)).unwrap();
        fail(&mut backoff, 50);
        assert_eq!(
            backoff.interval(),
            Duration::from_secs(5),
            "a nonsensical ceiling must not make probing faster than asked"
        );
    }
}
