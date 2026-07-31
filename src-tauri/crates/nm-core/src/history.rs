//! Bounded per-target sample history.

use std::time::{Duration, Instant};

use crate::ring::RingBuffer;
use crate::sample::ProbeSample;
use crate::stats::WindowStats;
use crate::Error;

/// A target's recent probe results, capped at a fixed number of samples.
///
/// Recording is allocation-free and O(1); statistics are computed on demand, at display
/// rate. The cap is what bounds memory: history never grows with uptime, only with the
/// number of monitored targets.
#[derive(Debug, Clone)]
pub struct SampleHistory {
    samples: RingBuffer<ProbeSample>,
}

impl SampleHistory {
    /// Creates a history holding at most `capacity` samples.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroCapacity`] if `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, Error> {
        Ok(Self {
            samples: RingBuffer::new(capacity)?,
        })
    }

    /// Records a sample, evicting the oldest one if the history is full.
    pub fn record(&mut self, sample: ProbeSample) {
        self.samples.push(sample);
    }

    /// How many samples are currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether anything has been recorded (and not yet evicted).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Maximum number of samples retained.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.samples.capacity()
    }

    /// The most recent sample, if any.
    #[must_use]
    pub fn latest(&self) -> Option<&ProbeSample> {
        self.samples.iter().next_back()
    }

    /// The last `count` samples, oldest first.
    ///
    /// Every retained sample when fewer than `count` are held. Exists so a caller can draw
    /// the series the user sees without reaching into the ring buffer, and yields failures
    /// alongside successes: a sparkline that silently omitted the timeouts would draw a
    /// smooth line through an outage.
    pub fn recent(&self, count: usize) -> impl Iterator<Item = &ProbeSample> + '_ {
        self.samples.iter().skip(self.len().saturating_sub(count))
    }

    /// Statistics over every retained sample.
    #[must_use]
    pub fn stats(&self) -> WindowStats {
        WindowStats::of(&self.samples)
    }

    /// Statistics over samples taken at or after `cutoff`.
    #[must_use]
    pub fn stats_since(&self, cutoff: Instant) -> WindowStats {
        WindowStats::of(self.samples.iter().filter(|sample| sample.at >= cutoff))
    }

    /// Statistics over the last `window` of time, as of `now`.
    ///
    /// If `now - window` underflows the clock's origin the whole history is used, which
    /// is the honest reading of "everything within the last very long time".
    #[must_use]
    pub fn stats_for_window(&self, now: Instant, window: Duration) -> WindowStats {
        now.checked_sub(window)
            .map_or_else(|| self.stats(), |cutoff| self.stats_since(cutoff))
    }

    /// Drops every retained sample, keeping the allocation.
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ProbeOutcome, Rtt};

    fn ms(millis: u32) -> Rtt {
        Rtt::from_micros(millis * 1_000)
    }

    #[test]
    fn rejects_a_history_that_could_hold_nothing() {
        assert_eq!(SampleHistory::new(0).unwrap_err(), Error::ZeroCapacity);
    }

    #[test]
    fn an_untouched_history_reports_nothing_measured() {
        let history = SampleHistory::new(8).unwrap();
        assert!(history.is_empty());
        assert_eq!(history.latest(), None);

        let stats = history.stats();
        assert_eq!(stats.loss_pct, None);
        assert_eq!(stats.rtt, None);
    }

    #[test]
    fn keeps_only_the_most_recent_samples() {
        let start = Instant::now();
        let mut history = SampleHistory::new(3).unwrap();
        for index in 0..10_u64 {
            history.record(ProbeSample::new(
                start + Duration::from_secs(index),
                ProbeOutcome::Success(ms(u32::try_from(index).unwrap())),
            ));
        }

        assert_eq!(history.len(), 3);
        assert_eq!(history.capacity(), 3);
        let stats = history.stats().rtt.unwrap();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.min, ms(7));
        assert_eq!(stats.max, ms(9));
    }

    #[test]
    fn latest_is_the_newest_sample_even_after_wrapping() {
        let start = Instant::now();
        let mut history = SampleHistory::new(2).unwrap();
        for index in 0..5_u64 {
            history.record(ProbeSample::new(
                start + Duration::from_secs(index),
                ProbeOutcome::Success(ms(u32::try_from(index).unwrap())),
            ));
        }
        assert_eq!(
            history.latest().unwrap().outcome,
            ProbeOutcome::Success(ms(4))
        );
    }

    #[test]
    fn a_time_window_excludes_older_samples() {
        let start = Instant::now();
        let mut history = SampleHistory::new(16).unwrap();
        // Ten seconds of probes: the first five timed out, the rest replied.
        for index in 0..10_u64 {
            let outcome = if index < 5 {
                ProbeOutcome::Timeout
            } else {
                ProbeOutcome::Success(ms(20))
            };
            history.record(ProbeSample::new(
                start + Duration::from_secs(index),
                outcome,
            ));
        }
        let now = start + Duration::from_secs(9);

        let all = history.stats();
        assert_eq!(all.loss_pct, Some(50.0));

        // The window is inclusive at its cutoff, so four seconds back from t=9 starts at
        // t=5 — the first reply — and excludes the timeout stamped at t=4.
        let recent = history.stats_for_window(now, Duration::from_secs(4));
        assert_eq!(recent.loss_pct, Some(0.0));
        assert_eq!(recent.rtt.unwrap().count, 5);

        // One second earlier the window still reaches back into the outage.
        let straddling = history.stats_for_window(now, Duration::from_secs(5));
        assert_eq!(straddling.outcomes.timeout, 1);
        assert_eq!(straddling.outcomes.success, 5);
    }

    #[test]
    fn a_window_with_no_samples_in_it_asserts_nothing() {
        let start = Instant::now();
        let mut history = SampleHistory::new(4).unwrap();
        history.record(ProbeSample::new(start, ProbeOutcome::Success(ms(10))));

        let much_later = start + Duration::from_secs(3_600);
        let stats = history.stats_for_window(much_later, Duration::from_secs(60));
        assert_eq!(stats.outcomes.total(), 0);
        assert_eq!(
            stats.loss_pct, None,
            "an idle window must not read as 0 % loss"
        );
        assert_eq!(stats.rtt, None);
    }

    #[test]
    fn a_window_wider_than_the_clocks_origin_falls_back_to_everything() {
        let start = Instant::now();
        let mut history = SampleHistory::new(4).unwrap();
        history.record(ProbeSample::new(start, ProbeOutcome::Success(ms(10))));

        // `now - Duration::MAX` cannot be represented; the whole history is used instead
        // of silently reporting an empty window.
        let stats = history.stats_for_window(start, Duration::MAX);
        assert_eq!(stats.outcomes.success, 1);
    }

    #[test]
    fn samples_stamped_out_of_order_are_still_filtered_by_time() {
        // Instant is monotonic, but a suspended machine can still produce surprising
        // gaps; filtering must depend on the stamp, never on position in the buffer.
        let start = Instant::now();
        let mut history = SampleHistory::new(4).unwrap();
        history.record(ProbeSample::new(
            start + Duration::from_secs(100),
            ProbeOutcome::Success(ms(10)),
        ));
        history.record(ProbeSample::new(start, ProbeOutcome::Timeout));

        let stats = history.stats_since(start + Duration::from_secs(50));
        assert_eq!(stats.outcomes.success, 1);
        assert_eq!(stats.outcomes.timeout, 0);
    }

    #[test]
    fn the_recent_slice_is_the_newest_samples_oldest_first() {
        let start = Instant::now();
        let mut history = SampleHistory::new(8).unwrap();
        for index in 0..8_u64 {
            history.record(ProbeSample::new(
                start + Duration::from_secs(index),
                ProbeOutcome::Success(ms(u32::try_from(index).unwrap())),
            ));
        }

        let last_three: Vec<_> = history.recent(3).map(|sample| sample.outcome).collect();
        assert_eq!(
            last_three,
            vec![
                ProbeOutcome::Success(ms(5)),
                ProbeOutcome::Success(ms(6)),
                ProbeOutcome::Success(ms(7)),
            ]
        );
    }

    #[test]
    fn asking_for_more_recent_samples_than_exist_yields_what_there_is() {
        let start = Instant::now();
        let mut history = SampleHistory::new(8).unwrap();
        history.record(ProbeSample::new(start, ProbeOutcome::Timeout));

        assert_eq!(history.recent(100).count(), 1);
        assert_eq!(history.recent(0).count(), 0);
        assert_eq!(SampleHistory::new(4).unwrap().recent(10).count(), 0);
    }

    #[test]
    fn the_recent_slice_keeps_failures_rather_than_smoothing_over_them() {
        // A sparkline drawn from successes alone would show a flat healthy line through
        // an outage.
        let start = Instant::now();
        let mut history = SampleHistory::new(4).unwrap();
        history.record(ProbeSample::new(start, ProbeOutcome::Success(ms(10))));
        history.record(ProbeSample::new(
            start + Duration::from_secs(1),
            ProbeOutcome::Timeout,
        ));

        let outcomes: Vec<_> = history.recent(4).map(|sample| sample.outcome).collect();
        assert_eq!(
            outcomes,
            vec![ProbeOutcome::Success(ms(10)), ProbeOutcome::Timeout]
        );
    }

    #[test]
    fn the_recent_slice_survives_the_ring_wrapping() {
        let start = Instant::now();
        let mut history = SampleHistory::new(3).unwrap();
        for index in 0..7_u64 {
            history.record(ProbeSample::new(
                start + Duration::from_secs(index),
                ProbeOutcome::Success(ms(u32::try_from(index).unwrap())),
            ));
        }

        let all: Vec<_> = history.recent(10).map(|sample| sample.outcome).collect();
        assert_eq!(
            all,
            vec![
                ProbeOutcome::Success(ms(4)),
                ProbeOutcome::Success(ms(5)),
                ProbeOutcome::Success(ms(6)),
            ]
        );
    }

    #[test]
    fn clearing_discards_every_sample() {
        let mut history = SampleHistory::new(4).unwrap();
        history.record(ProbeSample::new(Instant::now(), ProbeOutcome::Timeout));
        history.clear();
        assert!(history.is_empty());
        assert_eq!(history.stats().loss_pct, None);
    }
}
