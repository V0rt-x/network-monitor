//! Probe scheduling model.
//!
//! Decides *which* targets to probe *when*, and nothing else: it issues no probes, owns
//! no clock and performs no I/O. Callers pass the current [`Instant`] in, which is what
//! makes an entire day of scheduling reproducible in a millisecond of test time.
//!
//! Two rules define its behaviour:
//!
//! * **The global rate cap is never exceeded.** The budget is a token bucket with a
//!   deliberately small burst allowance, so a hundred simultaneously-due targets trickle
//!   out instead of firing at once — probe traffic must never add jitter to the game
//!   connection it is measuring.
//! * **Nothing is ever silently dropped.** When more targets are due than the budget
//!   allows, the most overdue go first and the rest stay due. Under sustained pressure
//!   every target's *effective* interval stretches, and it stretches fairly, because
//!   waiting is exactly what makes a target win the next round.
//!
//! Priority is expressed as interval length, not as a separate rank: an idle endpoint is
//! demoted by giving it a longer interval. That keeps the model starvation-free by
//! construction — a strict priority order could let a busy high-priority set freeze out
//! everything below it forever.
//!
//! The policy numbers themselves (the 32 probes/s cap, the one-second default interval)
//! live in `nm-probes`, which owns the product's budget; this module only enforces
//! whatever cap it is handed.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::target::TargetId;
use crate::Error;

/// The burst allowance is one `BURST_DIVISOR`th of a second's worth of probes.
///
/// At the product's 32 probes/s cap that is 4 probes, roughly 125 ms of budget — enough
/// that the scheduler is not stalled by its own accounting, small enough that a backlog
/// drains as a trickle rather than a spike.
const BURST_DIVISOR: u32 = 8;

/// When a target should next be probed, and how often.
#[derive(Debug, Clone, Copy)]
struct Schedule {
    interval: Duration,
    next_due: Instant,
}

/// A token bucket over the global probe budget.
///
/// Refilling is exact integer arithmetic: the clock only advances by the time actually
/// converted into whole probes, so fractional remainders carry forward instead of being
/// rounded away into long-term drift.
#[derive(Debug, Clone, Copy)]
struct RateLimiter {
    per_sec: u32,
    burst: u32,
    available: u32,
    last_refill: Instant,
}

impl RateLimiter {
    fn new(per_sec: u32, now: Instant) -> Self {
        let burst = (per_sec / BURST_DIVISOR).max(1);
        Self {
            per_sec,
            burst,
            available: burst,
            last_refill: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        // A clock that appears to move backwards yields zero elapsed time rather than a
        // panic or a windfall of budget.
        let elapsed = now.saturating_duration_since(self.last_refill);
        let earned = elapsed.as_micros() * u128::from(self.per_sec) / 1_000_000;
        if earned == 0 {
            return;
        }

        let earned = u32::try_from(earned).unwrap_or(u32::MAX);
        self.available = self.available.saturating_add(earned).min(self.burst);

        // Advance only by the time those whole probes cost, keeping the remainder.
        let consumed_micros = u128::from(earned) * 1_000_000 / u128::from(self.per_sec);
        let consumed = Duration::from_micros(u64::try_from(consumed_micros).unwrap_or(u64::MAX));
        self.last_refill = self.last_refill.checked_add(consumed).unwrap_or(now);
    }

    fn take(&mut self, now: Instant, wanted: usize) -> usize {
        self.refill(now);
        let available = usize::try_from(self.available).unwrap_or(usize::MAX);
        let granted = wanted.min(available);
        self.available -= u32::try_from(granted).unwrap_or(self.available);
        granted
    }
}

/// Tracks when each registered target is next due, within a global rate cap.
#[derive(Debug, Clone)]
pub struct ProbeScheduler {
    entries: BTreeMap<TargetId, Schedule>,
    rate: RateLimiter,
}

impl ProbeScheduler {
    /// Creates a scheduler limited to `probes_per_sec` across every target it manages.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroProbeRate`] if the cap is zero, which would stall every
    /// target forever.
    pub fn new(probes_per_sec: u32, now: Instant) -> Result<Self, Error> {
        if probes_per_sec == 0 {
            return Err(Error::ZeroProbeRate);
        }
        Ok(Self {
            entries: BTreeMap::new(),
            rate: RateLimiter::new(probes_per_sec, now),
        })
    }

    /// Starts probing `id` every `interval`, or changes the interval of one already
    /// scheduled.
    ///
    /// A newly scheduled target is due immediately, so a freshly discovered endpoint is
    /// measured at once rather than after a blank first interval. Changing an existing
    /// target's interval leaves its pending deadline alone: demoting an idle endpoint
    /// should slow it down from now on, not cancel a probe it has already waited for.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroInterval`] if `interval` is zero, which would make the
    /// target permanently due and starve everything else.
    pub fn schedule(
        &mut self,
        id: TargetId,
        interval: Duration,
        now: Instant,
    ) -> Result<(), Error> {
        if interval.is_zero() {
            return Err(Error::ZeroInterval);
        }
        self.entries
            .entry(id)
            .and_modify(|schedule| schedule.interval = interval)
            .or_insert(Schedule {
                interval,
                next_due: now,
            });
        Ok(())
    }

    /// Schedules `id` with its first probe a full `interval` from `now`.
    ///
    /// The counterpart to [`ProbeScheduler::schedule`] for a target that has just *finished*
    /// a probe rather than just appeared. A driver that unschedules a target while its probe
    /// is in flight — which is how it stops a slow probe from being issued twice — uses this
    /// to put it back, so the interval is measured from the answer rather than from the
    /// request. An expensive probe therefore spaces itself out instead of queueing behind
    /// itself. Any existing deadline for `id` is replaced.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroInterval`] if `interval` is zero.
    pub fn schedule_after(
        &mut self,
        id: TargetId,
        interval: Duration,
        now: Instant,
    ) -> Result<(), Error> {
        if interval.is_zero() {
            return Err(Error::ZeroInterval);
        }
        self.entries.insert(
            id,
            Schedule {
                interval,
                next_due: now.checked_add(interval).unwrap_or(now),
            },
        );
        Ok(())
    }

    /// Stops probing `id`. Returns `true` if it was scheduled.
    pub fn unschedule(&mut self, id: TargetId) -> bool {
        self.entries.remove(&id).is_some()
    }

    /// Whether a target is scheduled.
    #[must_use]
    pub fn contains(&self, id: TargetId) -> bool {
        self.entries.contains_key(&id)
    }

    /// How many targets are scheduled.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is scheduled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The earliest deadline across all targets, so a driver knows how long it may sleep.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.entries
            .values()
            .map(|schedule| schedule.next_due)
            .min()
    }

    /// Fills `out` with the targets to probe right now, most overdue first.
    ///
    /// `out` is cleared first and reused, so a driver calling this in a loop performs no
    /// allocation after the first few ticks. Targets that were due but exceeded the
    /// budget are left due and will win a later round; none are discarded.
    pub fn due(&mut self, now: Instant, out: &mut Vec<TargetId>) {
        out.clear();

        for (id, schedule) in &self.entries {
            if schedule.next_due <= now {
                out.push(*id);
            }
        }
        if out.is_empty() {
            return;
        }

        // Longest-waiting first. The sort is stable and the map yielded handles in
        // order, so equally overdue targets keep a deterministic order.
        let entries = &self.entries;
        out.sort_by_key(|id| entries.get(id).map_or(now, |schedule| schedule.next_due));

        let granted = self.rate.take(now, out.len());
        out.truncate(granted);

        for id in out.iter() {
            if let Some(schedule) = self.entries.get_mut(id) {
                // Deadlines are set forward from *now*, not from the missed deadline:
                // catching up on a backlog would produce exactly the burst the budget
                // exists to prevent.
                schedule.next_due = now.checked_add(schedule.interval).unwrap_or(now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn id(raw: u32) -> TargetId {
        TargetId::from_raw(raw)
    }

    /// A scheduler holding `count` targets, all on the same interval, all due at `start`.
    fn with_targets(
        count: u32,
        interval: Duration,
        cap: u32,
        start: Instant,
    ) -> (ProbeScheduler, Vec<TargetId>) {
        let mut scheduler = ProbeScheduler::new(cap, start).unwrap();
        let ids: Vec<_> = (0..count).map(id).collect();
        for target in &ids {
            scheduler.schedule(*target, interval, start).unwrap();
        }
        (scheduler, ids)
    }

    // --- construction ----------------------------------------------------------------

    #[test]
    fn rejects_a_rate_cap_that_would_stall_everything() {
        assert_eq!(
            ProbeScheduler::new(0, Instant::now()).unwrap_err(),
            Error::ZeroProbeRate
        );
    }

    #[test]
    fn rejects_an_interval_that_would_never_wait() {
        let now = Instant::now();
        let mut scheduler = ProbeScheduler::new(32, now).unwrap();
        assert_eq!(
            scheduler.schedule(id(0), Duration::ZERO, now).unwrap_err(),
            Error::ZeroInterval
        );
        assert!(scheduler.is_empty());
    }

    #[test]
    fn an_empty_scheduler_has_nothing_due_and_no_deadline() {
        let now = Instant::now();
        let mut scheduler = ProbeScheduler::new(32, now).unwrap();
        let mut out = Vec::new();

        scheduler.due(now, &mut out);
        assert!(out.is_empty());
        assert_eq!(scheduler.next_deadline(), None);
    }

    // --- basic cadence ---------------------------------------------------------------

    #[test]
    fn a_new_target_is_due_immediately() {
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(1, SECOND, 32, start);
        let mut out = Vec::new();

        scheduler.due(start, &mut out);
        assert_eq!(out, ids);
        assert_eq!(scheduler.next_deadline(), Some(start + SECOND));
    }

    #[test]
    fn a_target_rescheduled_after_a_probe_waits_a_full_interval() {
        // How a driver stops a slow probe being issued twice: unschedule on dispatch, put it
        // back when the answer arrives. The interval must then run from the answer.
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(1, SECOND, 32, start);
        let mut out = Vec::new();

        scheduler.due(start, &mut out);
        scheduler.unschedule(ids[0]);
        assert!(scheduler.is_empty());

        // The probe took five seconds; the next one is due a second after it finished.
        let finished = start + Duration::from_secs(5);
        scheduler.schedule_after(ids[0], SECOND, finished).unwrap();
        assert_eq!(scheduler.next_deadline(), Some(finished + SECOND));

        scheduler.due(finished, &mut out);
        assert!(out.is_empty(), "not due until the interval has passed");
        scheduler.due(finished + SECOND, &mut out);
        assert_eq!(out, ids);
    }

    #[test]
    fn rescheduling_after_a_probe_replaces_any_pending_deadline() {
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(1, SECOND, 32, start);
        scheduler
            .schedule_after(ids[0], Duration::from_secs(10), start)
            .unwrap();
        assert_eq!(
            scheduler.next_deadline(),
            Some(start + Duration::from_secs(10))
        );
    }

    #[test]
    fn rescheduling_after_a_probe_refuses_a_zero_interval() {
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(1, SECOND, 32, start);
        assert_eq!(
            scheduler
                .schedule_after(ids[0], Duration::ZERO, start)
                .unwrap_err(),
            Error::ZeroInterval
        );
    }

    #[test]
    fn a_probed_target_waits_out_its_interval() {
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(1, SECOND, 32, start);
        let mut out = Vec::new();

        scheduler.due(start, &mut out);
        assert_eq!(out.len(), 1);

        scheduler.due(start + Duration::from_millis(999), &mut out);
        assert!(
            out.is_empty(),
            "not due until the interval has fully elapsed"
        );

        scheduler.due(start + SECOND, &mut out);
        assert_eq!(out, ids);
    }

    #[test]
    fn deadlines_run_forward_from_now_so_a_backlog_never_becomes_a_burst() {
        let start = Instant::now();
        let (mut scheduler, _) = with_targets(1, SECOND, 32, start);
        let mut out = Vec::new();

        // Probe it ten seconds late: it must become due again one interval from now,
        // not fire nine more times to catch up.
        let late = start + Duration::from_secs(10);
        scheduler.due(late, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(scheduler.next_deadline(), Some(late + SECOND));

        scheduler.due(late, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn rescheduling_changes_the_interval_without_cancelling_a_pending_probe() {
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(1, SECOND, 32, start);
        let mut out = Vec::new();

        // Demote to a slow cadence before the first probe happens.
        scheduler
            .schedule(ids[0], Duration::from_secs(60), start)
            .unwrap();
        scheduler.due(start, &mut out);
        assert_eq!(
            out, ids,
            "the probe it was already waiting for still happens"
        );
        assert_eq!(
            scheduler.next_deadline(),
            Some(start + Duration::from_secs(60))
        );
        assert_eq!(
            scheduler.len(),
            1,
            "rescheduling must not duplicate a target"
        );
    }

    #[test]
    fn unscheduling_removes_a_target_from_future_rounds() {
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(2, SECOND, 32, start);
        let mut out = Vec::new();

        assert!(scheduler.unschedule(ids[0]));
        assert!(!scheduler.unschedule(ids[0]));
        assert!(!scheduler.contains(ids[0]));

        scheduler.due(start, &mut out);
        assert_eq!(out, vec![ids[1]]);
    }

    // --- the rate cap ----------------------------------------------------------------

    #[test]
    fn a_backlog_is_released_as_a_trickle_not_a_spike() {
        // 50 targets all due at once against a 32/s cap: the burst allowance is an
        // eighth of a second's budget, so only four go out on the first tick.
        let start = Instant::now();
        let (mut scheduler, _) = with_targets(50, SECOND, 32, start);
        let mut out = Vec::new();

        scheduler.due(start, &mut out);
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn the_cap_holds_over_a_long_run() {
        let start = Instant::now();
        let cap = 32;
        let (mut scheduler, _) = with_targets(100, SECOND, cap, start);
        let mut out = Vec::new();

        let mut probes = 0_usize;
        for tick in 0..100_u64 {
            scheduler.due(start + Duration::from_millis(tick * 100), &mut out);
            probes += out.len();
        }

        // Ten seconds of budget plus the initial burst allowance.
        let ceiling = usize::try_from(cap).unwrap() * 10 + 4;
        assert!(
            probes <= ceiling,
            "issued {probes} probes, ceiling is {ceiling}"
        );
        assert!(
            probes > 300,
            "the budget should be nearly fully used, got {probes}"
        );
    }

    #[test]
    fn oversubscription_stretches_intervals_instead_of_dropping_targets() {
        // 100 targets each wanting a probe every second, against a 32/s cap. Nothing may
        // be starved: the effective interval stretches for everyone, roughly evenly.
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(100, SECOND, 32, start);
        let mut out = Vec::new();
        let mut counts: BTreeMap<TargetId, usize> = ids.iter().map(|id| (*id, 0)).collect();

        for tick in 0..100_u64 {
            scheduler.due(start + Duration::from_millis(tick * 100), &mut out);
            for probed in &out {
                *counts.entry(*probed).or_default() += 1;
            }
        }

        let fewest = counts.values().copied().min().unwrap();
        let most = counts.values().copied().max().unwrap();
        assert!(
            fewest >= 2,
            "a target was starved: only {fewest} probes in 10 s"
        );
        assert!(
            most - fewest <= 1,
            "unfair sharing: between {fewest} and {most} probes per target"
        );
    }

    #[test]
    fn the_most_overdue_target_goes_first() {
        let start = Instant::now();
        let mut scheduler = ProbeScheduler::new(32, start).unwrap();
        let mut out = Vec::new();

        let early = id(0);
        let late = id(1);
        scheduler.schedule(early, SECOND, start).unwrap();
        scheduler
            .schedule(late, SECOND, start + Duration::from_millis(500))
            .unwrap();

        scheduler.due(start + SECOND, &mut out);
        assert_eq!(out, vec![early, late]);
    }

    #[test]
    fn a_target_held_back_by_the_budget_stays_due() {
        let start = Instant::now();
        let (mut scheduler, _) = with_targets(10, SECOND, 8, start);
        let mut out = Vec::new();

        // Burst is max(8/8, 1) = 1, so nine of the ten are held back.
        scheduler.due(start, &mut out);
        assert_eq!(out.len(), 1);
        let first = out[0];

        // A moment later the budget has refilled and the rest are still waiting.
        scheduler.due(start + Duration::from_millis(200), &mut out);
        assert!(!out.is_empty());
        assert!(
            !out.contains(&first),
            "the one already probed is not due again yet"
        );
    }

    // --- clock robustness ------------------------------------------------------------

    #[test]
    fn a_clock_that_moves_backwards_grants_no_budget_and_does_not_panic() {
        let start = Instant::now() + Duration::from_secs(60);
        let (mut scheduler, _) = with_targets(20, SECOND, 32, start);
        let mut out = Vec::new();

        scheduler.due(start, &mut out);
        let burst = out.len();
        assert_eq!(burst, 4);

        // Time appears to jump back a minute: no refill, and the already-probed targets
        // are not due, so nothing goes out.
        let rewound = start.checked_sub(Duration::from_secs(60)).unwrap();
        scheduler.due(rewound, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn an_idle_period_does_not_accumulate_a_giant_burst() {
        let start = Instant::now();
        let (mut scheduler, _) = with_targets(100, SECOND, 32, start);
        let mut out = Vec::new();

        // Drain the initial allowance, then leave the scheduler alone for an hour.
        scheduler.due(start, &mut out);
        scheduler.due(start + Duration::from_secs(3_600), &mut out);

        assert_eq!(
            out.len(),
            4,
            "tokens are capped at the burst allowance, however long the wait"
        );
    }

    // --- resource behaviour ----------------------------------------------------------

    #[test]
    fn the_output_buffer_is_reused_without_reallocating() {
        // The driver calls this several times a second forever; it must not allocate.
        let start = Instant::now();
        let (mut scheduler, _) = with_targets(40, SECOND, 32, start);
        let mut out = Vec::new();

        for tick in 0..20_u64 {
            scheduler.due(start + Duration::from_millis(tick * 100), &mut out);
        }
        let settled = out.capacity();

        for tick in 20..200_u64 {
            scheduler.due(start + Duration::from_millis(tick * 100), &mut out);
            assert_eq!(out.capacity(), settled);
        }
    }

    #[test]
    fn stale_contents_of_the_output_buffer_are_discarded() {
        let start = Instant::now();
        let (mut scheduler, ids) = with_targets(1, SECOND, 32, start);
        let mut out = vec![ids[0]; 99];

        scheduler.due(start + Duration::from_millis(1), &mut out);
        assert_eq!(out, ids);
    }
}
