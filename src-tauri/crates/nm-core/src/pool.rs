//! A game's reference pool: the targets that say whether *its servers* are the problem.
//!
//! Every other measurement in this product describes the path between the user and one
//! endpoint. None of them can tell a game whose servers are down from a game the user
//! cannot reach — the symptom is identical, and the two call for opposite actions. A pool
//! is what separates them: several targets belonging to the same game's infrastructure, in
//! different places, probed at a trickle. If one is silent, that is a path. If all of them
//! are silent while the baselines are clean, that is the game.
//!
//! # Two kinds of entry, and why both are needed
//!
//! **Bundled seeds** are addresses the operator publishes (`assets/targets/pools/`). They
//! cover the cold start — the first match, before this machine has seen the game connect to
//! anything — and they ship with a release, never fetched.
//!
//! **Learned entries** are endpoints the user's own game actually connected to. They are
//! the better evidence by far, because they are the servers this user is really placed on,
//! and they are the only evidence at all for a title whose operator publishes nothing.
//!
//! # Wall clock here, and nowhere else
//!
//! Learned entries expire after days unseen, which is a span that has to survive the
//! application being closed — so it is measured on [`SystemTime`], not [`std::time::Instant`].
//! That is the exception `CLAUDE.md` allows: wall clock for persistence, monotonic for
//! measurement. Nothing in this module times a probe; a clock that jumps can only make an
//! entry expire early or late, never corrupt a measurement. Times are passed in, as
//! everywhere else in this crate.
//!
//! Expiry is not tidiness. **Game server addresses rotate**, so a learned entry the user
//! has not touched for a fortnight is as likely to be someone else's machine as the game's —
//! and a pool full of those would report an outage that is not happening, which is the one
//! thing a pool must never do.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use crate::health::{GroupHealth, Health, HealthCounts};
use crate::stats::WindowStats;
use crate::target::TargetAddress;

/// Where a pool entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum PoolSource {
    /// Published by the operator and shipped with the app.
    Bundled,
    /// An endpoint this machine's own game connected to.
    Learned,
}

/// One reference target of a pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolEntry {
    /// Where it lives.
    pub address: TargetAddress,
    /// The operator's name for the place it sits, when the seed carried one.
    ///
    /// [`None`] for a learned entry: we know an address the game connected to and nothing
    /// else about it, and a guessed name would be worse than an address.
    pub label: Option<String>,
    /// Where it came from.
    pub source: PoolSource,
    /// When this machine last saw the game use it, for a learned entry.
    ///
    /// [`None`] for a bundled seed, which never expires and is never evicted: it is the
    /// cold start, and a pool that evicted its seeds would have nothing to say on the first
    /// match after an update.
    pub last_seen: Option<SystemTime>,
}

/// How large a pool may grow and how long an unseen entry survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolPolicy {
    /// How many learned entries one game may keep.
    ///
    /// The cap is on *learned* entries only. Seeds are bounded by the bundled file, which a
    /// test keeps small, and evicting them would defeat their whole purpose.
    pub max_learned: usize,
    /// How long a learned entry survives without being seen again.
    pub expire_after: Duration,
}

impl Default for PoolPolicy {
    /// Thirty-two learned entries, expiring after a fortnight unseen.
    ///
    /// Thirty-two is enough to cover a season's worth of the regions a player is actually
    /// placed in, and small enough that the whole pool cycles through the trickle in
    /// minutes. A fortnight is the shortest span that survives someone not playing over a
    /// holiday, and short enough that a rotated address is gone before it can fake an
    /// outage.
    fn default() -> Self {
        Self {
            max_learned: 32,
            expire_after: Duration::from_secs(14 * 24 * 60 * 60),
        }
    }
}

/// One game's reference targets: what was bundled, and what this machine has learned.
#[derive(Debug, Clone)]
pub struct ReferencePool {
    policy: PoolPolicy,
    bundled: Vec<PoolEntry>,
    /// Learned entries by address, so re-seeing one is a touch rather than a duplicate.
    learned: BTreeMap<TargetAddress, SystemTime>,
}

impl ReferencePool {
    /// Creates a pool from its bundled seeds.
    #[must_use]
    pub fn new(policy: PoolPolicy, seeds: Vec<PoolEntry>) -> Self {
        Self {
            policy,
            bundled: seeds,
            learned: BTreeMap::new(),
        }
    }

    /// Records that the game used `address` at `at`.
    ///
    /// Re-seeing an entry refreshes it rather than duplicating it. Past the cap the
    /// **least recently seen** entry is evicted, which is what keeps a pool describing the
    /// servers this user is currently placed on rather than every one they have ever
    /// touched.
    ///
    /// An address that is already a bundled seed is ignored: it is in the pool either way,
    /// and recording it again would spend one of the learned slots on something the file
    /// already covers.
    pub fn observe(&mut self, address: TargetAddress, at: SystemTime) {
        if self.bundled.iter().any(|seed| seed.address == address) {
            return;
        }

        // A stamp that would move an entry backwards is refused: an entry made *older* by
        // being seen again would expire early, and a wall clock really does step backwards.
        let stamp = self
            .learned
            .get(&address)
            .map_or(at, |seen| if at > *seen { at } else { *seen });
        self.learned.insert(address, stamp);
        self.evict_beyond_cap();
    }

    /// Drops learned entries not seen for longer than the policy allows.
    ///
    /// Bundled seeds are untouched: they do not go stale, because the operator published
    /// them and a release is what changes them.
    pub fn expire(&mut self, now: SystemTime) {
        let cutoff = self.policy.expire_after;
        self.learned.retain(|_, seen| {
            now.duration_since(*seen)
                // A stamp in the future is a wall clock that moved, not an entry to throw
                // away: keeping it is the harmless direction of that error.
                .map_or(true, |unseen| unseen <= cutoff)
        });
    }

    /// Every target the pool would have probed, seeds first.
    ///
    /// Seeds first so a cold pool starts on the addresses the operator published, and
    /// learned entries in most-recently-seen order so the trickle reaches the servers this
    /// user is actually being placed on first.
    #[must_use]
    pub fn entries(&self) -> Vec<PoolEntry> {
        let mut learned: Vec<(&TargetAddress, &SystemTime)> = self.learned.iter().collect();
        learned.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));

        let mut entries = self.bundled.clone();
        entries.extend(learned.into_iter().map(|(address, seen)| PoolEntry {
            address: *address,
            label: None,
            source: PoolSource::Learned,
            last_seen: Some(*seen),
        }));
        entries
    }

    /// How many targets the pool holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bundled.len() + self.learned.len()
    }

    /// Whether the pool has nothing to probe.
    ///
    /// True for a title whose operator publishes nothing and whose servers this machine has
    /// never seen. A real state, and one the UI has to say out loud: a pool with no members
    /// cannot report an outage *or* rule one out, and an empty verdict must never read as a
    /// clean one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The learned entries, for persisting them.
    pub fn learned(&self) -> impl Iterator<Item = (TargetAddress, SystemTime)> + '_ {
        self.learned.iter().map(|(address, seen)| (*address, *seen))
    }

    /// Evicts the least recently seen entries until the cap is met.
    fn evict_beyond_cap(&mut self) {
        while self.learned.len() > self.policy.max_learned {
            let oldest = self
                .learned
                .iter()
                .min_by(|left, right| left.1.cmp(right.1).then_with(|| left.0.cmp(right.0)))
                .map(|(address, _)| *address);
            match oldest {
                Some(address) => {
                    self.learned.remove(&address);
                }
                // Unreachable while the map is over its cap and therefore non-empty; the
                // branch exists so the loop cannot spin.
                None => break,
            }
        }
    }
}

/// One pool member as the reading reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolMember<'a> {
    /// Whether this member has *ever* answered a probe since it joined the pool.
    ///
    /// The single most important field here, and the reason a pool cannot be judged from
    /// its statistics alone. **A learned member is an endpoint a game connected to, and for
    /// most titles that is a UDP match server — which answers nothing we can send, by
    /// design, while the match runs perfectly.** Counting one of those as unreachable would
    /// fill a pool with silence and report a working game as down: precisely the lie this
    /// product exists not to tell, arrived at from a new direction.
    ///
    /// So silence only means something once the member has shown it *can* answer. Until
    /// then it is not evidence about the game, it is an address we have no baseline for.
    pub answered_before: bool,
    /// Its window of probe results.
    pub stats: &'a WindowStats,
}

/// What a pool says about the game's own infrastructure.
///
/// Deliberately **not** a verdict about the game. It reports how much of the pool answers
/// and how much of it could not be judged at all, and leaves the conclusion to
/// [`crate::diagnosis`], which is the only place that has the baselines to compare against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolReading {
    /// The distribution across the members that have proven they answer.
    pub counts: HealthCounts,
    /// How many members have never answered a probe at all.
    ///
    /// Held out of every figure above rather than folded in as failures — see
    /// [`PoolMember::answered_before`]. Reported so the UI can say what the pool is
    /// *unable* to speak for, which for a title whose servers all ignore probes is the
    /// whole of it.
    pub unproven: usize,
    /// Median round-trip time across the answering members, in milliseconds.
    pub rtt_ms: Option<f64>,
    /// Loss across the pool, weighted by probes.
    pub loss_pct: Option<f64>,
}

impl PoolReading {
    /// Judges a pool from its members.
    ///
    /// Members that have never answered are counted apart and left out of every ratio: an
    /// address we have no baseline for cannot tell us that something changed.
    #[must_use]
    pub fn of<'a, I>(members: I, thresholds: &crate::health::HealthThresholds) -> Self
    where
        I: IntoIterator<Item = PoolMember<'a>>,
    {
        let mut proven: Vec<&WindowStats> = Vec::new();
        let mut unproven = 0_usize;
        for member in members {
            if member.answered_before {
                proven.push(member.stats);
            } else {
                unproven += 1;
            }
        }

        let health = GroupHealth::of(proven, thresholds);
        Self {
            counts: health.counts,
            unproven,
            rtt_ms: health.rtt_ms,
            loss_pct: health.loss_pct,
        }
    }

    /// How many members were judged at all.
    ///
    /// The denominator of every ratio below, and never assumed: a pool where nothing has
    /// been probed yet, or where every probe was filtered, has no ratio — not a ratio of
    /// zero.
    #[must_use]
    pub const fn judged(self) -> usize {
        self.counts.known() - self.counts.blocked
    }

    /// The share of judged members that answer, as a fraction of one.
    ///
    /// [`None`] when nothing was judged. That is the state on a network that filters echoes
    /// outright, and it must stay distinct from "nothing answered": one is an absence of
    /// knowledge and the other is a finding.
    #[must_use]
    pub fn answering_ratio(self) -> Option<f64> {
        let judged = self.judged();
        if judged == 0 {
            return None;
        }
        // Pool sizes are bounded by the policy — dozens — so both convert exactly.
        #[allow(clippy::cast_precision_loss)]
        Some(self.counts.answering() as f64 / judged as f64)
    }

    /// The pool's headline state, in the vocabulary everything else uses.
    ///
    /// [`Health::Blocked`] where nothing could be judged, [`Health::Unknown`] where nothing
    /// has been probed yet, and otherwise the same rule a baseline group follows: a clean
    /// sweep is [`Health::Ok`], anything mixed is [`Health::Degraded`] with the counts
    /// beside it, and nothing answering is [`Health::Unreachable`].
    #[must_use]
    pub fn health(self) -> Health {
        if self.counts.total() == 0 {
            return Health::Unknown;
        }
        if self.judged() == 0 {
            return if self.counts.blocked > 0 {
                Health::Blocked
            } else {
                Health::Unknown
            };
        }
        if self.counts.answering() == 0 {
            return Health::Unreachable;
        }
        if self.counts.answering() == self.judged() {
            Health::Ok
        } else {
            Health::Degraded
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;
    use crate::health::HealthThresholds;
    use crate::sample::{ProbeOutcome, ProbeSample, Rtt};

    fn address(last: u8) -> TargetAddress {
        TargetAddress::icmp(IpAddr::V4(Ipv4Addr::new(203, 0, 113, last)))
    }

    fn seed(last: u8) -> PoolEntry {
        PoolEntry {
            address: address(last),
            label: Some(format!("Seed {last}")),
            source: PoolSource::Bundled,
            last_seen: None,
        }
    }

    fn epoch(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn policy() -> PoolPolicy {
        PoolPolicy {
            max_learned: 3,
            expire_after: Duration::from_secs(100),
        }
    }

    /// A member that has proven it answers, over `stats`.
    fn proven(stats: &WindowStats) -> PoolMember<'_> {
        PoolMember {
            answered_before: true,
            stats,
        }
    }

    /// A member that has never answered anything.
    fn unproven(stats: &WindowStats) -> PoolMember<'_> {
        PoolMember {
            answered_before: false,
            stats,
        }
    }

    /// A window of `outcomes`, one second apart.
    fn window(outcomes: &[ProbeOutcome]) -> WindowStats {
        let start = std::time::Instant::now();
        let samples: Vec<ProbeSample> = outcomes
            .iter()
            .enumerate()
            .map(|(index, outcome)| {
                let step = u64::try_from(index).unwrap_or(0);
                ProbeSample::new(start + Duration::from_secs(step), *outcome)
            })
            .collect();
        WindowStats::of(&samples)
    }

    fn ok() -> ProbeOutcome {
        ProbeOutcome::Success(Rtt::from_micros(30_000))
    }

    #[test]
    fn a_pool_with_no_seeds_and_nothing_learned_is_empty() {
        let pool = ReferencePool::new(policy(), Vec::new());
        assert!(pool.is_empty());
        assert!(pool.entries().is_empty());
    }

    #[test]
    fn seeds_cover_the_cold_start() {
        let pool = ReferencePool::new(policy(), vec![seed(1), seed(2)]);
        assert_eq!(pool.len(), 2);
        assert!(pool
            .entries()
            .iter()
            .all(|entry| entry.source == PoolSource::Bundled));
    }

    #[test]
    fn an_observed_endpoint_joins_the_pool() {
        let mut pool = ReferencePool::new(policy(), vec![seed(1)]);
        pool.observe(address(10), epoch(1_000));

        let entries = pool.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source, PoolSource::Bundled);
        assert_eq!(entries[1].source, PoolSource::Learned);
        assert_eq!(entries[1].address, address(10));
        assert_eq!(entries[1].last_seen, Some(epoch(1_000)));
        // Nothing is known about a learned address but the address itself.
        assert_eq!(entries[1].label, None);
    }

    #[test]
    fn seeing_an_endpoint_again_refreshes_it_rather_than_duplicating_it() {
        let mut pool = ReferencePool::new(policy(), Vec::new());
        pool.observe(address(10), epoch(1_000));
        pool.observe(address(10), epoch(2_000));

        assert_eq!(pool.len(), 1);
        assert_eq!(pool.entries()[0].last_seen, Some(epoch(2_000)));
    }

    #[test]
    fn a_stamp_that_would_age_an_entry_is_refused() {
        // A wall clock really does step backwards, and an entry made older by being seen
        // again would expire early.
        let mut pool = ReferencePool::new(policy(), Vec::new());
        pool.observe(address(10), epoch(2_000));
        pool.observe(address(10), epoch(1_000));

        assert_eq!(pool.entries()[0].last_seen, Some(epoch(2_000)));
    }

    #[test]
    fn an_address_that_is_already_a_seed_does_not_spend_a_learned_slot() {
        let mut pool = ReferencePool::new(policy(), vec![seed(1)]);
        pool.observe(address(1), epoch(1_000));

        assert_eq!(pool.len(), 1);
        assert_eq!(pool.learned().count(), 0);
    }

    #[test]
    fn past_the_cap_the_least_recently_seen_entry_goes() {
        let mut pool = ReferencePool::new(policy(), Vec::new());
        pool.observe(address(10), epoch(1_000));
        pool.observe(address(11), epoch(2_000));
        pool.observe(address(12), epoch(3_000));
        pool.observe(address(13), epoch(4_000));

        let held: Vec<TargetAddress> = pool.entries().into_iter().map(|e| e.address).collect();
        assert_eq!(held.len(), 3);
        assert!(
            !held.contains(&address(10)),
            "the oldest must be the one evicted"
        );
        assert!(held.contains(&address(13)));
    }

    #[test]
    fn the_cap_never_evicts_a_bundled_seed() {
        // Seeds are the cold start: a pool that evicted them would have nothing to say on
        // the first match after an update.
        let mut pool = ReferencePool::new(policy(), vec![seed(1), seed(2)]);
        for index in 0..10 {
            pool.observe(address(20 + index), epoch(1_000 + u64::from(index)));
        }

        let seeds = pool
            .entries()
            .into_iter()
            .filter(|entry| entry.source == PoolSource::Bundled)
            .count();
        assert_eq!(seeds, 2);
        assert_eq!(
            pool.len(),
            5,
            "two seeds and the three learned the cap allows"
        );
    }

    #[test]
    fn learned_entries_come_back_most_recently_seen_first() {
        // The trickle should reach the servers this user is actually being placed on before
        // the ones they touched a week ago.
        let mut pool = ReferencePool::new(policy(), Vec::new());
        pool.observe(address(10), epoch(1_000));
        pool.observe(address(11), epoch(3_000));
        pool.observe(address(12), epoch(2_000));

        let order: Vec<TargetAddress> = pool.entries().into_iter().map(|e| e.address).collect();
        assert_eq!(order, vec![address(11), address(12), address(10)]);
    }

    #[test]
    fn an_entry_unseen_for_too_long_expires() {
        // Game server addresses rotate; a stale entry would fake an outage.
        let mut pool = ReferencePool::new(policy(), Vec::new());
        pool.observe(address(10), epoch(1_000));
        pool.observe(address(11), epoch(1_080));

        pool.expire(epoch(1_150));

        let held: Vec<TargetAddress> = pool.entries().into_iter().map(|e| e.address).collect();
        assert_eq!(held, vec![address(11)]);
    }

    #[test]
    fn expiry_is_inclusive_of_the_boundary() {
        let mut pool = ReferencePool::new(policy(), Vec::new());
        pool.observe(address(10), epoch(1_000));

        pool.expire(epoch(1_100));
        assert_eq!(pool.len(), 1, "exactly at the limit is still within it");

        pool.expire(epoch(1_101));
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn expiry_never_touches_a_seed() {
        let mut pool = ReferencePool::new(policy(), vec![seed(1)]);
        pool.expire(epoch(999_999));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn a_clock_that_stepped_backwards_does_not_expire_everything() {
        let mut pool = ReferencePool::new(policy(), Vec::new());
        pool.observe(address(10), epoch(5_000));
        pool.expire(epoch(1_000));
        assert_eq!(
            pool.len(),
            1,
            "a stamp in the future is a moved clock, not a stale entry"
        );
    }

    #[test]
    fn a_pool_nothing_has_probed_reports_nothing() {
        let reading = PoolReading::of(std::iter::empty(), &HealthThresholds::default());
        assert_eq!(reading.health(), Health::Unknown);
        assert_eq!(reading.answering_ratio(), None);
        assert_eq!(reading.judged(), 0);
    }

    #[test]
    fn a_pool_answering_everywhere_is_ok() {
        let windows = [window(&[ok(), ok()]), window(&[ok(), ok()])];
        let reading = PoolReading::of(windows.iter().map(proven), &HealthThresholds::default());

        assert_eq!(reading.health(), Health::Ok);
        assert_eq!(reading.answering_ratio(), Some(1.0));
    }

    #[test]
    fn a_pool_answering_nowhere_is_unreachable() {
        let dead = [ProbeOutcome::Timeout, ProbeOutcome::Timeout];
        let windows = [window(&dead), window(&dead)];
        let reading = PoolReading::of(windows.iter().map(proven), &HealthThresholds::default());

        assert_eq!(reading.health(), Health::Unreachable);
        assert_eq!(reading.answering_ratio(), Some(0.0));
    }

    #[test]
    fn a_partly_answering_pool_reports_the_share_rather_than_a_verdict() {
        // The partial outage case, which is what a pool exists to make visible: some of a
        // game's regions gone while others serve normally.
        let dead = [ProbeOutcome::Timeout, ProbeOutcome::Timeout];
        let windows = [
            window(&[ok(), ok()]),
            window(&[ok(), ok()]),
            window(&dead),
            window(&dead),
        ];
        let reading = PoolReading::of(windows.iter().map(proven), &HealthThresholds::default());

        assert_eq!(reading.health(), Health::Degraded);
        assert_eq!(reading.answering_ratio(), Some(0.5));
        assert_eq!(reading.counts.unreachable, 2);
    }

    #[test]
    fn a_pool_whose_probes_are_all_filtered_has_no_ratio_at_all() {
        // On a network that filters echoes outright the pool knows nothing, and "nothing
        // answered" would be a finding it has not earned.
        let filtered = [ProbeOutcome::Blocked, ProbeOutcome::Blocked];
        let windows = [window(&filtered), window(&filtered)];
        let reading = PoolReading::of(windows.iter().map(proven), &HealthThresholds::default());

        assert_eq!(reading.health(), Health::Blocked);
        assert_eq!(reading.answering_ratio(), None);
    }

    #[test]
    fn a_member_that_has_never_answered_is_not_evidence_of_anything() {
        // The failure this rule exists to prevent, and it is the *normal* case for most
        // titles: a learned member is an endpoint a game connected to, which for a UDP
        // match server answers nothing we can send while the match runs perfectly. Counted
        // as unreachable it would fill a pool with silence and report a working game as
        // down — the same lie as calling the endpoint itself unreachable, reached from a
        // new direction.
        let dead = [ProbeOutcome::Timeout, ProbeOutcome::Timeout];
        let windows = [window(&dead), window(&dead), window(&dead)];
        let reading = PoolReading::of(windows.iter().map(unproven), &HealthThresholds::default());

        assert_eq!(reading.unproven, 3);
        assert_eq!(reading.counts.total(), 0);
        assert_eq!(reading.answering_ratio(), None);
        assert_eq!(
            reading.health(),
            Health::Unknown,
            "a pool with no baseline for any member knows nothing, and must not say otherwise"
        );
    }

    #[test]
    fn a_member_that_answered_once_and_then_went_silent_is_the_real_signal() {
        // The complement: silence *after* a member has shown it can answer is exactly what
        // a pool exists to notice.
        let dead = [ProbeOutcome::Timeout, ProbeOutcome::Timeout];
        let windows = [window(&dead)];
        let reading = PoolReading::of(windows.iter().map(proven), &HealthThresholds::default());

        assert_eq!(reading.unproven, 0);
        assert_eq!(reading.health(), Health::Unreachable);
        assert_eq!(reading.answering_ratio(), Some(0.0));
    }

    #[test]
    fn proven_members_are_judged_while_unproven_ones_are_only_counted() {
        // The mixed case, which is what a Valve title looks like: published relays that
        // answer beside learned match servers that never will.
        let dead = [ProbeOutcome::Timeout, ProbeOutcome::Timeout];
        let alive = window(&[ok(), ok()]);
        let silent = window(&dead);
        let reading = PoolReading::of(
            [proven(&alive), unproven(&silent), unproven(&silent)],
            &HealthThresholds::default(),
        );

        assert_eq!(reading.unproven, 2);
        assert_eq!(reading.answering_ratio(), Some(1.0));
        assert_eq!(reading.health(), Health::Ok);
    }

    #[test]
    fn filtered_members_stay_out_of_the_ratios_denominator() {
        let filtered = [ProbeOutcome::Blocked, ProbeOutcome::Blocked];
        let dead = [ProbeOutcome::Timeout, ProbeOutcome::Timeout];
        let windows = [window(&[ok(), ok()]), window(&dead), window(&filtered)];
        let reading = PoolReading::of(windows.iter().map(proven), &HealthThresholds::default());

        assert_eq!(reading.judged(), 2);
        assert_eq!(reading.answering_ratio(), Some(0.5));
    }
}
