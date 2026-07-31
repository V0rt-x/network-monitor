//! Measuring the path to an endpoint that answers nothing.
//!
//! A game's match server replies to no echo, accepts no connection and returns no hello —
//! correctly, because nothing listens on a game port but the game. [`crate::path`] can still
//! walk outward and find the deepest router that answers, but one walk is a snapshot: it says
//! where the path reached a second ago and nothing about how it is behaving now.
//!
//! This module turns that snapshot into a measurement. It picks the deepest hops a walk
//! found, keeps a history for each of them as they are probed at the ordinary cadence, and
//! decides what their figures together say about *the path*.
//!
//! # Why more than one hop is probed
//!
//! A single hop is not evidence. Routers routinely rate-limit or deprioritise ICMP addressed
//! **to themselves** while forwarding transit traffic perfectly, so a spike or a run of loss
//! at the deepest hop is as likely to be a busy control plane as a bad path. The edge is
//! therefore [`EdgePolicy::hops`] hops deep, and degradation is only attributed to the path
//! when it shows at the hops before the deepest one as well. When it does not, that is
//! reported as its own state ([`PathQuality::Uncorroborated`]) rather than as a path fault —
//! without this rule the metric lies, which is worse than not having it.
//!
//! # What this figure is not
//!
//! It is **not** a round-trip time to the server, and nothing here may be labelled as one.
//! The deepest answering hop is short of the destination by an unknown distance: the server
//! never answered at any time-to-live, so the walk cannot say whether it sits one router
//! beyond that hop or ten. What can honestly be stated is the hop's *own* distance
//! ([`EdgeHopReading::ttl`]) and where it sits ([`EdgeReading::end`]), and both are carried
//! here so the layer above can say exactly that and no more.
//!
//! Like everything in this crate it reads no clock: callers pass `now` in, so a session of
//! route changes, rate-limiting and recovery replays in a test in microseconds.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::address::AddressPolicy;
use crate::health::{Health, HealthThresholds};
use crate::history::SampleHistory;
use crate::path::{classify, PathEnd, PathTrace};
use crate::sample::ProbeSample;
use crate::stats::WindowStats;
use crate::Error;

/// How many of the deepest answering hops stand in for a silent target.
///
/// Three is the smallest number that can tell a busy router apart from a busy path — one
/// figure to report and two to corroborate it — and it is also the budget: at the ordinary
/// one-second cadence an edge costs three probes a second, which is what `PLAN.md` allots to
/// the one endpoint that matters most.
pub const EDGE_HOPS: usize = 3;

/// How many samples are retained per edge hop.
///
/// The same depth as an application endpoint's own history, for the same reason: two minutes
/// at the active cadence, in a fixed ring of four-byte round-trip times.
pub const EDGE_HISTORY_CAPACITY: usize = 120;

/// When the route to a silent target is walked again.
///
/// Values rather than constants so the tests can pin them and the settings page could one day
/// move them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgePolicy {
    /// How many of the deepest answering hops to probe.
    pub hops: usize,
    /// How many samples each hop retains.
    pub history_capacity: usize,
    /// How long a route is trusted before it is walked again.
    ///
    /// A walk is up to thirty probes issued back to back, so this is minutes rather than
    /// seconds: at five minutes the walk costs a tenth of a probe per second, against the
    /// three per second the hops themselves cost.
    pub rewalk_after: Duration,
    /// How long the deepest hop may stay silent before the route is walked again.
    ///
    /// A route change moves the whole edge somewhere else, and the first symptom is the hop
    /// we chose no longer answering. Waiting out [`EdgePolicy::rewalk_after`] would report
    /// that as loss on a path that is fine.
    pub rewalk_after_silence: Duration,
    /// The shortest gap between two walks, whatever prompts them.
    ///
    /// A hop that permanently rate-limits echoes would otherwise trigger a walk every time
    /// the silence rule fired. This is the floor that keeps that case bounded.
    pub min_rewalk_gap: Duration,
}

impl Default for EdgePolicy {
    fn default() -> Self {
        Self {
            hops: EDGE_HOPS,
            history_capacity: EDGE_HISTORY_CAPACITY,
            rewalk_after: Duration::from_secs(300),
            rewalk_after_silence: Duration::from_secs(20),
            min_rewalk_gap: Duration::from_secs(60),
        }
    }
}

impl EdgePolicy {
    /// The same policy with values that would do nothing raised to the least that works.
    ///
    /// A zero hop count would build an edge that probes nothing and reports silence forever,
    /// which is indistinguishable from a path that is genuinely dead.
    const fn sanitised(mut self) -> Self {
        if self.hops == 0 {
            self.hops = 1;
        }
        self
    }
}

/// One hop being probed as a stand-in for a target that answers nothing.
#[derive(Debug, Clone)]
struct EdgeHop {
    ttl: u8,
    address: IpAddr,
    history: SampleHistory,
    /// When this hop last replied, which is what the silence rule measures.
    last_answer: Option<Instant>,
}

/// Which hops a walk added to the edge and which it took away.
///
/// Returned rather than applied, for the same reason [`crate::endpoint`]'s decisions are: the
/// probe engine is reachable only by message, and a caller that can see the diff can assert
/// that a re-walk finding the same route asks for nothing at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EdgeChange {
    /// Hops that must start being probed.
    pub added: Vec<IpAddr>,
    /// Hops that are no longer part of the edge and must stop being probed.
    pub removed: Vec<IpAddr>,
}

impl EdgeChange {
    /// Whether the walk changed nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The deepest hops that answer on the way to a target that does not.
#[derive(Debug, Clone)]
pub struct PathEdge {
    policy: EdgePolicy,
    /// Shallowest first, so the deepest — the one whose figure is reported — is last.
    hops: Vec<EdgeHop>,
    /// [`None`] until the first walk lands, which is what makes a new edge ask for one.
    walked_at: Option<Instant>,
    end: PathEnd,
}

impl PathEdge {
    /// An edge with no hops yet, which asks to be walked at once.
    #[must_use]
    pub fn new(policy: EdgePolicy) -> Self {
        Self {
            policy: policy.sanitised(),
            hops: Vec::new(),
            walked_at: None,
            end: PathEnd::NothingAnswered,
        }
    }

    /// Takes the hops of a completed walk, keeping the history of any hop still on the route.
    ///
    /// The history is what makes a re-walk cheap: a route that has not changed keeps every
    /// sample it has collected, and the returned [`EdgeChange`] is empty, so the probe engine
    /// is not disturbed at all. A route that *has* changed hands back both halves of the diff
    /// so the caller can register the new hops and release the old ones.
    ///
    /// A walk that reached the target leaves no edge: the target answers, so nothing needs to
    /// stand in for it. Hops the address policy says are not worth probing — the user's own
    /// router, the provider's carrier NAT — are passed over, because a probe to them would be
    /// refused and the slot would be spent on a hop that can never produce a figure.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroCapacity`] for a policy whose hops could retain no samples. The
    /// edge is left exactly as it was: every history is built before anything is replaced.
    pub fn adopt(
        &mut self,
        trace: &PathTrace,
        addresses: &AddressPolicy,
        now: Instant,
    ) -> Result<EdgeChange, Error> {
        let chosen = self.choose(trace, addresses);

        let fresh_needed = chosen
            .iter()
            .filter(|(_, address)| !self.contains(*address))
            .count();
        let mut fresh = Vec::with_capacity(fresh_needed);
        for _ in 0..fresh_needed {
            fresh.push(SampleHistory::new(self.policy.history_capacity)?);
        }
        let mut fresh = fresh.into_iter();

        let mut change = EdgeChange::default();
        let mut kept: Vec<EdgeHop> = Vec::with_capacity(chosen.len());
        for (ttl, address) in chosen {
            if let Some(index) = self.hops.iter().position(|hop| hop.address == address) {
                let mut hop = self.hops.swap_remove(index);
                // The same router can sit at a different distance after a route change; its
                // measurements are still its own, so only the distance is updated.
                hop.ttl = ttl;
                kept.push(hop);
            } else if let Some(history) = fresh.next() {
                change.added.push(address);
                kept.push(EdgeHop {
                    ttl,
                    address,
                    history,
                    last_answer: None,
                });
            }
            // A hop with no history left to give it cannot happen — they were counted from
            // this same list — and skipping it is the harmless reading of it either way.
        }

        change.removed = self.hops.drain(..).map(|hop| hop.address).collect();
        change.removed.sort_unstable();
        self.hops = kept;
        self.walked_at = Some(now);
        self.end = classify(trace, addresses);
        Ok(change)
    }

    /// The hops of a walk that are worth probing, shallowest first.
    fn choose(&self, trace: &PathTrace, addresses: &AddressPolicy) -> Vec<(u8, IpAddr)> {
        if trace.reached_target() {
            // The target answers for itself. An edge would measure a router instead of the
            // destination while a real round trip was available.
            return Vec::new();
        }

        let mut chosen: Vec<(u8, IpAddr)> = Vec::with_capacity(self.policy.hops);
        for hop in trace.hops().iter().rev() {
            if chosen.len() >= self.policy.hops {
                break;
            }
            let Some(address) = hop.address else {
                continue;
            };
            let class = addresses.classify(address);
            if !class.worth_probing() || !class.trusts_transport_rtt() {
                continue;
            }
            // One router can answer at two distances when a walk crosses a load-balanced
            // link. Probing it twice would spend two of the three slots on one machine and
            // then "corroborate" its own rate limiting.
            if chosen.iter().any(|(_, seen)| *seen == address) {
                continue;
            }
            chosen.push((hop.ttl, address));
        }

        chosen.reverse();
        chosen
    }

    /// Records a probe result against one of the hops. Returns `false` for an address that is
    /// not part of the edge — a result that arrived after a re-walk moved on.
    pub fn record(&mut self, address: IpAddr, sample: ProbeSample) -> bool {
        let Some(hop) = self.hops.iter_mut().find(|hop| hop.address == address) else {
            return false;
        };
        if sample.outcome.rtt().is_some() {
            hop.last_answer = Some(sample.at);
        }
        hop.history.record(sample);
        true
    }

    /// Drops a hop nothing can measure, so its slot is not spent on silence.
    ///
    /// Answering a time-to-live expiry does not oblige a router to answer an echo addressed
    /// to it, and a hop that refuses one is worth nothing to the edge. Returns `true` if the
    /// hop was part of it.
    pub fn drop_hop(&mut self, address: IpAddr) -> bool {
        let before = self.hops.len();
        self.hops.retain(|hop| hop.address != address);
        self.hops.len() != before
    }

    /// The addresses being probed, shallowest first.
    #[must_use]
    pub fn addresses(&self) -> impl ExactSizeIterator<Item = IpAddr> + '_ {
        self.hops.iter().map(|hop| hop.address)
    }

    /// Whether an address is part of the edge.
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        self.hops.iter().any(|hop| hop.address == address)
    }

    /// How many hops are being probed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.hops.len()
    }

    /// Whether the edge probes nothing — before the first walk, or after one that found no
    /// hop worth probing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hops.is_empty()
    }

    /// When the route was last walked, or [`None`] if it never has been.
    #[must_use]
    pub const fn walked_at(&self) -> Option<Instant> {
        self.walked_at
    }

    /// Where the last walk said the path stops.
    #[must_use]
    pub const fn end(&self) -> PathEnd {
        self.end
    }

    /// Whether the route should be walked.
    ///
    /// An edge that has never been walked says yes at once: it exists because the layer above
    /// decided this endpoint needs one, and it can measure nothing until a walk gives it hops.
    /// Afterwards there are three reasons, and a floor that keeps all of them affordable. The
    /// walk found no hop worth probing; the route has been trusted for long enough; or the
    /// deepest hop — the one whose figure is reported — has stopped answering, which is what a
    /// route change looks like from here. None of them may fire more often than
    /// [`EdgePolicy::min_rewalk_gap`], because a walk is thirty probes and a hop that always
    /// rate-limits would otherwise ask for one every few seconds forever.
    #[must_use]
    pub fn needs_rewalk(&self, now: Instant) -> bool {
        let Some(walked_at) = self.walked_at else {
            return true;
        };
        let since_walk = now.saturating_duration_since(walked_at);
        if since_walk < self.policy.min_rewalk_gap {
            return false;
        }
        if self.hops.is_empty() || since_walk >= self.policy.rewalk_after {
            return true;
        }

        let silent_for = self
            .hops
            .last()
            .and_then(|hop| hop.last_answer)
            .map_or(since_walk, |at| now.saturating_duration_since(at));
        silent_for >= self.policy.rewalk_after_silence
    }

    /// What the edge's hops say, taken together.
    #[must_use]
    pub fn reading(
        &self,
        now: Instant,
        window: Duration,
        thresholds: &HealthThresholds,
    ) -> EdgeReading {
        let hops: Vec<EdgeHopReading> = self
            .hops
            .iter()
            .map(|hop| {
                let stats = hop.history.stats_for_window(now, window);
                EdgeHopReading {
                    ttl: hop.ttl,
                    address: hop.address,
                    health: thresholds.health_of(&stats),
                    stats,
                }
            })
            .collect();

        let reported = hops.iter().rposition(|hop| hop.health.is_answering());
        EdgeReading {
            quality: quality_of(&hops, reported),
            reported,
            hops,
            end: self.end,
        }
    }
}

/// What the hops together say about the path.
fn quality_of(hops: &[EdgeHopReading], reported: Option<usize>) -> PathQuality {
    if !hops.iter().any(|hop| hop.health.is_known()) {
        return PathQuality::NotMeasuredYet;
    }
    let Some(reported) = reported else {
        // Something is known about these hops and none of it is an answer.
        return PathQuality::Lost;
    };
    let Some(deepest) = hops.get(reported) else {
        return PathQuality::NotMeasuredYet;
    };
    if deepest.health != Health::Degraded {
        // Distance does not come back: a clean figure at the deepest hop that answers means
        // the path up to that depth is clean, whatever the routers before it say about
        // echoes addressed to themselves.
        return PathQuality::Ok;
    }

    // Only hops before the reported one can corroborate it. Anything deeper is, by the
    // definition of "deepest answering", not answering.
    let mut corroborating = hops
        .get(..reported)
        .unwrap_or_default()
        .iter()
        .filter(|hop| hop.health.is_answering())
        .peekable();
    if corroborating.peek().is_none() {
        return PathQuality::Uncorroborated;
    }
    if corroborating.all(|hop| hop.health == Health::Degraded) {
        PathQuality::Degraded
    } else {
        PathQuality::Uncorroborated
    }
}

/// What an edge of hops says about the path to a target that answers nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathQuality {
    /// No hop has been measured enough to say anything yet.
    NotMeasuredYet,
    /// The deepest hop that answers is within every threshold.
    Ok,
    /// Degradation shows at every answering hop, so it belongs to the path.
    Degraded,
    /// The deepest hop's figure moved while the hops before it stayed clean.
    ///
    /// Not reported as a path fault, because it is at least as likely to be that one router's
    /// control plane: routers rate-limit ICMP addressed to themselves while forwarding
    /// everything else perfectly. It is a state of its own rather than silence, since a
    /// degradation confined to the last link really would look like this too, and the user is
    /// owed the observation together with its ambiguity.
    Uncorroborated,
    /// Nothing on the edge answers any more.
    ///
    /// Either the route moved out from under the hops that were chosen, or the path has
    /// broken short of them. A re-walk is what tells the two apart.
    Lost,
}

/// One hop of the edge, measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeHopReading {
    /// The hop's own distance in routers. **Not** the distance to the target, which is
    /// unknown: the target answered at no time-to-live at all.
    pub ttl: u8,
    /// Which router this is.
    pub address: IpAddr,
    /// What its own samples say.
    pub health: Health,
    /// Those samples.
    pub stats: WindowStats,
}

/// The edge's measurement of a path whose destination answers nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeReading {
    /// The hops, shallowest first.
    pub hops: Vec<EdgeHopReading>,
    /// Index into [`EdgeReading::hops`] of the deepest one that is answering — the hop whose
    /// figures the UI shows. [`None`] when none of them is.
    pub reported: Option<usize>,
    /// What the hops say together.
    pub quality: PathQuality,
    /// Where the last walk said the path stops.
    pub end: PathEnd,
}

impl EdgeReading {
    /// The hop whose figures are reported.
    #[must_use]
    pub fn reported_hop(&self) -> Option<&EdgeHopReading> {
        self.hops.get(self.reported?)
    }

    /// Mean round trip to the reported hop, in milliseconds.
    ///
    /// **To that hop, not to the target.** Presenting it as a round trip to the server is the
    /// one thing this whole module exists to avoid.
    #[must_use]
    pub fn rtt_ms(&self) -> Option<f64> {
        self.reported_hop()?.stats.rtt.map(|rtt| rtt.mean_ms)
    }

    /// Jitter at the reported hop, in milliseconds.
    #[must_use]
    pub fn jitter_ms(&self) -> Option<f64> {
        self.reported_hop()?.stats.rtt.and_then(|rtt| rtt.jitter_ms)
    }

    /// Loss at the reported hop, as a percentage.
    #[must_use]
    pub fn loss_pct(&self) -> Option<f64> {
        self.reported_hop()?.stats.loss_pct
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Hop;
    use crate::sample::{ProbeOutcome, Rtt};

    /// Stand-ins for the routers of a path, in increasing distance.
    ///
    /// The documentation ranges would read better, but the address policy classifies them as
    /// unusable — correctly, which is exactly why they cannot play a hop worth probing. These
    /// are well-known public resolver addresses used as routable constants and nothing more:
    /// nothing here sends a packet anywhere, and no address below was observed on any machine.
    const HOME: &str = "192.168.1.1";
    const CARRIER: &str = "100.64.0.1";
    const NEAR: &str = "1.1.1.1";
    const MID: &str = "8.8.8.8";
    const DEEP: &str = "9.9.9.9";
    const DEEPEST: &str = "1.0.0.1";
    const ELSEWHERE: &str = "8.8.4.4";

    fn ip(raw: &str) -> IpAddr {
        raw.parse().expect("a literal address")
    }

    fn addresses() -> AddressPolicy {
        AddressPolicy::default()
    }

    fn thresholds() -> HealthThresholds {
        HealthThresholds::default()
    }

    const WINDOW: Duration = Duration::from_secs(60);

    /// A walk over `(address, milliseconds)` pairs starting at TTL 1; an empty address is a
    /// silent hop. Never reaches the target, which is the case this module exists for.
    fn walk(hops: &[(&str, u32)]) -> PathTrace {
        let hops = hops
            .iter()
            .enumerate()
            .map(|(index, (address, millis))| {
                let ttl = u8::try_from(index + 1).unwrap_or(u8::MAX);
                if address.is_empty() {
                    Hop::silent(ttl)
                } else {
                    Hop::answered(ttl, ip(address), Rtt::from_micros(millis * 1_000))
                }
            })
            .collect();
        PathTrace::new(hops, false)
    }

    /// The ordinary shape: a home router, the provider, then four public hops, the last two
    /// of them past a long link.
    fn typical_walk() -> PathTrace {
        walk(&[
            (HOME, 1),
            (CARRIER, 4),
            (NEAR, 6),
            (MID, 9),
            (DEEP, 40),
            (DEEPEST, 42),
            ("", 0),
            ("", 0),
        ])
    }

    fn edge() -> PathEdge {
        PathEdge::new(EdgePolicy::default())
    }

    /// An edge that has adopted the typical walk.
    fn walked(now: Instant) -> (PathEdge, EdgeChange) {
        let mut edge = edge();
        let change = edge
            .adopt(&typical_walk(), &addresses(), now)
            .expect("the default policy retains samples");
        (edge, change)
    }

    fn ok(millis: u32) -> ProbeOutcome {
        ProbeOutcome::Success(Rtt::from_micros(millis * 1_000))
    }

    /// Feeds `count` results to one hop, one second apart, starting at `start`.
    fn feed(edge: &mut PathEdge, address: &str, start: Instant, count: u32, outcome: ProbeOutcome) {
        for step in 0..count {
            let at = start + Duration::from_secs(u64::from(step));
            edge.record(ip(address), ProbeSample::new(at, outcome));
        }
    }

    #[test]
    fn a_fresh_edge_probes_nothing_and_claims_nothing() {
        let start = Instant::now();
        let edge = edge();
        assert!(edge.is_empty());
        assert_eq!(edge.len(), 0);
        assert_eq!(edge.end(), PathEnd::NothingAnswered);
        assert_eq!(
            edge.reading(start, WINDOW, &thresholds()).quality,
            PathQuality::NotMeasuredYet
        );
    }

    #[test]
    fn the_deepest_answering_hops_are_the_ones_probed() {
        let start = Instant::now();
        let (edge, change) = walked(start);

        assert_eq!(
            edge.addresses().collect::<Vec<_>>(),
            vec![ip(MID), ip(DEEP), ip(DEEPEST)],
            "the edge must sit as close to the destination as the walk reached"
        );
        assert_eq!(change.added.len(), 3);
        assert!(change.removed.is_empty());
    }

    #[test]
    fn hops_inside_the_users_own_network_are_passed_over() {
        // A probe to them would be refused outright, and the slot would be spent on a hop
        // that can never produce a figure.
        let start = Instant::now();
        let mut edge = edge();
        edge.adopt(
            &walk(&[(HOME, 1), (CARRIER, 4), (NEAR, 6)]),
            &addresses(),
            start,
        )
        .unwrap();

        assert_eq!(edge.addresses().collect::<Vec<_>>(), vec![ip(NEAR)]);
    }

    #[test]
    fn a_path_that_dies_inside_the_provider_leaves_no_edge_to_probe() {
        let start = Instant::now();
        let mut edge = edge();
        edge.adopt(
            &walk(&[(HOME, 1), (CARRIER, 4), ("", 0), ("", 0)]),
            &addresses(),
            start,
        )
        .unwrap();

        assert!(edge.is_empty());
        assert_eq!(
            edge.end(),
            PathEnd::InsideTheAccessNetwork { last_hop: 2 },
            "the position is still known, and it is the diagnosis"
        );
    }

    #[test]
    fn a_walk_that_reached_the_target_leaves_no_edge() {
        // The destination answers for itself; measuring a router instead would replace a real
        // round trip with a substitute for one.
        let start = Instant::now();
        let mut edge = edge();
        let reached = PathTrace::new(
            vec![
                Hop::answered(1, ip(NEAR), Rtt::from_micros(5_000)),
                Hop::answered(2, ip(DEEP), Rtt::from_micros(30_000)),
            ],
            true,
        );
        edge.adopt(&reached, &addresses(), start).unwrap();

        assert!(edge.is_empty());
        assert_eq!(edge.end(), PathEnd::Reached);
    }

    #[test]
    fn one_router_answering_at_two_distances_takes_one_slot() {
        // A load-balanced link can put the same machine at two time-to-live values. Probing
        // it twice would spend two of three slots on one router — which would then
        // "corroborate" its own rate limiting.
        let start = Instant::now();
        let mut edge = edge();
        edge.adopt(
            &walk(&[(NEAR, 5), (MID, 9), (DEEP, 40), (DEEP, 41)]),
            &addresses(),
            start,
        )
        .unwrap();

        assert_eq!(
            edge.addresses().collect::<Vec<_>>(),
            vec![ip(NEAR), ip(MID), ip(DEEP)]
        );
    }

    #[test]
    fn a_shallow_walk_uses_every_hop_it_has() {
        let start = Instant::now();
        let mut edge = edge();
        edge.adopt(&walk(&[(NEAR, 5)]), &addresses(), start)
            .unwrap();
        assert_eq!(edge.len(), 1);
    }

    #[test]
    fn a_policy_that_would_probe_no_hops_probes_one() {
        let start = Instant::now();
        let mut edge = PathEdge::new(EdgePolicy {
            hops: 0,
            ..EdgePolicy::default()
        });
        edge.adopt(&typical_walk(), &addresses(), start).unwrap();
        assert_eq!(
            edge.len(),
            1,
            "an edge that probed nothing would report a live path as dead"
        );
    }

    #[test]
    fn a_rewalk_that_finds_the_same_route_keeps_every_sample() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        feed(&mut edge, DEEPEST, start, 5, ok(42));

        let later = start + Duration::from_secs(300);
        let change = edge.adopt(&typical_walk(), &addresses(), later).unwrap();

        assert!(
            change.is_empty(),
            "an unchanged route must not disturb the probe engine at all"
        );
        let reading = edge.reading(later, Duration::from_secs(600), &thresholds());
        assert_eq!(
            reading.reported_hop().unwrap().stats.outcomes.success,
            5,
            "a re-walk must not throw away the history it was measuring"
        );
    }

    #[test]
    fn a_route_change_hands_back_both_halves_of_the_difference() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        let later = start + Duration::from_secs(300);

        let change = edge
            .adopt(
                &walk(&[(HOME, 1), (NEAR, 6), (MID, 9), (DEEP, 40), (ELSEWHERE, 45)]),
                &addresses(),
                later,
            )
            .unwrap();

        assert_eq!(change.added, vec![ip(ELSEWHERE)]);
        assert_eq!(change.removed, vec![ip(DEEPEST)]);
        assert!(edge.contains(ip(ELSEWHERE)));
        assert!(!edge.contains(ip(DEEPEST)));
    }

    #[test]
    fn a_hop_that_moved_distance_keeps_its_measurements() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        feed(&mut edge, DEEPEST, start, 4, ok(42));

        let later = start + Duration::from_secs(300);
        // One hop shorter: the same routers, one fewer step to reach them.
        edge.adopt(
            &walk(&[(CARRIER, 4), (NEAR, 6), (MID, 9), (DEEP, 40), (DEEPEST, 42)]),
            &addresses(),
            later,
        )
        .unwrap();

        let reading = edge.reading(later, Duration::from_secs(600), &thresholds());
        let deepest = reading.reported_hop().unwrap();
        assert_eq!(deepest.ttl, 5, "the new distance is the one reported");
        assert_eq!(deepest.stats.outcomes.success, 4);
    }

    #[test]
    fn results_for_a_hop_that_is_no_longer_on_the_route_are_refused() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        assert!(edge.record(ip(DEEPEST), ProbeSample::new(start, ok(42))));
        assert!(!edge.record(ip(ELSEWHERE), ProbeSample::new(start, ok(42))));
    }

    #[test]
    fn a_hop_that_answers_nothing_can_be_dropped() {
        // Answering a time-to-live expiry does not oblige a router to answer an echo
        // addressed to it.
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        assert!(edge.drop_hop(ip(DEEPEST)));
        assert!(!edge.drop_hop(ip(DEEPEST)));
        assert_eq!(edge.len(), 2);
    }

    #[test]
    fn a_clean_deepest_hop_is_a_clean_path() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        feed(&mut edge, MID, start, 5, ok(9));
        feed(&mut edge, DEEP, start, 5, ok(40));
        feed(&mut edge, DEEPEST, start, 5, ok(42));

        let reading = edge.reading(start + Duration::from_secs(5), WINDOW, &thresholds());
        assert_eq!(reading.quality, PathQuality::Ok);
        assert_eq!(reading.reported_hop().unwrap().ttl, 6);
        assert_eq!(reading.rtt_ms(), Some(42.0));
        assert_eq!(reading.loss_pct(), Some(0.0));
    }

    #[test]
    fn degradation_at_every_hop_is_the_paths() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        for address in [MID, DEEP, DEEPEST] {
            feed(&mut edge, address, start, 3, ok(9));
            feed(
                &mut edge,
                address,
                start + Duration::from_secs(3),
                3,
                ProbeOutcome::Timeout,
            );
        }

        let reading = edge.reading(start + Duration::from_secs(6), WINDOW, &thresholds());
        assert_eq!(reading.quality, PathQuality::Degraded);
        assert_eq!(reading.loss_pct(), Some(50.0));
    }

    #[test]
    fn a_figure_that_moves_at_the_deepest_hop_alone_is_not_the_paths() {
        // The rule the whole module turns on: routers rate-limit echoes addressed to
        // themselves while forwarding perfectly, so one hop misbehaving is not a diagnosis.
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        feed(&mut edge, MID, start, 6, ok(9));
        feed(&mut edge, DEEP, start, 6, ok(40));
        feed(&mut edge, DEEPEST, start, 3, ok(42));
        feed(
            &mut edge,
            DEEPEST,
            start + Duration::from_secs(3),
            3,
            ProbeOutcome::Timeout,
        );

        let reading = edge.reading(start + Duration::from_secs(6), WINDOW, &thresholds());
        assert_eq!(reading.quality, PathQuality::Uncorroborated);
        assert_eq!(
            reading.reported_hop().unwrap().health,
            Health::Degraded,
            "the observation is still reported — only its attribution is withheld"
        );
    }

    #[test]
    fn a_single_hop_can_never_corroborate_itself() {
        let start = Instant::now();
        let mut edge = edge();
        edge.adopt(&walk(&[(NEAR, 5)]), &addresses(), start)
            .unwrap();
        feed(&mut edge, NEAR, start, 3, ok(5));
        feed(
            &mut edge,
            NEAR,
            start + Duration::from_secs(3),
            3,
            ProbeOutcome::Timeout,
        );

        let reading = edge.reading(start + Duration::from_secs(6), WINDOW, &thresholds());
        assert_eq!(reading.quality, PathQuality::Uncorroborated);
    }

    #[test]
    fn a_silent_deepest_hop_falls_back_to_the_one_before_it() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        feed(&mut edge, MID, start, 4, ok(9));
        feed(&mut edge, DEEP, start, 4, ok(40));
        feed(&mut edge, DEEPEST, start, 4, ProbeOutcome::Timeout);

        let reading = edge.reading(start + Duration::from_secs(4), WINDOW, &thresholds());
        assert_eq!(
            reading.reported_hop().unwrap().address,
            ip(DEEP),
            "a rate-limiting router must not cost the path its measurement"
        );
        assert_eq!(reading.quality, PathQuality::Ok);
        assert_eq!(reading.hops.len(), 3, "every hop is still reported");
    }

    #[test]
    fn an_edge_where_nothing_answers_any_more_is_lost() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        for address in [MID, DEEP, DEEPEST] {
            feed(&mut edge, address, start, 4, ProbeOutcome::Timeout);
        }

        let reading = edge.reading(start + Duration::from_secs(4), WINDOW, &thresholds());
        assert_eq!(reading.quality, PathQuality::Lost);
        assert_eq!(reading.reported, None);
        assert_eq!(reading.rtt_ms(), None, "absent knowledge stays absent");
        assert_eq!(reading.jitter_ms(), None);
        assert_eq!(reading.loss_pct(), None);
    }

    #[test]
    fn one_sample_is_not_yet_a_verdict() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        feed(&mut edge, DEEPEST, start, 1, ok(42));

        assert_eq!(
            edge.reading(start, WINDOW, &thresholds()).quality,
            PathQuality::NotMeasuredYet
        );
    }

    #[test]
    fn samples_older_than_the_window_are_not_counted() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        feed(&mut edge, DEEPEST, start, 5, ok(42));

        let much_later = start + Duration::from_secs(3_600);
        assert_eq!(
            edge.reading(much_later, WINDOW, &thresholds()).quality,
            PathQuality::NotMeasuredYet
        );
    }

    #[test]
    fn a_route_is_walked_again_once_it_has_been_trusted_long_enough() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        let policy = EdgePolicy::default();

        // Answering all along, so only the age of the walk can prompt another.
        for step in 0..400_u64 {
            let at = start + Duration::from_secs(step);
            edge.record(ip(DEEPEST), ProbeSample::new(at, ok(42)));
        }

        let just_before = (start + policy.rewalk_after)
            .checked_sub(Duration::from_secs(1))
            .expect("a moment before the periodic walk");
        assert!(!edge.needs_rewalk(just_before));
        assert!(edge.needs_rewalk(start + policy.rewalk_after));
    }

    #[test]
    fn a_deepest_hop_that_stops_answering_prompts_a_walk_without_waiting() {
        // The first symptom of a route change is the hop we chose no longer answering.
        // Waiting out the full period would report that as loss on a path that is fine.
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        let policy = EdgePolicy::default();
        edge.record(ip(DEEPEST), ProbeSample::new(start, ok(42)));

        let silent_at = start + policy.min_rewalk_gap;
        assert!(edge.needs_rewalk(silent_at));
        assert!(
            silent_at < start + policy.rewalk_after,
            "the point is that it does not wait for the periodic walk"
        );
    }

    #[test]
    fn no_walk_may_follow_another_too_closely() {
        // A hop that permanently rate-limits echoes would otherwise ask for thirty probes
        // every few seconds, forever.
        let start = Instant::now();
        let (edge, _) = walked(start);
        let policy = EdgePolicy::default();

        assert!(!edge.needs_rewalk(start));
        assert!(!edge.needs_rewalk(start + policy.rewalk_after_silence));
        let just_before = (start + policy.min_rewalk_gap)
            .checked_sub(Duration::from_millis(1))
            .expect("a moment before the floor");
        assert!(!edge.needs_rewalk(just_before));
        assert!(edge.needs_rewalk(start + policy.min_rewalk_gap));
    }

    #[test]
    fn an_edge_that_has_never_been_walked_asks_for_one_at_once() {
        // It exists because the layer above decided this endpoint needs one, and it can
        // measure nothing at all until a walk gives it hops.
        let start = Instant::now();
        assert!(edge().needs_rewalk(start));
    }

    #[test]
    fn a_walk_that_found_no_hop_worth_probing_is_retried_but_not_at_once() {
        let start = Instant::now();
        let mut edge = edge();
        let policy = EdgePolicy::default();
        edge.adopt(&walk(&[(HOME, 1), ("", 0)]), &addresses(), start)
            .unwrap();

        assert!(edge.is_empty());
        assert!(!edge.needs_rewalk(start));
        assert!(edge.needs_rewalk(start + policy.min_rewalk_gap));
    }

    #[test]
    fn an_answering_edge_is_left_alone_between_periodic_walks() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        let policy = EdgePolicy::default();
        for step in 0..120_u64 {
            let at = start + Duration::from_secs(step);
            edge.record(ip(DEEPEST), ProbeSample::new(at, ok(42)));
        }

        assert!(!edge.needs_rewalk(start + Duration::from_secs(120)));
        assert!(policy.min_rewalk_gap < Duration::from_secs(120));
    }

    #[test]
    fn a_clock_that_appears_to_go_backwards_asks_for_nothing() {
        let start = Instant::now() + Duration::from_secs(1_000);
        let (edge, _) = walked(start);
        let backwards = start
            .checked_sub(Duration::from_secs(500))
            .expect("the clock's origin is far enough back");
        assert!(!edge.needs_rewalk(backwards));
    }

    #[test]
    fn a_history_that_could_hold_nothing_is_refused_and_changes_nothing() {
        let start = Instant::now();
        let (mut edge, _) = walked(start);
        let mut broken = PathEdge::new(EdgePolicy {
            history_capacity: 0,
            ..EdgePolicy::default()
        });

        assert_eq!(
            broken
                .adopt(&typical_walk(), &addresses(), start)
                .unwrap_err(),
            Error::ZeroCapacity
        );
        assert!(broken.is_empty());
        // And the well-formed edge beside it is untouched.
        assert_eq!(edge.len(), 3);
        assert!(edge.record(ip(DEEPEST), ProbeSample::new(start, ok(42))));
    }

    #[test]
    fn the_reading_carries_where_the_path_stops() {
        let start = Instant::now();
        let (edge, _) = walked(start);
        let reading = edge.reading(start, WINDOW, &thresholds());
        assert_eq!(
            reading.end,
            PathEnd::BeyondALongHaulLink {
                last_hop: 6,
                long_haul_at: 5
            },
            "the position is half the answer, and it travels with the figure"
        );
    }
}
