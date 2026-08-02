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
//! **A slot shows its slowest sample, and nothing is averaged.** Where several samples fall
//! in one slot the worst of them is drawn. Averaging, or keeping whichever happened to be
//! last, would smooth away exactly the spike the user opened the chart to find — and a chart
//! that hides the worst moment of every interval is the wrong instrument for a product about
//! degradation. The statistics beside the chart *are* computed over every sample and remain
//! the authority on what the round trip typically is.
//!
//! The consequence has to be stated wherever this is drawn: a line of per-slot maxima sits
//! above the mean in the row beside it, on purpose.
//!
//! **The grid is anchored where monitoring began, and it steps rather than slides.** Slot
//! boundaries are whole steps from `start`, so the axis moves once every step instead of
//! drifting continuously — which is what stops a line the user is trying to point at from
//! sliding out from under the cursor. Until enough time has passed to fill the window the
//! drawing grows rightwards from the left edge rather than being pinned to the right with
//! empty space behind it; only then does the window begin to scroll.
//!
//! Every endpoint of one application shares that ladder, so a slower one trails off to the
//! left rather than appearing to have stopped.
//!
//! Like everything in this crate it reads no clock: `start` and `now` are passed in.

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

    /// Seconds since `start` for each slot of the window that ends at `now`.
    ///
    /// The x axis every series on this grid shares. Elapsed time rather than an age, because
    /// "the chart begins where monitoring began" is what makes the drawing grow from the
    /// left edge instead of appearing pinned to the right with empty space behind it — and
    /// because a monotonic instant means nothing outside this process.
    ///
    /// The whole ladder is returned from the first moment, even before there is anything to
    /// draw in most of it. That is what fixes the width of the axis: the line grows into it
    /// rather than being stretched across it.
    #[must_use]
    pub fn elapsed_secs(&self, start: Instant, now: Instant) -> Vec<f64> {
        let step = self.step.as_secs_f64();
        let first = self.first_slot(start, now);
        (0..self.points)
            .map(|index| {
                // A window is tens of slots and a session is hours; both convert exactly.
                #[allow(clippy::cast_precision_loss)]
                let slot = first.saturating_add(index as u64) as f64;
                slot * step
            })
            .collect()
    }

    /// Places a history's round-trip times on the grid, worst sample per slot.
    ///
    /// Samples outside the window are dropped, and a slot in which nothing answered stays
    /// empty. **That is what makes an empty slot mean something**: as long as the step is
    /// longer than the interval the target is probed at, every slot contains probes, so a
    /// break in the line is packets that did not come back rather than a moment nobody
    /// looked. A grid finer than the sampling would draw ordinary scheduling as loss.
    #[must_use]
    pub fn place(&self, history: &SampleHistory, start: Instant, now: Instant) -> Vec<Option<f64>> {
        let mut values: Vec<Option<f64>> = vec![None; self.points];
        let first = self.first_slot(start, now);
        let newest = self.newest_slot(start, now);
        for sample in history.recent(history.capacity()) {
            // A sample from before monitoring began cannot belong to this window; one from
            // after `now`, which a clock adjustment can produce, belongs to the present
            // rather than off the end of the array.
            let slot = self.slot_of(start, sample.at).min(newest);
            let Some(index) = slot
                .checked_sub(first)
                .and_then(|at| usize::try_from(at).ok())
            else {
                continue;
            };
            let Some(cell) = values.get_mut(index) else {
                continue;
            };
            if let Some(rtt) = sample.outcome.rtt() {
                let millis = Rtt::as_millis_f64(rtt);
                *cell = Some(cell.map_or(millis, |held: f64| held.max(millis)));
            }
        }
        values
    }

    /// Which slot of the whole session an instant belongs to, counted from `start`.
    ///
    /// Whole steps from the anchor, so the boundaries are the same for every endpoint of an
    /// application and do not move between emissions. That is the stepping: the ladder
    /// advances when `now` crosses a boundary, not continuously.
    fn slot_of(&self, start: Instant, at: Instant) -> u64 {
        let elapsed = at.saturating_duration_since(start);
        u64::try_from(elapsed.as_nanos() / self.step.as_nanos().max(1)).unwrap_or(u64::MAX)
    }

    /// The newest slot the window may show.
    fn newest_slot(&self, start: Instant, now: Instant) -> u64 {
        self.slot_of(start, now)
    }

    /// The oldest slot the window shows.
    ///
    /// Zero until a whole window has elapsed, which is what anchors the drawing at the left
    /// edge while it grows; after that it advances one slot at a time and the window
    /// scrolls.
    fn first_slot(&self, start: Instant, now: Instant) -> u64 {
        let span = u64::try_from(self.points.saturating_sub(1)).unwrap_or(u64::MAX);
        self.newest_slot(start, now).saturating_sub(span)
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
    fn the_axis_begins_where_monitoring_did_and_ascends() {
        let start = Instant::now();
        assert_eq!(
            grid().elapsed_secs(start, start),
            vec![0.0, 1.0, 2.0, 3.0, 4.0]
        );
        assert_eq!(grid().span(), Duration::from_secs(4));
    }

    #[test]
    fn the_drawing_grows_from_the_left_before_the_window_is_full() {
        // The failure this exists to fix: a fresh application drew a short line pinned to the
        // right edge with empty space behind it, and every emission walked the whole picture
        // one second to the left — which is what made a line something to be *caught* with
        // the pointer.
        let start = Instant::now();
        let history = history(&[(0, Some(1_000)), (1, Some(2_000))], start);

        let placed = grid().place(&history, start, start + Duration::from_secs(1));

        assert_eq!(
            placed,
            vec![Some(1.0), Some(2.0), None, None, None],
            "two seconds of measurement occupy the first two slots, not the last two"
        );
        assert_eq!(
            grid().elapsed_secs(start, start + Duration::from_secs(1)),
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            "and the axis keeps its full width, so the line grows into it"
        );
    }

    #[test]
    fn the_window_scrolls_only_once_it_is_full() {
        let start = Instant::now();
        for (secs, expected_first) in [(0, 0.0), (3, 0.0), (4, 0.0), (5, 1.0), (9, 5.0)] {
            let axis = grid().elapsed_secs(start, start + Duration::from_secs(secs));
            assert_eq!(
                axis.first().copied(),
                Some(expected_first),
                "at {secs} s the window should begin at {expected_first} s"
            );
        }
    }

    #[test]
    fn the_axis_steps_on_slot_boundaries_rather_than_sliding() {
        // Three-second slots must advance the picture once every three seconds, not drift
        // every time an emission happens — that drift is what the pointer has to chase.
        let start = Instant::now();
        let grid = Grid::new(Duration::from_secs(3), 5).unwrap();
        let full = start + Duration::from_secs(12);

        let at_twelve = grid.elapsed_secs(start, full);
        for offset in [0, 500, 1_500, 2_900] {
            assert_eq!(
                grid.elapsed_secs(start, full + Duration::from_millis(offset)),
                at_twelve,
                "the axis must not move part-way through a slot"
            );
        }
        assert_ne!(
            grid.elapsed_secs(start, full + Duration::from_secs(3)),
            at_twelve,
            "and it must move when the boundary is crossed"
        );
    }

    #[test]
    fn a_single_point_grid_is_the_present_alone() {
        let start = Instant::now();
        let grid = Grid::new(Duration::from_secs(1), 1).unwrap();
        assert_eq!(grid.elapsed_secs(start, start), vec![0.0]);
        assert_eq!(grid.span(), Duration::ZERO);
    }

    #[test]
    fn samples_land_on_the_slot_they_belong_to() {
        let start = Instant::now();
        let history = history(
            &[(0, Some(1_000)), (1, Some(2_000)), (2, Some(3_000))],
            start,
        );

        let placed = grid().place(&history, start, start + Duration::from_secs(2));

        assert_eq!(placed, vec![Some(1.0), Some(2.0), Some(3.0), None, None]);
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

        let placed = grid().place(&history, start, start + Duration::from_secs(4));

        assert_eq!(placed, vec![Some(1.0), None, Some(3.0), None, Some(5.0)]);
    }

    #[test]
    fn a_probe_that_did_not_come_back_leaves_its_slot_empty() {
        let start = Instant::now();
        let history = history(&[(0, Some(1_000)), (1, None), (2, Some(3_000))], start);

        let placed = grid().place(&history, start, start + Duration::from_secs(2));

        assert_eq!(placed, vec![Some(1.0), None, Some(3.0), None, None]);
    }

    #[test]
    fn samples_older_than_the_window_are_dropped() {
        // Once the window has filled it scrolls, and what falls off the left is gone.
        let start = Instant::now();
        let history = history(&[(0, Some(1_000)), (9, Some(9_000))], start);

        let placed = grid().place(&history, start, start + Duration::from_secs(9));

        assert_eq!(placed, vec![None, None, None, None, Some(9.0)]);
    }

    #[test]
    fn a_slot_shows_its_slowest_sample_whichever_order_they_arrived_in() {
        // Never an average, and never merely the last: smoothing or dropping would remove
        // the spike the chart was opened to find. The statistics beside it are computed
        // over every sample regardless, so the typical figure is not lost either.
        let start = Instant::now();
        for order in [[1_000, 9_000], [9_000, 1_000]] {
            let mut history = SampleHistory::new(8).unwrap();
            for (step, micros) in order.iter().enumerate() {
                history.record(ProbeSample::new(
                    start + Duration::from_millis(100 * step as u64),
                    ProbeOutcome::Success(Rtt::from_micros(*micros)),
                ));
            }
            assert_eq!(grid().place(&history, start, start)[0], Some(9.0));
        }
    }

    #[test]
    fn a_failure_beside_a_success_leaves_the_success_standing() {
        // A slot is empty only when nothing in it answered. One lost packet among several
        // is loss, which the row beside the chart reports as a percentage — it is not an
        // outage, and drawing it as a break would say it was.
        let start = Instant::now();
        let mut history = SampleHistory::new(8).unwrap();
        history.record(ProbeSample::new(start, ProbeOutcome::Timeout));
        history.record(ProbeSample::new(
            start + Duration::from_millis(100),
            ProbeOutcome::Success(Rtt::from_micros(5_000)),
        ));

        assert_eq!(grid().place(&history, start, start)[0], Some(5.0));
    }

    #[test]
    fn a_sample_belongs_to_the_slot_it_falls_inside() {
        // Whole steps from the anchor, so the boundaries are the same for every endpoint of
        // an application and do not move between emissions.
        let start = Instant::now();
        let history = history(&[(0, Some(1_000))], start);
        let placed = grid().place(&history, start, start + Duration::from_secs(2));
        assert_eq!(placed, vec![Some(1.0), None, None, None, None]);

        let mut late = SampleHistory::new(8).unwrap();
        late.record(ProbeSample::new(
            start + Duration::from_millis(1_900),
            ProbeOutcome::Success(Rtt::from_micros(1_000)),
        ));
        assert_eq!(
            grid().place(&late, start, start + Duration::from_secs(2)),
            vec![None, Some(1.0), None, None, None],
            "1.9 s is still inside the second slot, not rounded up into the third"
        );
    }

    #[test]
    fn a_sample_from_the_future_lands_in_the_present() {
        // A clock adjustment can stamp a sample ahead of `now`; it must not index off the
        // end of the array or be silently discarded.
        let start = Instant::now();
        let history = history(&[(5, Some(1_000))], start);

        let placed = grid().place(&history, start, start);

        assert_eq!(placed, vec![Some(1.0), None, None, None, None]);
    }

    #[test]
    fn a_sample_from_before_monitoring_began_is_dropped() {
        // The anchor is where monitoring started; nothing can be drawn to the left of it.
        let earlier = Instant::now();
        let start = earlier + Duration::from_secs(5);
        let history = history(&[(0, Some(1_000))], earlier);

        let placed = grid().place(&history, start, start + Duration::from_secs(20));

        assert!(placed.iter().all(Option::is_none));
    }

    #[test]
    fn an_empty_history_is_all_gaps() {
        let now = Instant::now();
        let history = SampleHistory::new(4).unwrap();
        assert_eq!(grid().place(&history, now, now), vec![None; 5]);
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

        let placed = grid.place(&history, start, start + Duration::from_secs(20));

        assert_eq!(placed.iter().filter(|value| value.is_some()).count(), 3);
        assert_eq!(placed[0], Some(1.0));
        assert_eq!(placed[10], Some(2.0));
        assert_eq!(placed[20], Some(3.0));
    }
}
