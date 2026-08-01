//! Judging a status-page service from the checks it has just answered.
//!
//! The status page asks a different question of the same samples than the dashboard does.
//! [`crate::health::HealthThresholds`] asks "what has this window been like", which is the
//! right question for a baseline whose loss rate is the finding. A service card asks "is it
//! up **right now**", and a window rule answers that badly in both directions: a service
//! that died a minute ago still reads mostly green, and a service that has just come back
//! still reads mostly red, because at a check every forty-odd seconds a ten-minute window is
//! mostly history.
//!
//! So the rule here reads the most recent checks in order, newest first, and reacts within
//! one check interval. Everything else about the vocabulary is shared: the states are
//! [`Health`]'s, and a service with several endpoints rolls up through
//! [`GroupHealth::of_judged`] exactly as a baseline group rolls up its members.
//!
//! # What it refuses to say
//!
//! One failed check is **not** an outage, and the page must not claim one. The same
//! reasoning as `min_delivery_attempts` on the dashboard: a single lost packet on a link
//! that is otherwise fine is the most common event on the internet, and a status page that
//! flashed "Steam is down" every time one arrived would be worth nothing on the day Steam
//! actually was. A failing check moves a service off `Ok` at once — the user sees it, and
//! the timeline shows the individual check that failed — but the word *unreachable* waits
//! for [`StatusThresholds::failures_before_unreachable`] in a row.

use crate::health::{GroupHealth, Health};
use crate::sample::{ProbeOutcome, ProbeSample};
use crate::stats::WindowStats;

/// Where the lines fall for a service check.
///
/// Values rather than constants, so every rule below is pinned by a test that states its
/// own thresholds instead of inheriting them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatusThresholds {
    /// A check answering at or above this many milliseconds reads as slow.
    ///
    /// Higher than the dashboard's degradation line on purpose: these are front doors
    /// reached over TLS on the other side of the world, and a service is not in trouble
    /// because it is far away.
    pub slow_rtt_ms: f64,
    /// How many consecutive failed checks it takes before a service is called unreachable.
    pub failures_before_unreachable: usize,
    /// How many recent checks the verdict looks at at all.
    ///
    /// The span the card describes. Beyond it the timeline still shows what happened, but
    /// no longer colours the headline: a service that failed twenty minutes ago and has
    /// answered every check since is up.
    pub checks_considered: usize,
}

impl Default for StatusThresholds {
    fn default() -> Self {
        Self {
            slow_rtt_ms: 400.0,
            failures_before_unreachable: 2,
            checks_considered: 5,
        }
    }
}

/// What one check produced, as the timeline draws it.
///
/// A fact about a single check, never a windowed verdict — which is precisely what makes
/// the timeline worth showing beside the card: it is the evidence the headline was reached
/// from, and it distinguishes the four ways a check can fail to produce a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CheckMark {
    /// It answered within the slow line.
    Answered,
    /// It answered, but slowly.
    Slow,
    /// Nothing came back in time.
    Lost,
    /// The destination answered that it cannot be reached.
    Refused,
    /// The probe kind was filtered, so this check measured nothing at all.
    Filtered,
}

impl StatusThresholds {
    /// How one check reads.
    #[must_use]
    pub fn mark(&self, outcome: ProbeOutcome) -> CheckMark {
        match outcome {
            ProbeOutcome::Success(rtt) if rtt.as_millis_f64() >= self.slow_rtt_ms => {
                CheckMark::Slow
            }
            ProbeOutcome::Success(_) => CheckMark::Answered,
            ProbeOutcome::Timeout => CheckMark::Lost,
            ProbeOutcome::Unreachable => CheckMark::Refused,
            ProbeOutcome::Blocked => CheckMark::Filtered,
        }
    }

    /// Judges one endpoint of a service from its checks, oldest first.
    ///
    /// `checks` may hold more than [`Self::checks_considered`]; only the newest of them are
    /// read, so a caller can hand over its whole history without slicing it first.
    ///
    /// Reading, in order:
    ///
    /// * Nothing checked yet is [`Health::Unknown`]. Never green.
    /// * The newest check answered → the service is up. [`Health::Degraded`] if it was slow,
    ///   or if any other considered check failed, because "reachable, but not every check
    ///   gets through" is a real state and hiding it behind a clean tick would be the
    ///   fake-good reading this product exists not to produce.
    /// * The newest check failed → count the unbroken run of failures behind it.
    ///   Below the threshold that is [`Health::Degraded`]: it answered a moment ago.
    ///   At or above it, the run says what kind of failure it is — [`Health::Blocked`] when
    ///   every check in the run was filtered, since filtering measured nothing and is not
    ///   evidence the service is down, and [`Health::Unreachable`] otherwise.
    #[must_use]
    pub fn health_of(&self, checks: &[ProbeSample]) -> Health {
        let considered = &checks[checks.len().saturating_sub(self.checks_considered)..];
        let Some(newest) = considered.last() else {
            return Health::Unknown;
        };

        if let Some(rtt) = newest.outcome.rtt() {
            if rtt.as_millis_f64() >= self.slow_rtt_ms {
                return Health::Degraded;
            }
            let steady = considered
                .iter()
                .all(|check| matches!(check.outcome, ProbeOutcome::Success(_)));
            return if steady { Health::Ok } else { Health::Degraded };
        }

        let run: Vec<ProbeOutcome> = considered
            .iter()
            .rev()
            .map(|check| check.outcome)
            .take_while(|outcome| outcome.rtt().is_none())
            .collect();
        if run.len() < self.failures_before_unreachable {
            return Health::Degraded;
        }
        if run
            .iter()
            .all(|outcome| matches!(outcome, ProbeOutcome::Blocked))
        {
            return Health::Blocked;
        }
        Health::Unreachable
    }

    /// Rolls a service's endpoints up into one verdict and the distribution behind it.
    ///
    /// Each pair is one endpoint's checks and the statistics of the window shown beside the
    /// card. The verdict comes from the checks; the medians and the loss figure come from
    /// the statistics, which is the same division of labour as everywhere else.
    #[must_use]
    pub fn service_health<'a, I>(&self, endpoints: I) -> GroupHealth
    where
        I: IntoIterator<Item = (&'a [ProbeSample], &'a WindowStats)>,
    {
        GroupHealth::of_judged(
            endpoints
                .into_iter()
                .map(|(checks, stats)| (self.health_of(checks), stats)),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::sample::Rtt;

    fn thresholds() -> StatusThresholds {
        StatusThresholds {
            slow_rtt_ms: 400.0,
            failures_before_unreachable: 2,
            checks_considered: 5,
        }
    }

    /// Builds a run of checks, oldest first, forty-five seconds apart.
    fn checks(outcomes: &[ProbeOutcome]) -> Vec<ProbeSample> {
        let start = Instant::now();
        outcomes
            .iter()
            .enumerate()
            .map(|(index, outcome)| {
                let step = u64::try_from(index).unwrap_or(0);
                ProbeSample::new(start + Duration::from_secs(45 * step), *outcome)
            })
            .collect()
    }

    fn ok(ms: u32) -> ProbeOutcome {
        ProbeOutcome::Success(Rtt::from_micros(ms * 1_000))
    }

    #[test]
    fn nothing_checked_yet_is_unknown_rather_than_reachable() {
        assert_eq!(thresholds().health_of(&[]), Health::Unknown);
    }

    #[test]
    fn a_run_of_clean_checks_is_ok() {
        let checks = checks(&[ok(30), ok(35), ok(28)]);
        assert_eq!(thresholds().health_of(&checks), Health::Ok);
    }

    #[test]
    fn one_slow_answer_is_degraded_not_unreachable() {
        let checks = checks(&[ok(30), ok(700)]);
        assert_eq!(thresholds().health_of(&checks), Health::Degraded);
    }

    #[test]
    fn a_service_answering_after_a_lost_check_is_degraded_not_ok() {
        // "Reachable, but not every check gets through" is the finding, and rounding it up
        // to a clean tick is exactly the fake-good reading the product must not produce.
        let checks = checks(&[ok(30), ProbeOutcome::Timeout, ok(31)]);
        assert_eq!(thresholds().health_of(&checks), Health::Degraded);
    }

    #[test]
    fn one_failed_check_is_not_an_outage() {
        let checks = checks(&[ok(30), ok(30), ProbeOutcome::Timeout]);
        assert_eq!(
            thresholds().health_of(&checks),
            Health::Degraded,
            "a single lost packet must not be reported as a service being down"
        );
    }

    #[test]
    fn the_second_consecutive_failure_makes_it_unreachable() {
        let checks = checks(&[ok(30), ProbeOutcome::Timeout, ProbeOutcome::Timeout]);
        assert_eq!(thresholds().health_of(&checks), Health::Unreachable);
    }

    #[test]
    fn a_refusal_counts_towards_the_run_as_a_failure_does() {
        let checks = checks(&[ok(30), ProbeOutcome::Unreachable, ProbeOutcome::Timeout]);
        assert_eq!(thresholds().health_of(&checks), Health::Unreachable);
    }

    #[test]
    fn a_run_of_filtered_checks_is_blocked_rather_than_unreachable() {
        // Filtering measured nothing about the service. Calling it unreachable would claim
        // knowledge the checks never produced.
        let checks = checks(&[ok(30), ProbeOutcome::Blocked, ProbeOutcome::Blocked]);
        assert_eq!(thresholds().health_of(&checks), Health::Blocked);
    }

    #[test]
    fn a_mixed_run_is_unreachable_because_something_was_actually_measured() {
        let checks = checks(&[ProbeOutcome::Blocked, ProbeOutcome::Timeout]);
        assert_eq!(thresholds().health_of(&checks), Health::Unreachable);
    }

    #[test]
    fn recovery_shows_within_one_check() {
        // The whole reason this rule exists rather than a window: the check that answered is
        // the newest, so the card stops claiming an outage immediately.
        let checks = checks(&[
            ProbeOutcome::Timeout,
            ProbeOutcome::Timeout,
            ProbeOutcome::Timeout,
            ok(30),
        ]);
        assert_eq!(thresholds().health_of(&checks), Health::Degraded);
    }

    #[test]
    fn failures_older_than_the_considered_span_stop_colouring_the_verdict() {
        let checks = checks(&[
            ProbeOutcome::Timeout,
            ProbeOutcome::Timeout,
            ok(30),
            ok(30),
            ok(30),
            ok(30),
            ok(30),
        ]);
        assert_eq!(
            thresholds().health_of(&checks),
            Health::Ok,
            "a service that has answered every check for five rounds is up"
        );
    }

    #[test]
    fn a_history_shorter_than_the_considered_span_is_read_whole() {
        let checks = checks(&[ProbeOutcome::Timeout, ProbeOutcome::Timeout]);
        assert_eq!(thresholds().health_of(&checks), Health::Unreachable);
    }

    #[test]
    fn checks_are_marked_by_what_they_produced() {
        let thresholds = thresholds();
        assert_eq!(thresholds.mark(ok(30)), CheckMark::Answered);
        assert_eq!(thresholds.mark(ok(700)), CheckMark::Slow);
        assert_eq!(thresholds.mark(ProbeOutcome::Timeout), CheckMark::Lost);
        assert_eq!(
            thresholds.mark(ProbeOutcome::Unreachable),
            CheckMark::Refused
        );
        assert_eq!(thresholds.mark(ProbeOutcome::Blocked), CheckMark::Filtered);
    }

    #[test]
    fn the_slow_line_is_inclusive_at_both_ends() {
        let thresholds = thresholds();
        assert_eq!(thresholds.mark(ok(400)), CheckMark::Slow);
        assert_eq!(thresholds.mark(ok(399)), CheckMark::Answered);
    }

    #[test]
    fn a_service_is_ok_only_when_every_endpoint_is() {
        let clean = checks(&[ok(30), ok(30)]);
        let broken = checks(&[ProbeOutcome::Timeout, ProbeOutcome::Timeout]);
        let clean_stats = WindowStats::of(&clean);
        let broken_stats = WindowStats::of(&broken);

        let health = thresholds().service_health([
            (clean.as_slice(), &clean_stats),
            (broken.as_slice(), &broken_stats),
        ]);

        assert_eq!(health.verdict, Health::Degraded);
        assert_eq!(health.counts.ok, 1);
        assert_eq!(health.counts.unreachable, 1);
    }

    #[test]
    fn a_service_whose_every_endpoint_is_gone_is_unreachable() {
        let broken = checks(&[ProbeOutcome::Timeout, ProbeOutcome::Timeout]);
        let stats = WindowStats::of(&broken);

        let health =
            thresholds().service_health([(broken.as_slice(), &stats), (broken.as_slice(), &stats)]);

        assert_eq!(health.verdict, Health::Unreachable);
        assert_eq!(health.counts.unreachable, 2);
    }

    #[test]
    fn a_service_with_no_endpoints_is_unknown() {
        let health = thresholds().service_health([]);
        assert_eq!(health.verdict, Health::Unknown);
        assert_eq!(health.counts.total(), 0);
    }
}
