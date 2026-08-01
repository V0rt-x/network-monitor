//! Passive metrics from the operating system's own flow events.
//!
//! Every other measurement in this crate describes a probe *we* sent. This one describes
//! the traffic the application itself exchanged, counted by the operating system and
//! handed to us as events. It is the only signal that measures the user's actual game
//! traffic rather than a substitute for it, and it costs nothing to collect: the events
//! are already being delivered for endpoint discovery.
//!
//! It is also the only thing that says anything at all about a game's match server, which
//! answers no echo, no connection and no hello — see [`crate::edge`] for the other half of
//! that answer, the route. **The two must never be merged into one figure called "ping".**
//! They measure different things: the route is a round trip to a router short of the
//! server, and what is here is the arrival pattern of the server's own traffic. Their
//! disagreement is the diagnosis — a clean route with ragged arrivals is the server's
//! problem, not the user's.
//!
//! # What these figures are not
//!
//! * **Not a round-trip time.** Nothing here times a request against its answer. The
//!   operating system reports that datagrams arrived, not what they were replying to.
//! * **Not purely the network's doing.** [`ArrivalStats`] folds in the server's own send
//!   cadence: a server that skips a tick looks exactly like a network that delayed one.
//!   That is a feature rather than a flaw — combined with a clean route it is precisely
//!   what points at a server-side problem — but it forbids calling the figure "jitter on
//!   the path".
//! * **Not loss.** A datagram that never arrives is invisible to us: only the far end
//!   knows what it sent. [`FlowReading::receive_shortfall_pct`] is the nearest honest
//!   thing — a fall in what comes back while what we send holds steady — and it is stated
//!   as that rather than as a loss percentage.
//!
//! # Two clocks, deliberately not mixed
//!
//! Flow events reach this process in buffered batches: measured on a live game, half of
//! them arrived about half a second after the moment they describe, and the slowest a
//! whole second (`docs/flow-metrics-spike.md`). Timing intervals with our own clock at
//! delivery would therefore measure the tracing facility's buffer flush, not the traffic.
//!
//! So everything here runs on [`FlowInstant`], the stamp the operating system put on the
//! event itself, and never on [`std::time::Instant`]. The type is separate precisely so
//! the two cannot be added by accident. The cost is that this module cannot tell how long
//! ago anything happened — a flow that stopped an hour ago still has a last observation —
//! and answering that is the caller's job, with a clock of its own.

use std::time::Duration;

use crate::ring::RingBuffer;
use crate::Error;

/// The RFC 3550 smoothing factor, as used for probe jitter in [`crate::stats`].
const JITTER_SMOOTHING: f64 = 16.0;

/// How much of its own earlier send rate the application must still be keeping up before a
/// fall in what comes back is attributed to the far end.
///
/// The whole claim of [`FlowReading::receive_shortfall_pct`] rests on "we know exactly what
/// we sent", so it has to be abandoned the moment our own sending changes. Deliberately
/// strict: a game closing, a match ending or a player alt-tabbing all reduce the outgoing
/// rate, and every one of them would otherwise show up as the server dropping traffic.
const SENDS_HOLDING_FRACTION: f64 = 0.8;

/// The smallest shortfall worth reporting, as a percentage.
///
/// Below this the figure is the quarter boundary, not a finding. The comparison counts
/// whole datagrams either side of a line drawn through the window, so at the twenty updates
/// a second a real game produces the recent quarter holds about fifty of them and a single
/// one landing on the wrong side of the line is already two per cent. Found by running the
/// build against a live match, where a perfectly healthy endpoint reported a flickering
/// "0.7 %" — true, meaningless, and exactly the sort of number that makes a user go looking
/// for a fault that is not there.
const MIN_REPORTED_SHORTFALL_PCT: f64 = 5.0;

/// How many observations of one endpoint are retained.
///
/// A busy game endpoint produces about forty events a second — twenty datagrams each way —
/// so this holds roughly thirteen seconds, comfortably more than the default window. At
/// the product's ceiling of five applications × sixteen probed endpoints it is under a
/// megabyte in total, and it never grows with uptime.
///
/// A faster stream fills it sooner and the window then covers less time than it asks for;
/// [`FlowReading::span`] says how much, so a figure is never quoted over a period it does
/// not cover.
pub const DEFAULT_CAPACITY: usize = 512;

/// A moment on the operating system's event clock.
///
/// Deliberately not [`std::time::Instant`]: these come from the operating system's own
/// stamp on a flow event, taken when the kernel wrote it rather than when this process
/// received it, and mixing the two would silently fold the tracing facility's buffering
/// delay into every interval. Only differences are meaningful; the origin is unspecified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowInstant(Duration);

impl FlowInstant {
    /// A moment `since` after the clock's unspecified origin.
    #[must_use]
    pub const fn from_origin(since: Duration) -> Self {
        Self(since)
    }

    /// How long after the origin this is.
    #[must_use]
    pub const fn since_origin(self) -> Duration {
        self.0
    }

    /// How long after `earlier` this is, or zero if it is not later.
    ///
    /// Saturating rather than signed: events can in principle be delivered out of order,
    /// and a negative interval is not a measurement — it is an ordering artefact, and
    /// treating it as zero folds it into the burst it belongs with.
    #[must_use]
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }

    /// This moment `offset` later.
    #[must_use]
    pub fn checked_add(self, offset: Duration) -> Option<Self> {
        self.0.checked_add(offset).map(Self)
    }

    /// This moment `offset` earlier, or the origin.
    #[must_use]
    pub fn saturating_sub(self, offset: Duration) -> Self {
        Self(self.0.saturating_sub(offset))
    }
}

/// Which way an observation went.
///
/// Declared here rather than reused from the platform layer: `nm-core` depends on no
/// operating system, and the mapping is one line where the two meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowDirection {
    /// The application sent them.
    Sent,
    /// The application received them.
    Received,
}

/// One flow event, as the passive metrics see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowObservation {
    /// When the operating system says it happened.
    pub at: FlowInstant,
    /// Which way it went.
    pub direction: FlowDirection,
    /// How many bytes it accounts for.
    pub bytes: u32,
}

impl FlowObservation {
    /// An observation of bytes sent.
    #[must_use]
    pub const fn sent(at: FlowInstant, bytes: u32) -> Self {
        Self {
            at,
            direction: FlowDirection::Sent,
            bytes,
        }
    }

    /// An observation of bytes received.
    #[must_use]
    pub const fn received(at: FlowInstant, bytes: u32) -> Self {
        Self {
            at,
            direction: FlowDirection::Received,
            bytes,
        }
    }
}

/// Where the lines are drawn for the passive figures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowPolicy {
    /// How much of the recent past every figure covers.
    pub window: Duration,
    /// Arrivals closer together than this belong to the same burst.
    ///
    /// **Load bearing, and measured rather than guessed.** A server does not always answer
    /// a tick with one datagram: on a live match a third of consecutive arrivals came less
    /// than a millisecond apart, in pairs, with the usual fifty-millisecond gap between the
    /// pairs. Timing raw events would then report enormous "jitter" for a stream arriving
    /// perfectly regularly, purely because each tick was split in two. Coalescing first
    /// measures what the player actually feels: the interval between one update and the
    /// next.
    pub burst_gap: Duration,
    /// A receive gap longer than this, while sending continues, is a stall.
    pub stall_after: Duration,
    /// How many bursts a window needs before any arrival figure is quoted.
    ///
    /// Below this the spread of two or three intervals says nothing, and quoting it would
    /// make an endpoint that has just appeared look unstable.
    pub min_arrivals: usize,
}

impl Default for FlowPolicy {
    /// Defaults chosen against a live match, not from taste.
    ///
    /// Ten seconds holds two hundred arrivals at the twenty updates a second a real game
    /// produced — enough for a percentile to mean something, short enough that the figure
    /// still describes now. Five milliseconds is two orders of magnitude clear of both
    /// clusters that stream showed (the sub-millisecond pairs and the fifty-millisecond
    /// cadence), so the coalescing rule never has to make a close call. Half a second of
    /// silence is ten missed updates at that cadence: unmistakable, and short enough to
    /// see a freeze the player is still in.
    fn default() -> Self {
        Self {
            window: Duration::from_secs(10),
            burst_gap: Duration::from_millis(5),
            stall_after: Duration::from_millis(500),
            min_arrivals: 8,
        }
    }
}

impl FlowPolicy {
    /// Checks the policy can produce a figure at all.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroFlowWindow`] for a window of zero, over which every rate is a
    /// division by nothing.
    pub const fn validate(&self) -> Result<(), Error> {
        if self.window.is_zero() {
            return Err(Error::ZeroFlowWindow);
        }
        Ok(())
    }
}

/// How much crossed an endpoint in one direction.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DirectionStats {
    /// Events counted.
    pub events: usize,
    /// Bytes they accounted for.
    pub bytes: u64,
    /// Events per second over the span the reading covers.
    pub events_per_sec: f64,
    /// Bytes per second over the span the reading covers.
    pub bytes_per_sec: f64,
}

/// How regularly the far end's traffic arrived.
///
/// **Not a round-trip time and not path jitter.** It is the spread of the intervals
/// between one arrival and the next, which folds in whatever cadence the server chose. A
/// server that stutters and a network that delays are indistinguishable here; separating
/// them is what the route figure beside it is for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrivalStats {
    /// How many arrivals the intervals were measured between.
    pub bursts: usize,
    /// How many events those arrivals were made of.
    ///
    /// More than [`ArrivalStats::bursts`] means the far end answers a tick with several
    /// datagrams; the difference is worth showing rather than hiding, because it is the
    /// reason the raw event rate and the update rate disagree.
    pub events: usize,
    /// Mean interval between arrivals, in milliseconds.
    pub mean_ms: f64,
    /// RFC 3550 mean deviation of those intervals, in milliseconds.
    ///
    /// The same smoothing the probe jitter uses, so the two figures are comparable in kind
    /// even though they measure different things.
    pub jitter_ms: f64,
    /// 95th percentile interval, in milliseconds.
    pub p95_ms: f64,
    /// Longest interval in the window, in milliseconds.
    ///
    /// The figure a player recognises: the worst hitch in the last ten seconds.
    pub max_ms: f64,
}

/// What one endpoint's own traffic says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowReading {
    /// How much time the figures actually cover.
    ///
    /// Normally the policy's window; less when the history does not reach that far back,
    /// either because the endpoint is new or because a very fast stream filled the ring.
    /// Quoted so a rate is never presented over a period it does not cover.
    pub span: Duration,
    /// What the application sent.
    pub sent: DirectionStats,
    /// What came back.
    pub received: DirectionStats,
    /// How regularly it came back, once there is enough of it to say.
    pub arrival: Option<ArrivalStats>,
    /// How long nothing has come back while sending continued.
    ///
    /// A one-way outage, seen without sending a single probe of our own. [`None`] when
    /// traffic is flowing both ways, when the application has stopped sending — a silence
    /// of ours explains a silence of theirs — or when the gap is still within
    /// [`FlowPolicy::stall_after`].
    pub stall: Option<Duration>,
    /// How far what comes back has fallen behind what the endpoint's own recent past
    /// promised, as a percentage.
    ///
    /// The honest form of "rate asymmetry": we know exactly what the application sent, so
    /// a fall in the ratio of received to sent — while sending holds up — is loss or a
    /// stall on the far side. Compared against the endpoint's own earlier behaviour rather
    /// than against any assumption of symmetry, because no protocol owes us one datagram
    /// back per datagram sent.
    ///
    /// [`None`] whenever it could not be said honestly: too little history, the
    /// application's own sending fell away too — in which case the drop is ours and blaming
    /// the far end would be a fabrication — or the difference is small enough to be the
    /// arithmetic of the comparison rather than a finding.
    pub receive_shortfall_pct: Option<f64>,
}

impl FlowReading {
    /// Whether any traffic at all was seen in the window.
    ///
    /// The evidence behind [`crate::health::with_passive_evidence`]: bytes the operating
    /// system counted cannot be faked, and they are what tells a silent-but-working game
    /// server apart from a dead host.
    #[must_use]
    pub const fn is_carrying_traffic(&self) -> bool {
        self.sent.events > 0 || self.received.events > 0
    }
}

/// One endpoint's flow history, and the figures it supports.
///
/// Recording is O(1) and allocates nothing — it happens once per datagram on a tracing
/// callback that must not block. The figures are computed on demand, at the rate the UI
/// is emitted, which at the product's ceiling is a few thousand additions a second.
#[derive(Debug, Clone)]
pub struct FlowMetrics {
    observations: RingBuffer<FlowObservation>,
    policy: FlowPolicy,
}

impl FlowMetrics {
    /// Creates a history holding at most `capacity` observations.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroCapacity`] for a capacity of zero, and
    /// [`Error::ZeroFlowWindow`] for a window of zero.
    pub fn new(policy: FlowPolicy, capacity: usize) -> Result<Self, Error> {
        policy.validate()?;
        Ok(Self {
            observations: RingBuffer::new(capacity)?,
            policy,
        })
    }

    /// Creates a history with [`DEFAULT_CAPACITY`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroFlowWindow`] for a window of zero.
    pub fn with_policy(policy: FlowPolicy) -> Result<Self, Error> {
        Self::new(policy, DEFAULT_CAPACITY)
    }

    /// Records one flow event.
    pub fn record(&mut self, observation: FlowObservation) {
        self.observations.push(observation);
    }

    /// The policy in force.
    #[must_use]
    pub const fn policy(&self) -> FlowPolicy {
        self.policy
    }

    /// When the most recent event happened, on the operating system's clock.
    #[must_use]
    pub fn latest(&self) -> Option<FlowInstant> {
        self.observations.iter().next_back().map(|last| last.at)
    }

    /// Whether anything has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    /// What the window ending at the last observation says.
    ///
    /// The window ends at the last event rather than at "now" because this module has no
    /// clock, and because the two are not the same instant — events arrive up to a buffer
    /// flush late. Whether the reading is *current* is the caller's question, answered with
    /// the caller's own clock; see the module documentation.
    ///
    /// [`None`] when nothing has been recorded at all.
    #[must_use]
    pub fn reading(&self) -> Option<FlowReading> {
        let last = self.latest()?;
        let cutoff = last.saturating_sub(self.policy.window);

        let mut first_in_window: Option<FlowInstant> = None;
        let mut sent = Counter::default();
        let mut received = Counter::default();
        // Burst coalescing, done here rather than at record time so the rule stays a
        // property of the reading and can be tested on its own.
        let mut previous_arrival: Option<FlowInstant> = None;
        let mut burst_started: Option<FlowInstant> = None;
        let mut intervals: Vec<f64> = Vec::new();
        let mut burst_events = 0_usize;

        for observation in self.observations.iter().filter(|held| held.at >= cutoff) {
            first_in_window.get_or_insert(observation.at);
            match observation.direction {
                FlowDirection::Sent => sent.add(observation.bytes),
                FlowDirection::Received => {
                    received.add(observation.bytes);
                    burst_events += 1;
                    let gap = previous_arrival
                        .map(|previous| observation.at.saturating_duration_since(previous));
                    previous_arrival = Some(observation.at);
                    match (gap, burst_started) {
                        // Close enough to the one before to be part of the same update.
                        (Some(gap), Some(_)) if gap <= self.policy.burst_gap => {}
                        (Some(_), Some(start)) => {
                            intervals.push(millis(observation.at.saturating_duration_since(start)));
                            burst_started = Some(observation.at);
                        }
                        _ => burst_started = Some(observation.at),
                    }
                }
            }
        }

        let span = first_in_window
            .map(|first| last.saturating_duration_since(first))
            .unwrap_or_default();

        Some(FlowReading {
            span,
            sent: sent.finish(span),
            received: received.finish(span),
            arrival: arrival_stats(&mut intervals, burst_events, self.policy.min_arrivals),
            stall: self.stall(last, cutoff),
            receive_shortfall_pct: self.receive_shortfall_pct(last, cutoff),
        })
    }

    /// How long nothing has come back while the application kept sending.
    fn stall(&self, last: FlowInstant, cutoff: FlowInstant) -> Option<Duration> {
        let mut last_sent: Option<FlowInstant> = None;
        let mut last_received: Option<FlowInstant> = None;
        let mut first: Option<FlowInstant> = None;
        for observation in self.observations.iter().filter(|held| held.at >= cutoff) {
            first.get_or_insert(observation.at);
            match observation.direction {
                FlowDirection::Sent => last_sent = Some(observation.at),
                FlowDirection::Received => last_received = Some(observation.at),
            }
        }

        // No send since the last arrival means our own side has gone quiet, and a silence
        // of ours is no evidence about theirs. Refusing to call that a stall is the whole
        // difference between a diagnosis and a guess.
        let last_sent = last_sent?;
        // Nothing has ever come back in this window: the reference is the window's own
        // start, which is the most that can be claimed.
        let since = last_received.or(first)?;
        if last_sent <= since {
            return None;
        }
        let gap = last.saturating_duration_since(since);
        (gap >= self.policy.stall_after).then_some(gap)
    }

    /// How far the return traffic has fallen behind what this endpoint itself established.
    ///
    /// The window is split at three quarters: the last quarter is *now*, the three before
    /// it are what this endpoint normally does. Comparing an endpoint against itself is the
    /// only honest baseline — no protocol owes one datagram back per datagram sent, so a
    /// ratio of anything but 1 means nothing on its own, while a *change* in that ratio
    /// means something.
    fn receive_shortfall_pct(&self, last: FlowInstant, cutoff: FlowInstant) -> Option<f64> {
        let window = last.saturating_duration_since(cutoff);
        let recent_from = last.saturating_sub(window / 4);
        if recent_from <= cutoff {
            return None;
        }

        let (mut earlier_sent, mut earlier_received) = (0_usize, 0_usize);
        let (mut recent_sent, mut recent_received) = (0_usize, 0_usize);
        for observation in self.observations.iter().filter(|held| held.at >= cutoff) {
            let recent = observation.at >= recent_from;
            match (observation.direction, recent) {
                (FlowDirection::Sent, false) => earlier_sent += 1,
                (FlowDirection::Sent, true) => recent_sent += 1,
                (FlowDirection::Received, false) => earlier_received += 1,
                (FlowDirection::Received, true) => recent_received += 1,
            }
        }

        // Too little of either period to compare. The thresholds are deliberately blunt:
        // this figure exists to catch a collapse, not to resolve a few percent.
        if earlier_sent < self.policy.min_arrivals || earlier_received < self.policy.min_arrivals {
            return None;
        }
        if recent_sent < 2 {
            return None;
        }

        let earlier_ratio = ratio(earlier_received, earlier_sent);
        let recent_ratio = ratio(recent_received, recent_sent);
        if earlier_ratio <= 0.0 {
            return None;
        }

        // The application's own sending must have held up. If it fell away, what comes
        // back falling away with it is the expected consequence, and reporting it as the
        // far end's failure would be a fabrication.
        let recent_rate = per_second(recent_sent, window / 4);
        let earlier_rate = per_second(earlier_sent, window.saturating_sub(window / 4));
        if recent_rate < earlier_rate * SENDS_HOLDING_FRACTION {
            return None;
        }

        let shortfall = (1.0 - recent_ratio / earlier_ratio) * 100.0;
        (shortfall >= MIN_REPORTED_SHORTFALL_PCT).then_some(shortfall.min(100.0))
    }
}

/// Running totals for one direction.
#[derive(Default)]
struct Counter {
    events: usize,
    bytes: u64,
}

impl Counter {
    fn add(&mut self, bytes: u32) {
        self.events += 1;
        self.bytes = self.bytes.saturating_add(u64::from(bytes));
    }

    fn finish(self, span: Duration) -> DirectionStats {
        let secs = span.as_secs_f64();
        // A span of zero is one observation, which supports no rate at all — reporting the
        // count divided by nothing would be an infinity, and reporting zero would be a lie.
        let per_sec = |value: f64| if secs > 0.0 { value / secs } else { 0.0 };
        // Counts are bounded by the ring's capacity and byte totals by what a window can
        // carry; both convert to f64 without loss at any figure this can reach.
        #[allow(clippy::cast_precision_loss)]
        let events = self.events as f64;
        #[allow(clippy::cast_precision_loss)]
        let bytes = self.bytes as f64;
        DirectionStats {
            events: self.events,
            bytes: self.bytes,
            events_per_sec: per_sec(events),
            bytes_per_sec: per_sec(bytes),
        }
    }
}

/// Statistics over the intervals between arrivals, or [`None`] with too few to mean
/// anything.
///
/// Takes the intervals by mutable reference because jitter needs them in order and the
/// percentile needs them sorted; sorting in place avoids a second allocation, exactly as
/// [`crate::stats`] does.
fn arrival_stats(intervals: &mut [f64], events: usize, minimum: usize) -> Option<ArrivalStats> {
    // One more arrival than there are intervals between them.
    let bursts = intervals.len() + 1;
    if intervals.is_empty() || bursts < minimum.max(2) {
        return None;
    }

    let mut jitter = 0.0_f64;
    let mut previous: Option<f64> = None;
    for interval in intervals.iter() {
        if let Some(previous) = previous {
            jitter += ((interval - previous).abs() - jitter) / JITTER_SMOOTHING;
        }
        previous = Some(*interval);
    }

    #[allow(clippy::cast_precision_loss)]
    let count = intervals.len() as f64;
    let mean_ms = intervals.iter().sum::<f64>() / count;

    intervals.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let max_ms = *intervals.last()?;
    // Nearest rank, as in `crate::stats`, so the boundary does not depend on rounding.
    let rank = (95 * intervals.len()).div_ceil(100).max(1);
    let p95_ms = *intervals.get(rank - 1)?;

    Some(ArrivalStats {
        bursts,
        events,
        mean_ms,
        jitter_ms: jitter,
        p95_ms,
        max_ms,
    })
}

/// A duration in milliseconds.
fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

/// A count over a span, per second, or zero over no span at all.
fn per_second(count: usize, span: Duration) -> f64 {
    let secs = span.as_secs_f64();
    if secs <= 0.0 {
        return 0.0;
    }
    // Counts are bounded by the ring's capacity, so this converts exactly.
    #[allow(clippy::cast_precision_loss)]
    let count = count as f64;
    count / secs
}

/// One count over another, as a float.
fn ratio(numerator: usize, denominator: usize) -> f64 {
    // Counts are bounded by the ring's capacity, so both convert exactly.
    #[allow(clippy::cast_precision_loss)]
    let (numerator, denominator) = (numerator as f64, denominator as f64);
    if denominator == 0.0 {
        return 0.0;
    }
    numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Comparison tolerance for figures that go through a division.
    const EPSILON: f64 = 1e-6;

    fn at(millis: u64) -> FlowInstant {
        FlowInstant::from_origin(Duration::from_millis(millis))
    }

    fn metrics() -> FlowMetrics {
        FlowMetrics::with_policy(FlowPolicy::default()).expect("the default policy is valid")
    }

    /// A steady exchange: `count` updates each way, `period_ms` apart.
    fn steady(period_ms: u64, count: u64) -> FlowMetrics {
        let mut flow = metrics();
        for index in 0..count {
            let moment = index * period_ms;
            flow.record(FlowObservation::sent(at(moment), 64));
            flow.record(FlowObservation::received(at(moment + 1), 256));
        }
        flow
    }

    #[test]
    fn nothing_recorded_reports_nothing() {
        let flow = metrics();
        assert!(flow.is_empty());
        assert_eq!(flow.latest(), None);
        assert_eq!(
            flow.reading(),
            None,
            "an empty window must not invent zeros"
        );
    }

    #[test]
    fn a_zero_window_is_refused_at_construction() {
        let policy = FlowPolicy {
            window: Duration::ZERO,
            ..FlowPolicy::default()
        };
        assert_eq!(
            FlowMetrics::with_policy(policy).unwrap_err(),
            Error::ZeroFlowWindow
        );
    }

    #[test]
    fn a_zero_capacity_is_refused_at_construction() {
        assert_eq!(
            FlowMetrics::new(FlowPolicy::default(), 0).unwrap_err(),
            Error::ZeroCapacity
        );
    }

    #[test]
    fn a_single_observation_supports_counts_but_no_rate_and_no_jitter() {
        let mut flow = metrics();
        flow.record(FlowObservation::received(at(0), 256));
        let reading = flow.reading().expect("one observation is still a reading");

        assert_eq!(reading.received.events, 1);
        assert_eq!(reading.received.bytes, 256);
        assert!(
            (reading.received.bytes_per_sec - 0.0).abs() < EPSILON,
            "one observation spans no time, so it supports no rate"
        );
        assert_eq!(
            reading.arrival, None,
            "a single arrival has no interval to measure"
        );
        assert!(reading.is_carrying_traffic());
    }

    // --- arrival timing ------------------------------------------------------------

    #[test]
    fn a_perfectly_regular_stream_has_no_jitter() {
        let reading = steady(50, 100).reading().expect("a reading");
        let arrival = reading.arrival.expect("a hundred arrivals is plenty");

        assert!((arrival.mean_ms - 50.0).abs() < EPSILON);
        assert!(
            arrival.jitter_ms.abs() < EPSILON,
            "identical intervals are zero deviation, not a small one"
        );
        assert!((arrival.max_ms - 50.0).abs() < EPSILON);
    }

    #[test]
    fn a_late_update_shows_up_as_the_worst_interval() {
        let mut flow = metrics();
        // Twenty updates a second, with one arriving 120 ms late — the shape a live match
        // produced at the 99th percentile.
        let mut moment = 0;
        for index in 0..40_u64 {
            flow.record(FlowObservation::sent(at(moment), 64));
            flow.record(FlowObservation::received(at(moment + 1), 256));
            moment += if index == 20 { 120 } else { 50 };
        }
        let arrival = flow
            .reading()
            .expect("a reading")
            .arrival
            .expect("arrivals");

        assert!((arrival.max_ms - 120.0).abs() < EPSILON);
        assert!(
            arrival.jitter_ms > 0.0,
            "one late update must move the deviation off zero"
        );
        assert!(
            arrival.mean_ms > 50.0 && arrival.mean_ms < 55.0,
            "one hitch must not drag the mean away from the cadence: {}",
            arrival.mean_ms
        );
    }

    #[test]
    fn datagrams_of_one_update_are_one_arrival() {
        // Measured on a live match: a third of consecutive arrivals came under a
        // millisecond apart, in pairs, with the usual cadence between the pairs. Timed
        // raw, that stream reports enormous jitter while arriving perfectly regularly.
        let mut flow = metrics();
        for index in 0..40_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::received(at(moment), 256));
            flow.record(FlowObservation::received(at(moment + 1), 256));
        }
        let arrival = flow
            .reading()
            .expect("a reading")
            .arrival
            .expect("arrivals");

        assert_eq!(arrival.bursts, 40, "each pair is one update");
        assert_eq!(arrival.events, 80, "and it is made of two datagrams");
        assert!((arrival.mean_ms - 50.0).abs() < EPSILON);
        assert!(
            arrival.jitter_ms.abs() < EPSILON,
            "a split update is not a jittery one"
        );
    }

    #[test]
    fn a_gap_wider_than_the_burst_rule_is_its_own_arrival() {
        let mut flow = metrics();
        for index in 0..40_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::received(at(moment), 256));
            // Six milliseconds apart — past the five-millisecond rule.
            flow.record(FlowObservation::received(at(moment + 6), 256));
        }
        let arrival = flow
            .reading()
            .expect("a reading")
            .arrival
            .expect("arrivals");

        assert_eq!(arrival.bursts, 80, "these are separate arrivals");
        assert!(
            arrival.jitter_ms > 0.0,
            "alternating 6 ms and 44 ms is genuinely irregular"
        );
    }

    #[test]
    fn too_few_arrivals_claim_nothing() {
        let mut flow = metrics();
        for index in 0..4_u64 {
            flow.record(FlowObservation::received(at(index * 50), 256));
        }
        assert_eq!(
            flow.reading().expect("a reading").arrival,
            None,
            "four arrivals cannot support a spread"
        );
    }

    #[test]
    fn arrivals_outside_the_window_are_not_counted() {
        let mut flow = metrics();
        // An old burst, then a gap far longer than the ten-second window, then a new one.
        for index in 0..20_u64 {
            flow.record(FlowObservation::received(at(index * 50), 256));
        }
        for index in 0..20_u64 {
            flow.record(FlowObservation::received(at(60_000 + index * 50), 256));
        }
        let reading = flow.reading().expect("a reading");

        assert_eq!(
            reading.received.events, 20,
            "the older burst is outside the window"
        );
        assert!(
            reading.span <= Duration::from_secs(10),
            "the span must never exceed the window"
        );
    }

    #[test]
    fn a_full_ring_reports_the_span_it_actually_covers() {
        // Capacity below what the window would hold: the reading must say so rather than
        // quote a rate over ten seconds it has four of.
        let mut flow =
            FlowMetrics::new(FlowPolicy::default(), 16).expect("sixteen is a valid capacity");
        for index in 0..200_u64 {
            flow.record(FlowObservation::received(at(index * 50), 256));
        }
        let reading = flow.reading().expect("a reading");

        assert_eq!(reading.received.events, 16);
        assert!(
            reading.span < Duration::from_secs(1),
            "sixteen arrivals 50 ms apart cover under a second: {:?}",
            reading.span
        );
    }

    // --- rates ---------------------------------------------------------------------

    #[test]
    fn rates_are_counted_over_the_span_the_reading_covers() {
        let reading = steady(50, 100).reading().expect("a reading");

        // The window keeps the last ten seconds of a five-second stream, so all of it.
        assert_eq!(reading.sent.events, 100);
        assert_eq!(reading.received.events, 100);
        assert_eq!(reading.sent.bytes, 6_400);
        assert_eq!(reading.received.bytes, 25_600);
        assert!(
            (reading.received.events_per_sec - 20.0).abs() < 0.5,
            "twenty updates a second: {}",
            reading.received.events_per_sec
        );
    }

    // --- stalls --------------------------------------------------------------------

    #[test]
    fn a_stream_flowing_both_ways_is_not_stalled() {
        assert_eq!(steady(50, 100).reading().expect("a reading").stall, None);
    }

    #[test]
    fn sending_into_silence_is_a_stall() {
        let mut flow = steady(50, 40);
        // The last arrival was at 1951 ms; keep sending for another second.
        for index in 0..20_u64 {
            flow.record(FlowObservation::sent(at(2_000 + index * 50), 64));
        }
        let stall = flow
            .reading()
            .expect("a reading")
            .stall
            .expect("a second of one-way traffic is a stall");

        assert!(
            stall >= Duration::from_millis(900) && stall <= Duration::from_millis(1_100),
            "the stall is measured from the last arrival: {stall:?}"
        );
    }

    #[test]
    fn our_own_silence_is_never_called_a_stall() {
        // The application stopped sending too — it was closed, or the match ended. Nothing
        // about the far end can be inferred from that, and saying "stalled" would blame it
        // for our own silence.
        let mut flow = metrics();
        for index in 0..40_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::sent(at(moment), 64));
            flow.record(FlowObservation::received(at(moment + 1), 256));
        }
        // Nothing at all for a while, then the reading is taken.
        assert_eq!(
            flow.reading().expect("a reading").stall,
            None,
            "the last event was an arrival, so nothing has gone unanswered"
        );
    }

    #[test]
    fn a_gap_shorter_than_the_threshold_is_not_a_stall() {
        let mut flow = steady(50, 40);
        for index in 0..4_u64 {
            flow.record(FlowObservation::sent(at(2_000 + index * 50), 64));
        }
        assert_eq!(
            flow.reading().expect("a reading").stall,
            None,
            "200 ms of silence is four missed updates, not an outage"
        );
    }

    #[test]
    fn sending_with_nothing_ever_coming_back_is_a_stall() {
        let mut flow = metrics();
        for index in 0..40_u64 {
            flow.record(FlowObservation::sent(at(index * 50), 64));
        }
        let reading = flow.reading().expect("a reading");

        assert!(
            reading.stall.is_some(),
            "two seconds of unanswered sending is the clearest stall there is"
        );
        assert_eq!(reading.received.events, 0);
        assert!(reading.is_carrying_traffic(), "sending is still traffic");
    }

    // --- rate asymmetry ------------------------------------------------------------

    #[test]
    fn a_symmetric_stream_reports_no_shortfall() {
        assert_eq!(
            steady(50, 100)
                .reading()
                .expect("a reading")
                .receive_shortfall_pct,
            None
        );
    }

    #[test]
    fn an_asymmetric_but_steady_stream_reports_no_shortfall() {
        // Two arrivals for every send, throughout. Asymmetry is normal; a *change* in it
        // is the signal.
        let mut flow = metrics();
        for index in 0..100_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::sent(at(moment), 64));
            flow.record(FlowObservation::received(at(moment + 10), 256));
            flow.record(FlowObservation::received(at(moment + 30), 256));
        }
        assert_eq!(
            flow.reading().expect("a reading").receive_shortfall_pct,
            None
        );
    }

    #[test]
    fn return_traffic_halving_while_sending_holds_is_a_shortfall() {
        // Ten seconds of twenty updates a second, with half the return traffic stopping
        // for the last quarter of it and the sending unchanged throughout.
        let mut flow = metrics();
        for index in 0..200_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::sent(at(moment), 64));
            if index < 150 || index % 2 == 0 {
                flow.record(FlowObservation::received(at(moment + 10), 256));
            }
        }
        let shortfall = flow
            .reading()
            .expect("a reading")
            .receive_shortfall_pct
            .expect("half the return traffic gone is exactly what this measures");

        assert!(
            (shortfall - 50.0).abs() < 5.0,
            "half the return traffic is a ~50 % shortfall, got {shortfall}"
        );
    }

    #[test]
    fn degradation_only_part_way_into_the_recent_quarter_is_reported_smaller() {
        // The figure compares the last quarter of the window against the rest, so a
        // collapse that began part way through that quarter is diluted by the healthy
        // traffic beside it. That is the honest reading — the quarter really did carry
        // more than the collapse alone — and it grows towards the full figure as the
        // window moves on. Pinned so the dilution is a known property rather than a
        // surprise in a bug report.
        let mut flow = metrics();
        for index in 0..200_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::sent(at(moment), 64));
            if index < 180 || index % 2 == 0 {
                flow.record(FlowObservation::received(at(moment + 10), 256));
            }
        }
        let shortfall = flow
            .reading()
            .expect("a reading")
            .receive_shortfall_pct
            .expect("a collapse in the last second is still visible");

        assert!(
            shortfall > 10.0 && shortfall < 30.0,
            "one second of a ten-second window, halved: {shortfall}"
        );
    }

    #[test]
    fn a_shortfall_is_not_claimed_when_our_own_sending_stopped_too() {
        let mut flow = metrics();
        for index in 0..160_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::sent(at(moment), 64));
            flow.record(FlowObservation::received(at(moment + 10), 256));
        }
        // The application goes nearly quiet: three sends and nothing coming back. The
        // ratio collapsed, but so did our own traffic — and this is the case that made the
        // rule strict. A guard that merely asked for half the previous send rate let a 12 %
        // shortfall through here, because the window's last quarter still held a second of
        // healthy traffic beside the silence.
        for index in 0..3_u64 {
            flow.record(FlowObservation::sent(at(8_000 + index * 600), 64));
        }
        assert_eq!(
            flow.reading().expect("a reading").receive_shortfall_pct,
            None,
            "our silence explains theirs; blaming the far end would be a fabrication"
        );
    }

    #[test]
    fn a_shortfall_too_small_to_mean_anything_is_not_reported() {
        // Found by running the build against a live match: a healthy endpoint reported a
        // flickering "0.7 %". True, and meaningless — the comparison counts whole datagrams
        // either side of a line through the window, so one landing on the wrong side of it
        // is already a couple of per cent. A number that small sends a user looking for a
        // fault that is not there.
        let mut flow = metrics();
        for index in 0..200_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::sent(at(moment), 64));
            // One arrival missing out of the last fifty: about 2 %.
            if index != 175 {
                flow.record(FlowObservation::received(at(moment + 10), 256));
            }
        }
        assert_eq!(
            flow.reading().expect("a reading").receive_shortfall_pct,
            None
        );
    }

    #[test]
    fn too_little_history_claims_no_shortfall() {
        let mut flow = metrics();
        for index in 0..6_u64 {
            let moment = index * 50;
            flow.record(FlowObservation::sent(at(moment), 64));
            flow.record(FlowObservation::received(at(moment + 10), 256));
        }
        assert_eq!(
            flow.reading().expect("a reading").receive_shortfall_pct,
            None
        );
    }

    // --- clock robustness -----------------------------------------------------------

    #[test]
    fn an_out_of_order_event_yields_no_negative_interval() {
        let mut flow = metrics();
        for index in 0..40_u64 {
            flow.record(FlowObservation::received(at(index * 50), 256));
        }
        // One event delivered with an older stamp than its predecessor.
        flow.record(FlowObservation::received(at(1_000), 256));
        let reading = flow.reading().expect("a reading");

        if let Some(arrival) = reading.arrival {
            assert!(arrival.mean_ms >= 0.0);
            assert!(arrival.max_ms >= 0.0);
            assert!(arrival.jitter_ms >= 0.0);
        }
        assert!(reading.span <= Duration::from_secs(10));
    }

    #[test]
    fn the_flow_instant_saturates_rather_than_wrapping() {
        let early = at(100);
        let late = at(500);
        assert_eq!(
            late.saturating_duration_since(early),
            Duration::from_millis(400)
        );
        assert_eq!(
            early.saturating_duration_since(late),
            Duration::ZERO,
            "an ordering artefact is not a negative measurement"
        );
        assert_eq!(at(50).saturating_sub(Duration::from_millis(500)), at(0));
    }
}
