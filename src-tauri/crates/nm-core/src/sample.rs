//! What a single probe produced.

use std::time::{Duration, Instant};

/// A round-trip time, stored as whole microseconds.
///
/// Integers rather than floats: storage stays exact and totally ordered, so sorting for
/// percentiles cannot be derailed by a `NaN`. The range reaches ~71.6 minutes, far
/// beyond any timeout the product will ever use, and each sample costs four bytes —
/// which matters when 5 apps × 16 endpoints each keep a window of history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rtt(u32);

impl Rtt {
    /// The largest representable round-trip time (~71.6 minutes).
    pub const MAX: Self = Self(u32::MAX);

    /// Builds a round-trip time from whole microseconds.
    #[must_use]
    pub const fn from_micros(micros: u32) -> Self {
        Self(micros)
    }

    /// Builds a round-trip time from a measured duration, saturating at [`Rtt::MAX`].
    ///
    /// Saturating rather than wrapping: an absurd measurement must not come out looking
    /// like a fast one.
    #[must_use]
    pub fn from_duration(measured: Duration) -> Self {
        Self(u32::try_from(measured.as_micros()).unwrap_or(u32::MAX))
    }

    /// The round-trip time in whole microseconds.
    #[must_use]
    pub const fn as_micros(self) -> u32 {
        self.0
    }

    /// The round-trip time in milliseconds.
    ///
    /// Exact: every [`u32`] converts to [`f64`] without loss.
    #[must_use]
    pub fn as_millis_f64(self) -> f64 {
        f64::from(self.0) / 1_000.0
    }
}

/// What came back from one probe.
///
/// The distinction between these variants is the difference between an honest verdict
/// and a misleading one, so nothing here collapses into a generic "failed":
///
/// * [`Timeout`](ProbeOutcome::Timeout) is packet loss — nothing answered in time.
/// * [`Unreachable`](ProbeOutcome::Unreachable) is a definitive negative answer (an ICMP
///   error, a refused connection): the path works, the destination does not.
/// * [`Blocked`](ProbeOutcome::Blocked) means this probe kind is filtered, so the sample
///   carries no information about the link at all. Counting it as loss would invent a
///   100 % loss figure for a perfectly healthy connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProbeOutcome {
    /// A reply arrived, taking this long.
    Success(Rtt),
    /// Nothing replied within the probe's deadline.
    Timeout,
    /// The destination answered that it cannot be reached.
    Unreachable,
    /// This probe kind is filtered on the path; the sample measures nothing.
    Blocked,
}

impl ProbeOutcome {
    /// The measured round-trip time, if this probe measured one.
    #[must_use]
    pub const fn rtt(self) -> Option<Rtt> {
        match self {
            Self::Success(rtt) => Some(rtt),
            Self::Timeout | Self::Unreachable | Self::Blocked => None,
        }
    }

    /// Whether this outcome is evidence about packet delivery.
    ///
    /// Only successes and timeouts are: they are the two ways a delivery test can end.
    /// Unreachable and blocked outcomes answer a different question and must stay out of
    /// the loss ratio's denominator.
    #[must_use]
    pub const fn tests_delivery(self) -> bool {
        matches!(self, Self::Success(_) | Self::Timeout)
    }
}

/// One probe result, stamped on the monotonic clock.
///
/// `at` comes from [`Instant`] so a wall-clock adjustment mid-session cannot reorder or
/// stretch a measurement window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeSample {
    /// When the probe completed.
    pub at: Instant,
    /// What it produced.
    pub outcome: ProbeOutcome,
}

impl ProbeSample {
    /// Builds a sample.
    #[must_use]
    pub const fn new(at: Instant, outcome: ProbeOutcome) -> Self {
        Self { at, outcome }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_durations_to_microseconds() {
        assert_eq!(Rtt::from_duration(Duration::ZERO).as_micros(), 0);
        assert_eq!(
            Rtt::from_duration(Duration::from_millis(12)).as_micros(),
            12_000
        );
        // Sub-microsecond remainders truncate rather than round up to a whole unit.
        assert_eq!(
            Rtt::from_duration(Duration::from_nanos(1_999)).as_micros(),
            1
        );
    }

    #[test]
    fn saturates_absurd_durations_instead_of_wrapping() {
        assert_eq!(Rtt::from_duration(Duration::MAX), Rtt::MAX);
        assert_eq!(Rtt::from_duration(Duration::from_secs(4_295)), Rtt::MAX);
    }

    #[test]
    fn converts_to_milliseconds_exactly() {
        assert!((Rtt::from_micros(12_500).as_millis_f64() - 12.5).abs() < f64::EPSILON);
        assert!((Rtt::from_micros(0).as_millis_f64()).abs() < f64::EPSILON);
    }

    #[test]
    fn orders_by_duration() {
        let mut rtts = [
            Rtt::from_micros(30),
            Rtt::from_micros(10),
            Rtt::from_micros(20),
        ];
        rtts.sort_unstable();
        assert_eq!(
            rtts,
            [
                Rtt::from_micros(10),
                Rtt::from_micros(20),
                Rtt::from_micros(30)
            ]
        );
    }

    #[test]
    fn only_successes_carry_an_rtt() {
        let rtt = Rtt::from_micros(1_000);
        assert_eq!(ProbeOutcome::Success(rtt).rtt(), Some(rtt));
        assert_eq!(ProbeOutcome::Timeout.rtt(), None);
        assert_eq!(ProbeOutcome::Unreachable.rtt(), None);
        assert_eq!(ProbeOutcome::Blocked.rtt(), None);
    }

    #[test]
    fn only_successes_and_timeouts_test_delivery() {
        // This is the rule that keeps a filtered probe from being reported as 100 % loss.
        assert!(ProbeOutcome::Success(Rtt::from_micros(1)).tests_delivery());
        assert!(ProbeOutcome::Timeout.tests_delivery());
        assert!(!ProbeOutcome::Unreachable.tests_delivery());
        assert!(!ProbeOutcome::Blocked.tests_delivery());
    }
}
