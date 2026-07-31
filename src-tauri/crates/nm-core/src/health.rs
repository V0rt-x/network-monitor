//! Turning a window of measurements into a verdict.
//!
//! Two levels, and the difference between them matters. [`HealthThresholds::health_of`]
//! judges *one* target from its own samples. [`GroupHealth::of`] judges a *group* — a
//! domestic baseline, a foreign baseline — and deliberately reports both a headline
//! verdict and the distribution behind it, because a group whose members disagree is the
//! interesting case, not an edge case: half the foreign list answering and half silent is
//! precisely what selective filtering looks like.
//!
//! Nothing here invents a number. A target with no delivery test is [`Health::Unknown`],
//! never "0 % loss"; a group with nothing measured yet is [`Health::Unknown`], never
//! green. Everything is a pure function of the statistics handed in, so a whole session's
//! worth of verdicts is testable without a network.

use crate::stats::WindowStats;

/// How something is doing, as far as its measurements can honestly say.
///
/// The same states describe one target and a whole group, so the UI needs one vocabulary
/// rather than two that have to be kept in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Health {
    /// Answering, within every threshold.
    Ok,
    /// Answering, but losing packets, slow or unstable.
    Degraded,
    /// Nothing gets through, or the destination explicitly refuses.
    Unreachable,
    /// Every probe was filtered, so nothing about the link was measured at all.
    ///
    /// Distinct from [`Unreachable`](Self::Unreachable) on purpose: a filtered probe is an
    /// absence of knowledge, not evidence that the endpoint is down.
    Blocked,
    /// Data is demonstrably crossing this endpoint, but no probe gets an answer.
    ///
    /// **The normal state of a game's match server**, not an edge case: nothing listens on
    /// a game port but the game, so an echo, a connection attempt and a hello are all
    /// refused or ignored while the game plays perfectly well over it. Reporting that as
    /// [`Unreachable`](Self::Unreachable) would say "your game server is down" about a
    /// server the user is currently playing on — the one lie this product must never tell.
    ///
    /// It is a separate state rather than a flavour of [`Blocked`](Self::Blocked) because
    /// `Blocked` promises that filtering has been *proven*, and that promise is what lets
    /// the UI state it as a fact. Silence is not proof.
    ///
    /// The evidence for it is passive and cannot be faked: bytes counted by the operating
    /// system against this endpoint. It says nothing about latency or loss, and the figures
    /// stay absent — knowing an endpoint is alive is not knowing how well it is doing.
    CarryingTraffic,
    /// Not measured enough yet to say anything.
    Unknown,
}

impl Health {
    /// Whether something came back.
    #[must_use]
    pub const fn is_answering(self) -> bool {
        matches!(self, Self::Ok | Self::Degraded)
    }

    /// Whether the measurements support any verdict at all.
    #[must_use]
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Whether this state says the endpoint is alive, however little else it says.
    #[must_use]
    pub const fn is_alive(self) -> bool {
        matches!(self, Self::Ok | Self::Degraded | Self::CarryingTraffic)
    }
}

/// Folds passive evidence into a verdict the probes reached on their own.
///
/// Probes are the only thing that can measure a path, but they are not the only thing that
/// can prove one exists. Where the operating system has counted bytes crossing an endpoint,
/// probe silence stops meaning "nothing is there" and starts meaning "nothing answers us" —
/// which for a game server is the expected state rather than a fault.
///
/// Deliberately narrow. It only ever *softens* a verdict that the endpoint is not there:
///
/// * [`Health::Unreachable`] becomes [`Health::CarryingTraffic`]. That covers a refusal as
///   well as silence, and on purpose: a TCP probe to a game port is normally answered with
///   "no service here", which is a fact about the port our probe chose, not about the path
///   the game is playing over. Data crossing the endpoint outranks both.
/// * [`Health::Unknown`] becomes [`Health::CarryingTraffic`] only when there is nothing
///   left to try. While a probe kind is still being tested, "not measured yet" is the
///   honest word, and it is about to become something better.
/// * [`Health::Blocked`] is left alone: proven filtering is a stronger statement than
///   liveness, and it is the one the user can act on.
/// * A verdict built on probes that *did* answer is never touched — a measured path says
///   more than the fact that bytes crossed it.
#[must_use]
pub const fn with_passive_evidence(probed: Health, carrying: bool, measurable: bool) -> Health {
    if !carrying {
        return probed;
    }
    match probed {
        Health::Unreachable => Health::CarryingTraffic,
        Health::Unknown if !measurable => Health::CarryingTraffic,
        other => other,
    }
}

/// Where the line between healthy, degraded and dead is drawn.
///
/// Values, not constants, so the settings page can move them and every test can pin them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HealthThresholds {
    /// How many delivery-testing probes a window needs before it is judged at all.
    ///
    /// One timeout is not an outage; below this the verdict stays [`Health::Unknown`]
    /// rather than flapping on the first lost packet.
    pub min_delivery_attempts: usize,
    /// Loss at or above this percentage is degradation.
    pub degraded_loss_pct: f64,
    /// Mean round-trip time at or above this many milliseconds is degradation.
    pub degraded_rtt_ms: f64,
    /// Jitter at or above this many milliseconds is degradation.
    pub degraded_jitter_ms: f64,
}

impl Default for HealthThresholds {
    /// Defaults chosen for the product's audience: a competitive game notices a couple of
    /// percent of loss and 30 ms of jitter long before it notices absolute latency, and
    /// 150 ms is roughly where an intercontinental path stops being playable.
    fn default() -> Self {
        Self {
            min_delivery_attempts: 2,
            degraded_loss_pct: 2.0,
            degraded_rtt_ms: 150.0,
            degraded_jitter_ms: 30.0,
        }
    }
}

impl HealthThresholds {
    /// Judges one target's window of samples.
    #[must_use]
    pub fn health_of(&self, stats: &WindowStats) -> Health {
        let outcomes = stats.outcomes;
        if outcomes.total() == 0 {
            return Health::Unknown;
        }

        if outcomes.delivery_attempts() == 0 {
            // Nothing tested whether packets get through. An explicit refusal is still
            // knowledge about the destination, so it outranks the filtered probes that
            // told us nothing.
            return if outcomes.unreachable > 0 {
                Health::Unreachable
            } else {
                Health::Blocked
            };
        }

        if outcomes.delivery_attempts() < self.min_delivery_attempts {
            return Health::Unknown;
        }

        // Present whenever there was a delivery attempt, which the check above guarantees.
        let loss_pct = stats.loss_pct.unwrap_or(0.0);
        if loss_pct >= 100.0 {
            return Health::Unreachable;
        }
        if loss_pct >= self.degraded_loss_pct {
            return Health::Degraded;
        }

        if let Some(rtt) = stats.rtt {
            if rtt.mean_ms >= self.degraded_rtt_ms {
                return Health::Degraded;
            }
            if rtt.jitter_ms.is_some_and(|j| j >= self.degraded_jitter_ms) {
                return Health::Degraded;
            }
        }

        Health::Ok
    }
}

/// How many members of a group are in each state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HealthCounts {
    /// Members answering within every threshold.
    pub ok: usize,
    /// Members answering but degraded.
    pub degraded: usize,
    /// Members nothing gets through to.
    pub unreachable: usize,
    /// Members whose probes are all filtered.
    pub blocked: usize,
    /// Members proven alive by their traffic, with no probe answering.
    pub carrying_traffic: usize,
    /// Members not measured enough to judge.
    pub unknown: usize,
}

impl HealthCounts {
    /// Adds one member's verdict.
    pub fn record(&mut self, health: Health) {
        match health {
            Health::Ok => self.ok += 1,
            Health::Degraded => self.degraded += 1,
            Health::Unreachable => self.unreachable += 1,
            Health::Blocked => self.blocked += 1,
            Health::CarryingTraffic => self.carrying_traffic += 1,
            // A state this build has no counter for must not vanish from a distribution
            // that claims to add up.
            _ => self.unknown += 1,
        }
    }

    /// How many members were counted.
    #[must_use]
    pub const fn total(self) -> usize {
        self.ok
            + self.degraded
            + self.unreachable
            + self.blocked
            + self.carrying_traffic
            + self.unknown
    }

    /// Members that came back.
    #[must_use]
    pub const fn answering(self) -> usize {
        self.ok + self.degraded
    }

    /// Members whose measurements support a verdict.
    #[must_use]
    pub const fn known(self) -> usize {
        self.total() - self.unknown
    }
}

/// What a group of targets says, taken together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroupHealth {
    /// The headline verdict.
    pub verdict: Health,
    /// The distribution behind it, which the UI must show rather than hide.
    pub counts: HealthCounts,
    /// Median of the answering members' mean round-trip times, in milliseconds.
    pub rtt_ms: Option<f64>,
    /// Median of the answering members' jitter, in milliseconds.
    pub jitter_ms: Option<f64>,
    /// Loss across the group: every timeout over every delivery attempt.
    ///
    /// Weighted by probes rather than averaged over members, so a target probed twice as
    /// often does not count double per sample — and [`None`] when the group tested
    /// delivery nowhere.
    pub loss_pct: Option<f64>,
}

impl GroupHealth {
    /// Judges a group from its members' windows.
    #[must_use]
    pub fn of<'a, I>(members: I, thresholds: &HealthThresholds) -> Self
    where
        I: IntoIterator<Item = &'a WindowStats>,
    {
        let mut counts = HealthCounts::default();
        let mut timeouts = 0_usize;
        let mut attempts = 0_usize;
        let mut rtts: Vec<f64> = Vec::new();
        let mut jitters: Vec<f64> = Vec::new();

        for stats in members {
            let health = thresholds.health_of(stats);
            counts.record(health);
            timeouts += stats.outcomes.timeout;
            attempts += stats.outcomes.delivery_attempts();
            if let Some(rtt) = stats.rtt {
                rtts.push(rtt.mean_ms);
                if let Some(jitter) = rtt.jitter_ms {
                    jitters.push(jitter);
                }
            }
        }

        Self {
            verdict: verdict_for(counts),
            counts,
            rtt_ms: median(&mut rtts),
            jitter_ms: median(&mut jitters),
            loss_pct: ratio_pct(timeouts, attempts),
        }
    }
}

/// The headline verdict a distribution implies.
fn verdict_for(counts: HealthCounts) -> Health {
    if counts.known() == 0 {
        // Either an empty group or one nothing has been measured on yet. Both are
        // "we don't know", and neither may be shown as healthy.
        return Health::Unknown;
    }
    if counts.answering() > 0 {
        // Anything less than a clean sweep is degradation, and the counts say how much. A
        // member proven alive by its traffic is not a clean sweep — nothing measured its
        // path — but it is not a failure either, so it lands here with the rest.
        if counts.degraded == 0
            && counts.unreachable == 0
            && counts.blocked == 0
            && counts.carrying_traffic == 0
        {
            return Health::Ok;
        }
        return Health::Degraded;
    }
    // Nothing in the group answers a probe. Being alive still outranks every explanation of
    // silence: the group is reaching *something*.
    if counts.carrying_traffic > 0 {
        return Health::CarryingTraffic;
    }
    // An explicit refusal anywhere is a stronger statement than filtering, which measured
    // nothing at all.
    if counts.unreachable > 0 {
        Health::Unreachable
    } else {
        Health::Blocked
    }
}

/// `part` as a percentage of `whole`, or [`None`] when nothing was tested.
fn ratio_pct(part: usize, whole: usize) -> Option<f64> {
    if whole == 0 {
        return None;
    }
    // Counts are bounded by the ring buffers feeding them — thousands at most — so they
    // convert to f64 exactly.
    #[allow(clippy::cast_precision_loss)]
    Some(part as f64 / whole as f64 * 100.0)
}

/// Middle value of `values`, sorting them in place; [`None`] when empty.
///
/// The median rather than the mean: one member behind a satellite link should not drag the
/// group's headline number away from what every other member is seeing.
fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values.get(middle).copied()
    } else {
        // Both indices exist: the length is even and non-zero.
        match (values.get(middle - 1), values.get(middle)) {
            (Some(low), Some(high)) => Some((low + high) / 2.0),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::sample::{ProbeOutcome, ProbeSample, Rtt};

    /// Builds a window from a list of outcomes, one second apart.
    fn window(outcomes: &[ProbeOutcome]) -> WindowStats {
        let start = Instant::now();
        let samples: Vec<ProbeSample> = outcomes
            .iter()
            .enumerate()
            .map(|(index, outcome)| {
                let offset = Duration::from_secs(u64::try_from(index).unwrap_or(0));
                ProbeSample::new(start + offset, *outcome)
            })
            .collect();
        WindowStats::of(&samples)
    }

    fn ok(millis: u32) -> ProbeOutcome {
        ProbeOutcome::Success(Rtt::from_micros(millis * 1_000))
    }

    fn health(outcomes: &[ProbeOutcome]) -> Health {
        HealthThresholds::default().health_of(&window(outcomes))
    }

    #[test]
    fn nothing_measured_is_unknown_rather_than_healthy() {
        assert_eq!(health(&[]), Health::Unknown);
    }

    #[test]
    fn a_single_lost_packet_is_not_yet_a_verdict() {
        // The state must not flap to "unreachable" on the first timeout of a session.
        assert_eq!(health(&[ProbeOutcome::Timeout]), Health::Unknown);
        assert_eq!(health(&[ok(10)]), Health::Unknown);
    }

    #[test]
    fn steady_fast_replies_are_healthy() {
        assert_eq!(health(&[ok(10), ok(11), ok(10), ok(12)]), Health::Ok);
    }

    #[test]
    fn total_silence_is_unreachable() {
        assert_eq!(
            health(&[ProbeOutcome::Timeout, ProbeOutcome::Timeout]),
            Health::Unreachable
        );
    }

    #[test]
    fn partial_loss_is_degradation_not_an_outage() {
        // Three of four delivered: 25 % loss. The user is still connected, badly.
        assert_eq!(
            health(&[ok(10), ProbeOutcome::Timeout, ok(11), ok(10)]),
            Health::Degraded
        );
    }

    #[test]
    fn loss_below_the_threshold_stays_healthy() {
        let mut outcomes = vec![ok(10); 99];
        outcomes.push(ProbeOutcome::Timeout);
        // 1 % loss, under the 2 % line.
        assert_eq!(health(&outcomes), Health::Ok);
    }

    #[test]
    fn a_slow_path_is_degraded_even_with_no_loss() {
        assert_eq!(health(&[ok(400), ok(410), ok(405)]), Health::Degraded);
    }

    #[test]
    fn an_unstable_path_is_degraded_even_when_fast_on_average() {
        // Alternating 5 ms and 200 ms: the mean is under the latency line, the jitter is
        // not, and jitter is what a game actually feels.
        let outcomes = [
            ok(5),
            ok(200),
            ok(5),
            ok(200),
            ok(5),
            ok(200),
            ok(5),
            ok(200),
        ];
        assert_eq!(health(&outcomes), Health::Degraded);
    }

    #[test]
    fn filtered_probes_are_blocked_rather_than_lost() {
        // The failure this product exists to avoid: four filtered probes must not read as
        // 100 % packet loss on a link that may be perfectly healthy.
        assert_eq!(health(&[ProbeOutcome::Blocked; 4]), Health::Blocked);
    }

    #[test]
    fn an_explicit_refusal_outranks_filtering() {
        assert_eq!(
            health(&[ProbeOutcome::Blocked, ProbeOutcome::Unreachable]),
            Health::Unreachable,
            "a definitive answer is knowledge; a filtered probe is the absence of it"
        );
    }

    #[test]
    fn a_refused_destination_is_unreachable() {
        assert_eq!(health(&[ProbeOutcome::Unreachable; 3]), Health::Unreachable);
    }

    #[test]
    fn thresholds_are_data_and_move_the_verdict() {
        let strict = HealthThresholds {
            degraded_rtt_ms: 5.0,
            ..HealthThresholds::default()
        };
        let stats = window(&[ok(10), ok(10), ok(10)]);
        assert_eq!(HealthThresholds::default().health_of(&stats), Health::Ok);
        assert_eq!(strict.health_of(&stats), Health::Degraded);
    }

    fn group(members: &[WindowStats]) -> GroupHealth {
        GroupHealth::of(members.iter(), &HealthThresholds::default())
    }

    #[test]
    fn an_empty_group_knows_nothing() {
        let health = group(&[]);
        assert_eq!(health.verdict, Health::Unknown);
        assert_eq!(health.counts, HealthCounts::default());
        assert_eq!(health.rtt_ms, None);
        assert_eq!(health.jitter_ms, None);
        assert_eq!(health.loss_pct, None);
    }

    #[test]
    fn a_group_nothing_has_been_measured_on_is_not_green() {
        assert_eq!(group(&[window(&[]), window(&[])]).verdict, Health::Unknown);
    }

    #[test]
    fn a_group_is_healthy_only_when_every_judged_member_is() {
        let health = group(&[
            window(&[ok(10), ok(11), ok(10)]),
            window(&[ok(20), ok(21), ok(20)]),
        ]);
        assert_eq!(health.verdict, Health::Ok);
        assert_eq!(health.counts.ok, 2);
    }

    #[test]
    fn a_member_still_warming_up_does_not_hold_the_group_back() {
        let health = group(&[window(&[ok(10), ok(11), ok(10)]), window(&[ok(20)])]);
        assert_eq!(health.verdict, Health::Ok);
        assert_eq!(health.counts.unknown, 1);
    }

    #[test]
    fn one_failing_member_degrades_the_group_without_hiding_the_rest() {
        // The distribution is the point: "3 clean, 1 unreachable" is actionable, one red
        // dot is not.
        let health = group(&[
            window(&[ok(10), ok(11), ok(10)]),
            window(&[ok(12), ok(11), ok(12)]),
            window(&[ok(10), ok(10), ok(11)]),
            window(&[ProbeOutcome::Timeout; 4]),
        ]);
        assert_eq!(health.verdict, Health::Degraded);
        assert_eq!(health.counts.ok, 3);
        assert_eq!(health.counts.unreachable, 1);
    }

    #[test]
    fn a_group_where_nothing_answers_is_unreachable() {
        let health = group(&[
            window(&[ProbeOutcome::Timeout; 3]),
            window(&[ProbeOutcome::Unreachable; 2]),
        ]);
        assert_eq!(health.verdict, Health::Unreachable);
        assert_eq!(health.counts.answering(), 0);
    }

    #[test]
    fn a_group_whose_probes_are_all_filtered_is_blocked_not_unreachable() {
        let health = group(&[
            window(&[ProbeOutcome::Blocked; 3]),
            window(&[ProbeOutcome::Blocked; 3]),
        ]);
        assert_eq!(health.verdict, Health::Blocked);
        assert_eq!(health.loss_pct, None, "filtered probes are not loss");
    }

    #[test]
    fn group_loss_is_weighted_by_probes_rather_than_by_member() {
        // One member probed 100 times with 1 loss, one probed twice with 1 loss. Averaging
        // the percentages would report 25.5 %; the truth is 2 of 102.
        let mut many = vec![ok(10); 99];
        many.push(ProbeOutcome::Timeout);
        let health = group(&[window(&many), window(&[ok(10), ProbeOutcome::Timeout])]);
        let loss = health.loss_pct.expect("delivery was tested");
        assert!((loss - 2.0 / 102.0 * 100.0).abs() < 1e-9, "{loss}");
    }

    #[test]
    fn the_group_round_trip_time_is_a_median_not_an_average() {
        // One member on a satellite link must not drag the headline figure with it.
        let health = group(&[
            window(&[ok(10), ok(10), ok(10)]),
            window(&[ok(12), ok(12), ok(12)]),
            window(&[ok(600), ok(600), ok(600)]),
        ]);
        assert_eq!(health.rtt_ms, Some(12.0));
    }

    #[test]
    fn an_even_sized_group_takes_the_midpoint() {
        let health = group(&[
            window(&[ok(10), ok(10), ok(10)]),
            window(&[ok(20), ok(20), ok(20)]),
        ]);
        assert_eq!(health.rtt_ms, Some(15.0));
    }

    #[test]
    fn counts_add_up_to_the_membership() {
        let health = group(&[
            window(&[ok(10), ok(10), ok(10)]),
            window(&[ok(10), ProbeOutcome::Timeout, ok(10), ok(10)]),
            window(&[ProbeOutcome::Timeout; 3]),
            window(&[ProbeOutcome::Blocked; 3]),
            window(&[]),
        ]);
        assert_eq!(health.counts.total(), 5);
        assert_eq!(health.counts.known(), 4);
        assert_eq!(health.counts.answering(), 2);
        assert_eq!(health.counts.ok, 1);
        assert_eq!(health.counts.degraded, 1);
        assert_eq!(health.counts.unreachable, 1);
        assert_eq!(health.counts.blocked, 1);
        assert_eq!(health.counts.unknown, 1);
    }

    #[test]
    fn states_report_what_they_mean() {
        assert!(Health::Ok.is_answering());
        assert!(Health::Degraded.is_answering());
        assert!(!Health::Unreachable.is_answering());
        assert!(!Health::Blocked.is_answering());
        assert!(!Health::Unknown.is_answering());

        assert!(Health::Blocked.is_known());
        assert!(!Health::Unknown.is_known());

        assert!(Health::Ok.is_alive());
        assert!(Health::Degraded.is_alive());
        assert!(Health::CarryingTraffic.is_alive());
        assert!(!Health::Unreachable.is_alive());
        assert!(!Health::Blocked.is_alive());
        assert!(!Health::Unknown.is_alive());
    }

    #[test]
    fn traffic_across_a_silent_endpoint_means_it_is_alive_not_unreachable() {
        // The headline case: a game's match server answers no probe of any kind, because
        // nothing listens on a game port but the game. Calling it unreachable would say
        // "your game server is down" about a server the user is playing on.
        assert_eq!(
            with_passive_evidence(Health::Unreachable, true, true),
            Health::CarryingTraffic
        );
    }

    #[test]
    fn a_refusal_is_softened_too_when_traffic_is_crossing() {
        // A TCP probe to a game port is normally refused outright. That is a fact about the
        // port our probe chose, not about the path the game is playing over.
        let refused = health(&[ProbeOutcome::Unreachable, ProbeOutcome::Unreachable]);
        assert_eq!(refused, Health::Unreachable);
        assert_eq!(
            with_passive_evidence(refused, true, true),
            Health::CarryingTraffic
        );
    }

    #[test]
    fn silence_without_traffic_stays_unreachable() {
        // Nothing counted any bytes — either the platform cannot, or none crossed. Absent
        // evidence must not become evidence of life.
        assert_eq!(
            with_passive_evidence(Health::Unreachable, false, true),
            Health::Unreachable
        );
    }

    #[test]
    fn traffic_never_makes_a_measured_verdict_look_better() {
        // Probes that answered say more than the fact that bytes crossed, so a measured
        // verdict is never touched — least of all a degraded one, which is the finding the
        // user came for.
        for measured in [Health::Ok, Health::Degraded] {
            assert_eq!(with_passive_evidence(measured, true, true), measured);
        }
    }

    #[test]
    fn proven_filtering_outranks_liveness() {
        // `Blocked` promises filtering was *proven*, which is actionable — "try a VPN".
        // Liveness is a weaker statement and must not overwrite it.
        assert_eq!(
            with_passive_evidence(Health::Blocked, true, true),
            Health::Blocked
        );
    }

    #[test]
    fn an_endpoint_still_being_tested_stays_unknown() {
        // While a probe kind is left to try, "not measured yet" is honest and is about to
        // become something better. Only when nothing is left does traffic become the answer.
        assert_eq!(
            with_passive_evidence(Health::Unknown, true, true),
            Health::Unknown
        );
        assert_eq!(
            with_passive_evidence(Health::Unknown, true, false),
            Health::CarryingTraffic
        );
    }

    #[test]
    fn a_group_reaching_something_is_not_reported_as_unreachable() {
        let mut counts = HealthCounts::default();
        counts.record(Health::CarryingTraffic);
        counts.record(Health::Unreachable);

        assert_eq!(verdict_for(counts), Health::CarryingTraffic);
        assert_eq!(counts.total(), 2);
    }

    #[test]
    fn a_group_is_not_clean_while_a_member_is_unmeasured() {
        // The member is alive, but nothing measured its path, so the group cannot claim a
        // clean sweep — the distribution says which member it is.
        let mut counts = HealthCounts::default();
        counts.record(Health::Ok);
        counts.record(Health::CarryingTraffic);

        assert_eq!(verdict_for(counts), Health::Degraded);
    }
}
