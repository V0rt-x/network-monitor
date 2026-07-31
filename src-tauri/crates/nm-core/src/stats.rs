//! Sliding-window statistics over probe samples.
//!
//! Everything here is a pure function of the samples handed in. Absent values are
//! [`None`], never a zero standing in for "we don't know" — a fabricated `0 ms` or
//! `0 % loss` is exactly the kind of confident-looking lie this product must not tell.
//!
//! These functions run at display rate (≤ 4 Hz per target), not once per sample, so the
//! one allocation they make — a scratch vector of the window's round-trip times — is off
//! the hot path. Recording a sample stays allocation-free; see [`crate::ring`].

use crate::sample::{ProbeOutcome, ProbeSample, Rtt};

/// The RFC 3550 smoothing factor: each new deviation moves the estimate by 1/16.
const JITTER_SMOOTHING: f64 = 16.0;

/// How many probes of each kind a window contained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutcomeCounts {
    /// Probes that got a reply.
    pub success: usize,
    /// Probes that got no reply in time — the only outcome that counts as loss.
    pub timeout: usize,
    /// Probes the destination explicitly rejected.
    pub unreachable: usize,
    /// Probes filtered on the path, carrying no information about the link.
    pub blocked: usize,
}

impl OutcomeCounts {
    /// Total number of probes counted.
    #[must_use]
    pub const fn total(self) -> usize {
        self.success + self.timeout + self.unreachable + self.blocked
    }

    /// Probes that actually tested whether packets get through.
    #[must_use]
    pub const fn delivery_attempts(self) -> usize {
        self.success + self.timeout
    }

    fn record(&mut self, outcome: ProbeOutcome) {
        match outcome {
            ProbeOutcome::Success(_) => self.success += 1,
            ProbeOutcome::Timeout => self.timeout += 1,
            ProbeOutcome::Unreachable => self.unreachable += 1,
            ProbeOutcome::Blocked => self.blocked += 1,
        }
    }
}

/// Round-trip time statistics over the successful probes in a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RttStats {
    /// How many successful probes these figures are based on.
    pub count: usize,
    /// Fastest reply.
    pub min: Rtt,
    /// Slowest reply.
    pub max: Rtt,
    /// Arithmetic mean, in milliseconds.
    pub mean_ms: f64,
    /// Median (50th percentile).
    pub p50: Rtt,
    /// 95th percentile.
    pub p95: Rtt,
    /// 99th percentile.
    pub p99: Rtt,
    /// Population standard deviation, in milliseconds.
    ///
    /// Exactly `0.0` for a single sample — that is the true value, not a placeholder.
    pub stddev_ms: f64,
    /// RFC 3550 interarrival jitter, in milliseconds.
    ///
    /// [`None`] with fewer than two successful probes, where the quantity is undefined.
    pub jitter_ms: Option<f64>,
}

/// Everything a window of samples says about a target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowStats {
    /// Probe counts by outcome.
    pub outcomes: OutcomeCounts,
    /// Percentage of delivery-testing probes that timed out.
    ///
    /// [`None`] when the window contained no delivery test at all — every probe was
    /// filtered or explicitly rejected, so no loss figure can honestly be quoted.
    pub loss_pct: Option<f64>,
    /// Round-trip statistics, or [`None`] when nothing replied.
    pub rtt: Option<RttStats>,
}

impl WindowStats {
    /// Computes statistics over `samples`, which must be in chronological order.
    ///
    /// Order matters only for jitter, which is defined over consecutive replies.
    #[must_use]
    pub fn of<'a, I>(samples: I) -> Self
    where
        I: IntoIterator<Item = &'a ProbeSample>,
    {
        let mut outcomes = OutcomeCounts::default();
        let mut replies: Vec<Rtt> = Vec::new();

        for sample in samples {
            outcomes.record(sample.outcome);
            if let Some(rtt) = sample.outcome.rtt() {
                replies.push(rtt);
            }
        }

        Self {
            outcomes,
            loss_pct: loss_pct(outcomes),
            rtt: rtt_stats(&mut replies),
        }
    }

    /// Whether the window suggests probes of this kind are filtered rather than lost.
    ///
    /// True when every probe was blocked, which the UI must surface as "probe blocked"
    /// instead of a loss percentage.
    #[must_use]
    pub const fn is_entirely_blocked(&self) -> bool {
        self.outcomes.blocked > 0 && self.outcomes.blocked == self.outcomes.total()
    }
}

/// Share of delivery tests that timed out, or [`None`] if there were none.
fn loss_pct(outcomes: OutcomeCounts) -> Option<f64> {
    let attempts = outcomes.delivery_attempts();
    if attempts == 0 {
        return None;
    }
    // Counts are bounded by the ring buffer's capacity — thousands at most — so they
    // convert to f64 exactly.
    #[allow(clippy::cast_precision_loss)]
    Some(outcomes.timeout as f64 / attempts as f64 * 100.0)
}

/// Statistics over the replies, or [`None`] if nothing replied.
///
/// Takes the replies by mutable reference because it needs them in chronological order
/// for jitter and in sorted order for percentiles; sorting in place avoids a second
/// allocation.
fn rtt_stats(replies: &mut [Rtt]) -> Option<RttStats> {
    if replies.is_empty() {
        return None;
    }

    let jitter_ms = rfc3550_jitter_ms(replies);
    let mean_ms = mean_ms(replies);
    let stddev_ms = stddev_ms(replies, mean_ms);

    replies.sort_unstable();
    // Non-empty, so both ends exist.
    let (min, max) = (*replies.first()?, *replies.last()?);

    Some(RttStats {
        count: replies.len(),
        min,
        max,
        mean_ms,
        p50: percentile(replies, 50)?,
        p95: percentile(replies, 95)?,
        p99: percentile(replies, 99)?,
        stddev_ms,
        jitter_ms,
    })
}

/// Arithmetic mean in milliseconds. `replies` must not be empty.
fn mean_ms(replies: &[Rtt]) -> f64 {
    let sum: f64 = replies.iter().map(|rtt| rtt.as_millis_f64()).sum();
    // Bounded by ring capacity; converts exactly.
    #[allow(clippy::cast_precision_loss)]
    let count = replies.len() as f64;
    sum / count
}

/// Population standard deviation in milliseconds. `replies` must not be empty.
fn stddev_ms(replies: &[Rtt], mean_ms: f64) -> f64 {
    let sum_squares: f64 = replies
        .iter()
        .map(|rtt| {
            let deviation = rtt.as_millis_f64() - mean_ms;
            deviation * deviation
        })
        .sum();
    // Bounded by ring capacity; converts exactly.
    #[allow(clippy::cast_precision_loss)]
    let count = replies.len() as f64;
    (sum_squares / count).sqrt()
}

/// Nearest-rank percentile over a **sorted** slice.
///
/// Rank is `ceil(p/100 × n)`, computed with integer arithmetic so the boundaries are
/// exact rather than dependent on floating-point rounding. Returns [`None`] for an empty
/// slice; `p` is clamped to `1..=100`.
fn percentile(sorted: &[Rtt], p: u32) -> Option<Rtt> {
    let count = sorted.len();
    if count == 0 {
        return None;
    }
    let p = p.clamp(1, 100) as usize;
    // Ceiling division: rank = ceil(p * count / 100), always within 1..=count.
    let rank = (p * count).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

/// RFC 3550 interarrival jitter, in milliseconds, over consecutive replies.
///
/// RFC 3550 defines jitter over the difference in *transit* times of consecutive
/// packets. A round-trip probe cannot observe one-way transit, so — as ping
/// implementations conventionally do — the difference of consecutive round-trip times
/// stands in for it, smoothed by the RFC's `J += (|D| - J) / 16`.
///
/// Timeouts and filtered probes are skipped rather than treated as gaps: the estimate is
/// defined over replies that actually arrived. Returns [`None`] with fewer than two.
fn rfc3550_jitter_ms(replies: &[Rtt]) -> Option<f64> {
    if replies.len() < 2 {
        return None;
    }
    let mut jitter = 0.0_f64;
    for pair in replies.windows(2) {
        let (previous, current) = (pair.first()?, pair.get(1)?);
        let deviation = (current.as_millis_f64() - previous.as_millis_f64()).abs();
        jitter += (deviation - jitter) / JITTER_SMOOTHING;
    }
    Some(jitter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Comparison tolerance for values that go through a square root or a division.
    const EPSILON: f64 = 1e-9;

    fn ms(millis: u32) -> Rtt {
        Rtt::from_micros(millis * 1_000)
    }

    fn samples(outcomes: &[ProbeOutcome]) -> Vec<ProbeSample> {
        let start = Instant::now();
        outcomes
            .iter()
            .enumerate()
            .map(|(index, outcome)| {
                ProbeSample::new(start + Duration::from_secs(index as u64), *outcome)
            })
            .collect()
    }

    fn replies(millis: &[u32]) -> Vec<ProbeSample> {
        let outcomes: Vec<_> = millis
            .iter()
            .map(|value| ProbeOutcome::Success(ms(*value)))
            .collect();
        samples(&outcomes)
    }

    // --- empty and degenerate windows ------------------------------------------------

    #[test]
    fn an_empty_window_asserts_nothing() {
        let stats = WindowStats::of(&[]);
        assert_eq!(stats.outcomes, OutcomeCounts::default());
        assert_eq!(
            stats.loss_pct, None,
            "no probes means no loss figure, not 0 %"
        );
        assert_eq!(stats.rtt, None);
        assert!(!stats.is_entirely_blocked());
    }

    #[test]
    fn a_window_of_only_timeouts_is_total_loss_with_no_rtt() {
        let stats = WindowStats::of(&samples(&[ProbeOutcome::Timeout; 5]));
        assert_eq!(stats.loss_pct, Some(100.0));
        assert_eq!(stats.rtt, None);
        assert_eq!(stats.outcomes.timeout, 5);
    }

    #[test]
    fn a_window_of_only_blocked_probes_quotes_no_loss_at_all() {
        // The headline honesty rule: a filtered probe kind must never be rendered as
        // 100 % packet loss on a link that may be perfectly healthy.
        let stats = WindowStats::of(&samples(&[ProbeOutcome::Blocked; 4]));
        assert_eq!(stats.loss_pct, None);
        assert_eq!(stats.rtt, None);
        assert_eq!(stats.outcomes.blocked, 4);
        assert!(stats.is_entirely_blocked());
    }

    #[test]
    fn unreachable_replies_stay_out_of_the_loss_ratio() {
        let stats = WindowStats::of(&samples(&[
            ProbeOutcome::Success(ms(10)),
            ProbeOutcome::Unreachable,
            ProbeOutcome::Unreachable,
        ]));
        // One delivery test, and it succeeded.
        assert_eq!(stats.loss_pct, Some(0.0));
        assert_eq!(stats.outcomes.unreachable, 2);
        assert!(!stats.is_entirely_blocked());
    }

    #[test]
    fn a_mixed_window_counts_every_outcome() {
        let stats = WindowStats::of(&samples(&[
            ProbeOutcome::Success(ms(10)),
            ProbeOutcome::Timeout,
            ProbeOutcome::Unreachable,
            ProbeOutcome::Blocked,
        ]));
        assert_eq!(
            stats.outcomes,
            OutcomeCounts {
                success: 1,
                timeout: 1,
                unreachable: 1,
                blocked: 1,
            }
        );
        assert_eq!(stats.outcomes.total(), 4);
        assert_eq!(stats.outcomes.delivery_attempts(), 2);
        assert_eq!(stats.loss_pct, Some(50.0));
    }

    // --- round-trip statistics -------------------------------------------------------

    #[test]
    fn a_single_reply_yields_exact_statistics() {
        let stats = WindowStats::of(&replies(&[42])).rtt.unwrap();
        assert_eq!(stats.count, 1);
        assert_eq!(stats.min, ms(42));
        assert_eq!(stats.max, ms(42));
        assert_eq!(stats.p50, ms(42));
        assert_eq!(stats.p95, ms(42));
        assert_eq!(stats.p99, ms(42));
        assert!((stats.mean_ms - 42.0).abs() < EPSILON);
        // The population standard deviation of one point genuinely is zero.
        assert!(stats.stddev_ms.abs() < EPSILON);
        assert_eq!(stats.jitter_ms, None, "jitter needs two replies to exist");
    }

    #[test]
    fn computes_mean_and_extremes() {
        let stats = WindowStats::of(&replies(&[10, 30, 20])).rtt.unwrap();
        assert_eq!(stats.min, ms(10));
        assert_eq!(stats.max, ms(30));
        assert!((stats.mean_ms - 20.0).abs() < EPSILON);
    }

    #[test]
    fn computes_population_standard_deviation() {
        // 10, 20, 30, 40 -> mean 25, deviations ±15 and ±5, variance 125.
        let stats = WindowStats::of(&replies(&[10, 20, 30, 40])).rtt.unwrap();
        assert!((stats.stddev_ms - 125.0_f64.sqrt()).abs() < EPSILON);
    }

    #[test]
    fn identical_replies_have_no_spread() {
        let stats = WindowStats::of(&replies(&[7; 6])).rtt.unwrap();
        assert!(stats.stddev_ms.abs() < EPSILON);
        assert_eq!(stats.jitter_ms, Some(0.0));
    }

    // --- percentiles -----------------------------------------------------------------

    #[test]
    fn percentiles_use_nearest_rank() {
        let sorted: Vec<Rtt> = (1..=10).map(ms).collect();
        assert_eq!(percentile(&sorted, 50), Some(ms(5)));
        assert_eq!(percentile(&sorted, 95), Some(ms(10)));
        assert_eq!(percentile(&sorted, 99), Some(ms(10)));
        assert_eq!(percentile(&sorted, 10), Some(ms(1)));
        assert_eq!(percentile(&sorted, 100), Some(ms(10)));
    }

    #[test]
    fn percentiles_handle_the_smallest_slices() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[ms(5)], 99), Some(ms(5)));
        assert_eq!(percentile(&[ms(5), ms(9)], 50), Some(ms(5)));
        assert_eq!(percentile(&[ms(5), ms(9)], 51), Some(ms(9)));
    }

    #[test]
    fn percentiles_never_index_out_of_range() {
        // Ranks are clamped at both ends, so no combination can escape the slice.
        for count in 1_usize..=64 {
            let sorted: Vec<Rtt> = (1..=count)
                .map(|value| ms(u32::try_from(value).unwrap()))
                .collect();
            for p in 0..=120 {
                let value = percentile(&sorted, p).expect("a non-empty slice always has one");
                assert!(sorted.contains(&value));
            }
        }
    }

    #[test]
    fn percentiles_come_from_sorted_order_not_arrival_order() {
        let stats = WindowStats::of(&replies(&[100, 1, 50])).rtt.unwrap();
        assert_eq!(stats.p50, ms(50));
        assert_eq!(stats.min, ms(1));
        assert_eq!(stats.max, ms(100));
    }

    // --- jitter ----------------------------------------------------------------------

    #[test]
    fn jitter_follows_the_rfc_3550_recurrence() {
        // 10 -> 20 -> 15 ms.
        // J = 0 + (10 - 0)/16 = 0.625
        // J = 0.625 + (5 - 0.625)/16 = 0.8984375
        // Every step is exact in binary floating point, so this compares exactly.
        let jitter = rfc3550_jitter_ms(&[ms(10), ms(20), ms(15)]);
        assert_eq!(jitter, Some(0.898_437_5));
    }

    #[test]
    fn jitter_needs_two_replies() {
        assert_eq!(rfc3550_jitter_ms(&[]), None);
        assert_eq!(rfc3550_jitter_ms(&[ms(10)]), None);
        assert_eq!(rfc3550_jitter_ms(&[ms(10), ms(10)]), Some(0.0));
    }

    #[test]
    fn jitter_is_computed_over_replies_only_skipping_failures() {
        // The timeout between the two replies must not create a phantom deviation.
        let with_gap = WindowStats::of(&samples(&[
            ProbeOutcome::Success(ms(10)),
            ProbeOutcome::Timeout,
            ProbeOutcome::Blocked,
            ProbeOutcome::Success(ms(20)),
        ]));
        let without_gap = WindowStats::of(&replies(&[10, 20]));
        assert_eq!(
            with_gap.rtt.unwrap().jitter_ms,
            without_gap.rtt.unwrap().jitter_ms
        );
    }

    #[test]
    fn jitter_grows_with_instability_and_decays_with_calm() {
        let unstable = WindowStats::of(&replies(&[10, 90, 10, 90, 10]))
            .rtt
            .unwrap()
            .jitter_ms
            .unwrap();
        let steady = WindowStats::of(&replies(&[50, 50, 50, 50, 50]))
            .rtt
            .unwrap()
            .jitter_ms
            .unwrap();
        assert!(unstable > steady);
        assert!(steady.abs() < EPSILON);
    }

    #[test]
    fn jitter_is_direction_agnostic() {
        // The recurrence uses |D|, so rising and falling by the same amounts match.
        let rising = rfc3550_jitter_ms(&[ms(10), ms(20), ms(30)]);
        let falling = rfc3550_jitter_ms(&[ms(30), ms(20), ms(10)]);
        assert_eq!(rising, falling);
    }

    // --- extremes --------------------------------------------------------------------

    #[test]
    fn handles_the_largest_representable_round_trip_time() {
        let stats = WindowStats::of(&samples(&[
            ProbeOutcome::Success(Rtt::MAX),
            ProbeOutcome::Success(Rtt::from_micros(0)),
        ]))
        .rtt
        .unwrap();
        assert_eq!(stats.min, Rtt::from_micros(0));
        assert_eq!(stats.max, Rtt::MAX);
        assert!(stats.mean_ms.is_finite());
        assert!(stats.stddev_ms.is_finite());
        assert!(stats.jitter_ms.is_some_and(f64::is_finite));
    }

    #[test]
    fn loss_percentage_stays_within_bounds_for_every_mix() {
        for successes in 0..8_usize {
            for timeouts in 0..8_usize {
                let mut outcomes = vec![ProbeOutcome::Success(ms(1)); successes];
                outcomes.extend(std::iter::repeat_n(ProbeOutcome::Timeout, timeouts));
                let stats = WindowStats::of(&samples(&outcomes));
                match stats.loss_pct {
                    None => assert_eq!(successes + timeouts, 0),
                    Some(loss) => assert!((0.0..=100.0).contains(&loss), "loss was {loss}"),
                }
            }
        }
    }
}
