//! Putting several targets' samples on one time axis.
//!
//! A chart with one line per endpoint needs a *single* set of x values: sixteen endpoints
//! probed a second apart do not share sample times, and drawing each on its own axis is
//! sixteen charts rather than one. This module is that alignment, and it lives here rather
//! than in the UI for the reason every calculation does — it is a decision about what the
//! numbers mean, and it is testable.
//!
//! Three rules, all of which exist because the obvious shortcut lies:
//!
//! **A slot with no sample stays [`None`].** It is a gap in the line, not a zero and not an
//! interpolation between its neighbours. An endpoint probed once every ten seconds really
//! does have nothing to say about the nine seconds in between, and a chart that joined
//! those points would draw a measurement nobody took.
//!
//! **Nothing is averaged into a slot.** Where two samples fall in one, the later one is
//! shown. Averaging would smooth away exactly the spike the user opened the chart to find,
//! and the statistics beside the chart — which *are* computed over every sample — remain
//! the authority on what happened.
//!
//! **The grid ends at `now`.** The rightmost slot is the present, so several endpoints
//! aligned against the same grid are aligned against the same instant, and a slower one
//! trails off to the left rather than appearing to have stopped.
//!
//! Like everything in this crate it reads no clock: `now` is passed in.

use std::time::{Duration, Instant};

use crate::history::SampleHistory;
use crate::sample::Rtt;
use crate::Error;

/// A fixed ladder of instants, newest last, to place samples on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    step: Duration,
    points: usize,
}

impl Grid {
    /// Builds a grid of `points` slots, `step` apart, ending at whatever `now` is given.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroInterval`] for a zero step — every sample would land in one
    /// slot — or [`Error::ZeroCapacity`] for a grid with no slots at all.
    pub fn new(step: Duration, points: usize) -> Result<Self, Error> {
        if step.is_zero() {
            return Err(Error::ZeroInterval);
        }
        if points == 0 {
            return Err(Error::ZeroCapacity);
        }
        Ok(Self { step, points })
    }

    /// How many slots the grid has.
    #[must_use]
    pub const fn points(&self) -> usize {
        self.points
    }

    /// The span the grid covers.
    #[must_use]
    pub fn span(&self) -> Duration {
        self.step
            .saturating_mul(u32::try_from(self.points.saturating_sub(1)).unwrap_or(u32::MAX))
    }

    /// Seconds before `now` for each slot — negative, ascending, ending at zero.
    ///
    /// The x axis every series on this grid shares. Ages rather than absolute times because
    /// a monotonic instant means nothing outside this process, and "twelve seconds ago" is
    /// what the axis is labelled with anyway.
    #[must_use]
    pub fn ages_secs(&self) -> Vec<f64> {
        let step = self.step.as_secs_f64();
        (0..self.points)
            // Counting down from the oldest slot, so the last value is exactly zero rather
            // than the accumulated rounding of `points` additions.
            .map(|index| {
                let back = self.points.saturating_sub(1).saturating_sub(index);
                // A grid is tens of points; the conversion is exact far past that.
                #[allow(clippy::cast_precision_loss)]
                let back = back as f64;
                -(back * step)
            })
            .collect()
    }

    /// Places a history's round-trip times on the grid.
    ///
    /// Samples older than the grid are dropped, and a probe that did not come back leaves
    /// its slot empty — which is the same [`None`] as a slot nothing was measured in, and
    /// deliberately so: both mean "no round trip was observed here", and the chart is not
    /// where the difference between a timeout and a silence is explained.
    #[must_use]
    pub fn place(&self, history: &SampleHistory, now: Instant) -> Vec<Option<f64>> {
        let mut values = vec![None; self.points];
        // Oldest first, so a later sample in the same slot overwrites an earlier one.
        for sample in history.recent(history.capacity()) {
            let Some(slot) = self.slot_of(sample.at, now) else {
                continue;
            };
            if let Some(rtt) = sample.outcome.rtt() {
                values[slot] = Some(Rtt::as_millis_f64(rtt));
            }
        }
        values
    }

    /// Which slot an instant belongs to, or [`None`] if it falls outside the grid.
    ///
    /// A sample belongs to the slot it is nearest, so the assignment error is never more
    /// than half a step — and a sample from the future, which a clock adjustment can
    /// produce, lands in the newest slot rather than off the end of the array.
    fn slot_of(&self, at: Instant, now: Instant) -> Option<usize> {
        let age = now.saturating_duration_since(at);
        let half = self.step / 2;
        if age > self.span().saturating_add(half) {
            return None;
        }
        let steps_back = (age.saturating_add(half).as_nanos() / self.step.as_nanos().max(1))
            .try_into()
            .unwrap_or(usize::MAX);
        Some(self.points.saturating_sub(1).saturating_sub(steps_back))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ProbeOutcome, ProbeSample};

    fn grid() -> Grid {
        Grid::new(Duration::from_secs(1), 5).unwrap()
    }

    fn history(samples: &[(u64, Option<u32>)], start: Instant) -> SampleHistory {
        let mut history = SampleHistory::new(64).unwrap();
        for (secs, micros) in samples {
            let outcome = micros.map_or(ProbeOutcome::Timeout, |micros| {
                ProbeOutcome::Success(Rtt::from_micros(micros))
            });
            history.record(ProbeSample::new(
                start + Duration::from_secs(*secs),
                outcome,
            ));
        }
        history
    }

    #[test]
    fn rejects_a_grid_that_could_hold_nothing() {
        assert_eq!(
            Grid::new(Duration::ZERO, 5).unwrap_err(),
            Error::ZeroInterval
        );
        assert_eq!(
            Grid::new(Duration::from_secs(1), 0).unwrap_err(),
            Error::ZeroCapacity
        );
    }

    #[test]
    fn the_axis_ends_at_now_and_ascends() {
        assert_eq!(grid().ages_secs(), vec![-4.0, -3.0, -2.0, -1.0, 0.0]);
        assert_eq!(grid().span(), Duration::from_secs(4));
    }

    #[test]
    fn a_single_point_grid_is_the_present_alone() {
        let grid = Grid::new(Duration::from_secs(1), 1).unwrap();
        assert_eq!(grid.ages_secs(), vec![0.0]);
        assert_eq!(grid.span(), Duration::ZERO);
    }

    #[test]
    fn samples_land_on_the_slot_they_belong_to() {
        let start = Instant::now();
        let history = history(
            &[(0, Some(1_000)), (1, Some(2_000)), (2, Some(3_000))],
            start,
        );

        let placed = grid().place(&history, start + Duration::from_secs(2));

        assert_eq!(placed, vec![None, None, Some(1.0), Some(2.0), Some(3.0)]);
    }

    #[test]
    fn a_gap_stays_a_gap() {
        // The rule the whole module exists for: an endpoint probed every other second has
        // nothing to say about the seconds in between, and joining the points would draw a
        // measurement nobody took.
        let start = Instant::now();
        let history = history(
            &[(0, Some(1_000)), (2, Some(3_000)), (4, Some(5_000))],
            start,
        );

        let placed = grid().place(&history, start + Duration::from_secs(4));

        assert_eq!(placed, vec![Some(1.0), None, Some(3.0), None, Some(5.0)]);
    }

    #[test]
    fn a_probe_that_did_not_come_back_leaves_its_slot_empty() {
        let start = Instant::now();
        let history = history(&[(0, Some(1_000)), (1, None), (2, Some(3_000))], start);

        let placed = grid().place(&history, start + Duration::from_secs(2));

        assert_eq!(placed, vec![None, None, Some(1.0), None, Some(3.0)]);
    }

    #[test]
    fn samples_older_than_the_grid_are_dropped() {
        let start = Instant::now();
        let history = history(&[(0, Some(1_000)), (9, Some(9_000))], start);

        let placed = grid().place(&history, start + Duration::from_secs(9));

        assert_eq!(placed, vec![None, None, None, None, Some(9.0)]);
    }

    #[test]
    fn the_later_of_two_samples_in_one_slot_is_shown() {
        // Never an average: smoothing would remove the spike the chart was opened to find,
        // and the statistics beside it are computed over every sample regardless.
        let start = Instant::now();
        let mut history = SampleHistory::new(8).unwrap();
        history.record(ProbeSample::new(
            start,
            ProbeOutcome::Success(Rtt::from_micros(1_000)),
        ));
        history.record(ProbeSample::new(
            start + Duration::from_millis(100),
            ProbeOutcome::Success(Rtt::from_micros(9_000)),
        ));

        let placed = grid().place(&history, start);

        assert_eq!(placed[4], Some(9.0));
    }

    #[test]
    fn a_sample_between_two_slots_takes_the_nearer_one() {
        let start = Instant::now();
        let history = history(&[(0, Some(1_000))], start);

        // 1.4 s old: nearer the slot one step back.
        let placed = grid().place(&history, start + Duration::from_millis(1_400));
        assert_eq!(placed, vec![None, None, None, Some(1.0), None]);

        // 1.6 s old: nearer the slot two steps back.
        let placed = grid().place(&history, start + Duration::from_millis(1_600));
        assert_eq!(placed, vec![None, None, Some(1.0), None, None]);
    }

    #[test]
    fn a_sample_from_the_future_lands_in_the_present() {
        // A clock adjustment can stamp a sample ahead of `now`; it must not index off the
        // end of the array or be silently discarded.
        let start = Instant::now();
        let history = history(&[(5, Some(1_000))], start);

        let placed = grid().place(&history, start);

        assert_eq!(placed, vec![None, None, None, None, Some(1.0)]);
    }

    #[test]
    fn an_empty_history_is_all_gaps() {
        let history = SampleHistory::new(4).unwrap();
        assert_eq!(grid().place(&history, Instant::now()), vec![None; 5]);
    }

    #[test]
    fn a_slow_endpoint_keeps_its_few_points_rather_than_none() {
        // A demoted endpoint is probed a tenth as often. Its line is sparse, which is the
        // truth about it, and it must still be on the chart.
        let start = Instant::now();
        let grid = Grid::new(Duration::from_secs(1), 30).unwrap();
        let history = history(
            &[(0, Some(1_000)), (10, Some(2_000)), (20, Some(3_000))],
            start,
        );

        let placed = grid.place(&history, start + Duration::from_secs(20));

        assert_eq!(placed.iter().filter(|value| value.is_some()).count(), 3);
        assert_eq!(placed[29], Some(3.0));
        assert_eq!(placed[19], Some(2.0));
        assert_eq!(placed[9], Some(1.0));
    }
}
