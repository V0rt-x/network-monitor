//! Which endpoints each monitored application is talking to, and which of them to probe.
//!
//! This sits between discovery and scheduling. Discovery hands over observations from two
//! sources of very different shape — a connection table poll, which sees that a socket
//! exists but not how busy it is, and flow events, which see volume — and this module
//! turns that stream into a stable set of endpoints per application, each with a state and
//! a decision about how often it is worth probing.
//!
//! Three rules, all of which exist because the obvious alternative is worse:
//!
//! **Nothing is ever silently dropped.** The per-application cap limits how many endpoints
//! are probed *at the normal interval*, not how many are known. Past the cap an endpoint
//! demotes to a longer interval and stays visible, because an endpoint that vanished from
//! the UI because it was ranked seventeenth would look exactly like one that stopped
//! working.
//!
//! **Absent knowledge stays absent.** A source that cannot count bytes yields
//! [`TrackedEndpoint::recent_bytes`] of [`None`], never `Some(0)`: on Windows without the
//! one-time tracing setup there are no byte counters at all, and reporting zero throughput
//! for a busy game would be a lie the user cannot see through. Ranking then falls back to
//! recency on its own, with no special case.
//!
//! **Silence is not death.** An endpoint that stops being observed goes idle before it is
//! forgotten, and the gap between the two is generous, because flows go quiet between
//! rounds and a game whose endpoints were forgotten during a loading screen would have to
//! rediscover and re-measure everything the moment play resumed.
//!
//! No type here reads a clock: callers pass `now`, which is what lets the tests below play
//! out hours of lifecycle in microseconds.

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crate::Error;

/// Maximum number of applications the user may monitor at the same time.
///
/// A product promise from `CLAUDE.md` rather than a tuning knob.
pub const MAX_MONITORED_APPS: u32 = 5;

/// Maximum number of endpoints probed at the normal interval per application.
///
/// Endpoints past this count are not dropped: they demote to infrequent probing,
/// prioritized by recent traffic. Note that `MAX_MONITORED_APPS *
/// MAX_ACTIVE_ENDPOINTS_PER_APP` deliberately exceeds the global probe rate cap at a
/// one-second interval — the scheduler is *expected* to be oversubscribed, and answers by
/// stretching intervals rather than by abandoning targets.
pub const MAX_ACTIVE_ENDPOINTS_PER_APP: u32 = 16;

/// Identifies one monitored application for the life of a session.
///
/// Deliberately not a process identifier. The layer above assigns these, which keeps this
/// module free of any operating system's notion of a process and lets a caller decide for
/// itself whether a restarted game is the same application or a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppId(u32);

impl AppId {
    /// Wraps a raw identifier.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw identifier, for logging and for crossing the IPC boundary.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Transport an endpoint is reached over.
///
/// Mirrors the platform layer's notion deliberately rather than sharing it: `nm-core` sits
/// below `nm-platform` in the dependency order and must not learn about it. The layer that
/// owns both converts, which costs one `match` and keeps the core buildable on a machine
/// with no operating-system backend at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Transport {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

/// One remote endpoint of an application.
///
/// The transport is part of the identity: a server reached over TCP on one port and UDP on
/// another is two endpoints with two independent fates, which is the normal shape of a
/// game (a lobby over TCP, play over UDP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EndpointKey {
    /// Transport the application uses to reach it.
    pub transport: Transport,
    /// Where it is.
    pub address: SocketAddr,
}

impl EndpointKey {
    /// An endpoint reached over TCP.
    #[must_use]
    pub const fn tcp(address: SocketAddr) -> Self {
        Self {
            transport: Transport::Tcp,
            address,
        }
    }

    /// An endpoint reached over UDP.
    #[must_use]
    pub const fn udp(address: SocketAddr) -> Self {
        Self {
            transport: Transport::Udp,
            address,
        }
    }
}

/// Whether an application is currently using an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Liveness {
    /// Observed recently.
    Active,
    /// Not observed for a while, but still remembered.
    Idle,
}

/// How often an endpoint is worth probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Probing {
    /// Probed at the normal interval.
    Active,
    /// Probed at the long interval — idle, or ranked past the per-application cap.
    /// Still tracked, still shown, never dropped.
    Demoted,
}

/// Whether an observation introduced an endpoint or matched a known one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Appearance {
    /// First time this application was seen talking to it; the caller should register a
    /// probe target.
    New,
    /// Already tracked; the observation only refreshed it.
    Known,
}

/// Timings and caps governing the endpoint lifecycle.
///
/// The defaults are chosen against a one-second discovery poll; every one of them is a
/// trade the documentation on each field spells out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecyclePolicy {
    /// Silence longer than this makes an endpoint [`Liveness::Idle`].
    ///
    /// Ten seconds is many discovery polls, so a single missed poll never demotes a busy
    /// endpoint, while a genuinely finished flow stops consuming a probe slot promptly.
    pub active_for: Duration,
    /// Silence longer than this forgets the endpoint entirely.
    ///
    /// Two minutes covers a loading screen, a between-rounds pause or a menu visit.
    /// Forgetting sooner would mean rediscovering and re-measuring an endpoint the user
    /// was in the middle of watching; forgetting later would keep a finished match's
    /// servers in the list long after they stopped meaning anything.
    pub retain_for: Duration,
    /// Span each traffic bucket covers when ranking by recent volume.
    pub traffic_window: Duration,
    /// Interval for endpoints inside the cap.
    pub active_interval: Duration,
    /// Interval for demoted endpoints.
    ///
    /// Ten times the active interval: enough to keep an idle endpoint honestly measured —
    /// so that it is known to be reachable when the application returns to it — while
    /// costing a tenth of the budget.
    pub demoted_interval: Duration,
    /// How many endpoints per application are probed at the active interval.
    pub max_active_endpoints: u32,
}

impl Default for LifecyclePolicy {
    fn default() -> Self {
        Self {
            active_for: Duration::from_secs(10),
            retain_for: Duration::from_secs(120),
            traffic_window: Duration::from_secs(30),
            active_interval: Duration::from_secs(1),
            demoted_interval: Duration::from_secs(10),
            max_active_endpoints: MAX_ACTIVE_ENDPOINTS_PER_APP,
        }
    }
}

impl LifecyclePolicy {
    /// The probe interval a [`Probing`] decision implies.
    #[must_use]
    pub const fn interval_for(&self, probing: Probing) -> Duration {
        match probing {
            Probing::Active => self.active_interval,
            Probing::Demoted => self.demoted_interval,
        }
    }

    /// Rejects a policy whose parts contradict each other.
    fn validate(&self) -> Result<(), Error> {
        if self.active_interval.is_zero()
            || self.demoted_interval.is_zero()
            || self.traffic_window.is_zero()
        {
            return Err(Error::ZeroInterval);
        }
        if self.max_active_endpoints == 0 {
            return Err(Error::ZeroEndpointCap);
        }
        if self.retain_for < self.active_for {
            return Err(Error::RetentionShorterThanActivity);
        }
        Ok(())
    }
}

/// One endpoint, with everything known about how the application uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedEndpoint {
    key: EndpointKey,
    first_seen: Instant,
    last_seen: Instant,
    window_started: Instant,
    bytes_current: u64,
    bytes_previous: u64,
    counted: bool,
    liveness: Liveness,
    probing: Probing,
}

impl TrackedEndpoint {
    /// Which endpoint this is.
    #[must_use]
    pub const fn key(&self) -> EndpointKey {
        self.key
    }

    /// When the application was first seen using it.
    #[must_use]
    pub const fn first_seen(&self) -> Instant {
        self.first_seen
    }

    /// When it was last seen using it.
    #[must_use]
    pub const fn last_seen(&self) -> Instant {
        self.last_seen
    }

    /// Whether the application is currently using it.
    #[must_use]
    pub const fn liveness(&self) -> Liveness {
        self.liveness
    }

    /// How often it is worth probing, as of the last sweep.
    #[must_use]
    pub const fn probing(&self) -> Probing {
        self.probing
    }

    /// Bytes observed recently, or [`None`] when no source counts them.
    ///
    /// [`None`] and `Some(0)` are different answers and must stay so: the first says
    /// throughput is unknown here, the second says it was measured and was nothing. A
    /// build with no flow-event source reports the first for every endpoint.
    #[must_use]
    pub const fn recent_bytes(&self) -> Option<u64> {
        if self.counted {
            Some(self.bytes_current.saturating_add(self.bytes_previous))
        } else {
            None
        }
    }

    /// Volume used for ranking.
    ///
    /// Unknown counts as nothing here, which is *not* the fake zero the crate forbids: it
    /// decides an ordering rather than being shown to anyone, and when nothing is counted
    /// every endpoint scores alike and the ordering falls through to recency by itself.
    const fn rank_bytes(&self) -> u64 {
        match self.recent_bytes() {
            Some(bytes) => bytes,
            None => 0,
        }
    }

    /// Advances the traffic buckets to `now`.
    fn roll_traffic_window(&mut self, window: Duration, now: Instant) {
        let elapsed = now.saturating_duration_since(self.window_started);
        if elapsed >= window.saturating_mul(2) {
            // Long enough that both buckets describe a past that no longer says anything
            // about current traffic.
            self.bytes_current = 0;
            self.bytes_previous = 0;
            self.window_started = now;
        } else if elapsed >= window {
            self.bytes_previous = self.bytes_current;
            self.bytes_current = 0;
            self.window_started = self.window_started.checked_add(window).unwrap_or(now);
        }
    }
}

/// The endpoints of one application.
#[derive(Debug, Clone, Default)]
struct AppEndpoints {
    endpoints: BTreeMap<EndpointKey, TrackedEndpoint>,
}

/// Tracks what every monitored application is talking to.
#[derive(Debug, Clone)]
pub struct EndpointTracker {
    policy: LifecyclePolicy,
    apps: BTreeMap<AppId, AppEndpoints>,
    /// Reused by each sweep so that ranking allocates nothing in the steady state.
    ranking: Vec<(u64, Instant, EndpointKey)>,
}

impl EndpointTracker {
    /// Creates a tracker.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroInterval`], [`Error::ZeroEndpointCap`] or
    /// [`Error::RetentionShorterThanActivity`] for a policy that cannot be satisfied.
    pub fn new(policy: LifecyclePolicy) -> Result<Self, Error> {
        policy.validate()?;
        Ok(Self {
            policy,
            apps: BTreeMap::new(),
            ranking: Vec::new(),
        })
    }

    /// The policy in force.
    #[must_use]
    pub const fn policy(&self) -> &LifecyclePolicy {
        &self.policy
    }

    /// Starts tracking an application. Monitoring one already tracked changes nothing.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TooManyApps`] once [`MAX_MONITORED_APPS`] are monitored. The cap
    /// is refused rather than enforced by evicting somebody, because which application to
    /// stop watching is the user's decision, not ours.
    pub fn monitor(&mut self, app: AppId) -> Result<(), Error> {
        if self.apps.contains_key(&app) {
            return Ok(());
        }
        let limit = MAX_MONITORED_APPS;
        if self.apps.len() >= usize::try_from(limit).unwrap_or(usize::MAX) {
            return Err(Error::TooManyApps { limit });
        }
        self.apps.insert(app, AppEndpoints::default());
        Ok(())
    }

    /// Whether an application is being tracked.
    #[must_use]
    pub fn is_monitored(&self, app: AppId) -> bool {
        self.apps.contains_key(&app)
    }

    /// How many applications are tracked.
    #[must_use]
    pub fn app_count(&self) -> usize {
        self.apps.len()
    }

    /// Stops tracking an application, returning the endpoints that are now unreferenced.
    ///
    /// The caller needs that list to release the probe targets it registered; dropping it
    /// silently would leave them being probed for an application nobody is watching.
    pub fn forget(&mut self, app: AppId) -> Vec<EndpointKey> {
        self.apps
            .remove(&app)
            .map(|endpoints| endpoints.endpoints.into_keys().collect())
            .unwrap_or_default()
    }

    /// Records that `app` exchanged data with `endpoint`.
    ///
    /// `bytes` is [`None`] when the source cannot count them — a connection-table poll
    /// sees that a socket exists and nothing more. That is carried through rather than
    /// substituted with zero.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownApp`] when the application is not monitored. Registering it
    /// implicitly would let discovery walk straight past [`MAX_MONITORED_APPS`].
    pub fn observe(
        &mut self,
        app: AppId,
        endpoint: EndpointKey,
        bytes: Option<u64>,
        now: Instant,
    ) -> Result<Appearance, Error> {
        let window = self.policy.traffic_window;
        let endpoints = self
            .apps
            .get_mut(&app)
            .ok_or(Error::UnknownApp { app: app.get() })?;

        if let Some(tracked) = endpoints.endpoints.get_mut(&endpoint) {
            tracked.roll_traffic_window(window, now);
            tracked.last_seen = now;
            tracked.liveness = Liveness::Active;
            if let Some(bytes) = bytes {
                tracked.counted = true;
                tracked.bytes_current = tracked.bytes_current.saturating_add(bytes);
            }
            return Ok(Appearance::Known);
        }

        endpoints.endpoints.insert(
            endpoint,
            TrackedEndpoint {
                key: endpoint,
                first_seen: now,
                last_seen: now,
                window_started: now,
                bytes_current: bytes.unwrap_or(0),
                bytes_previous: 0,
                counted: bytes.is_some(),
                liveness: Liveness::Active,
                // Conservative until the first sweep ranks it: a new endpoint costs the
                // long interval for at most one discovery cycle, and the budget is the
                // thing that must not be surprised.
                probing: Probing::Demoted,
            },
        );
        Ok(Appearance::New)
    }

    /// Ages every endpoint, forgets the long-silent ones, and re-ranks the rest.
    ///
    /// `gone` is cleared and filled with the endpoints that were forgotten, so the caller
    /// can release their probe targets. Call this once per discovery cycle: observations
    /// record what happened, and this is what turns the record into decisions.
    pub fn sweep(&mut self, now: Instant, gone: &mut Vec<(AppId, EndpointKey)>) {
        gone.clear();
        let policy = &self.policy;
        let ranking = &mut self.ranking;

        for (&app, endpoints) in &mut self.apps {
            endpoints.endpoints.retain(|key, tracked| {
                let silent_for = now.saturating_duration_since(tracked.last_seen);
                if silent_for > policy.retain_for {
                    gone.push((app, *key));
                    false
                } else {
                    true
                }
            });

            ranking.clear();
            for tracked in endpoints.endpoints.values_mut() {
                tracked.roll_traffic_window(policy.traffic_window, now);
                let silent_for = now.saturating_duration_since(tracked.last_seen);
                tracked.liveness = if silent_for > policy.active_for {
                    Liveness::Idle
                } else {
                    Liveness::Active
                };
                // Everything starts demoted; the ranking below promotes the survivors.
                tracked.probing = Probing::Demoted;
                if tracked.liveness == Liveness::Active {
                    ranking.push((tracked.rank_bytes(), tracked.last_seen, tracked.key));
                }
            }

            // Busiest first, then most recently seen, then by key so the order is
            // reproducible rather than dependent on how the map happened to be built.
            ranking.sort_unstable_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then(right.1.cmp(&left.1))
                    .then(left.2.cmp(&right.2))
            });

            let cap = usize::try_from(policy.max_active_endpoints).unwrap_or(usize::MAX);
            for (_, _, key) in ranking.iter().take(cap) {
                if let Some(tracked) = endpoints.endpoints.get_mut(key) {
                    tracked.probing = Probing::Active;
                }
            }
        }
    }

    /// Every endpoint of an application, in a stable order.
    ///
    /// Ordered by endpoint rather than by severity or volume: sorting for presentation is
    /// the UI's job, and a stable order here keeps callers reproducible.
    pub fn endpoints(&self, app: AppId) -> impl Iterator<Item = &TrackedEndpoint> + '_ {
        self.apps
            .get(&app)
            .into_iter()
            .flat_map(|endpoints| endpoints.endpoints.values())
    }

    /// How many endpoints an application has, probed or demoted.
    #[must_use]
    pub fn endpoint_count(&self, app: AppId) -> usize {
        self.apps
            .get(&app)
            .map_or(0, |endpoints| endpoints.endpoints.len())
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    fn tracker() -> EndpointTracker {
        EndpointTracker::new(LifecyclePolicy::default()).unwrap()
    }

    fn socket(last: u8, port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, last)), port)
    }

    fn udp(last: u8) -> EndpointKey {
        EndpointKey::udp(socket(last, 27_015))
    }

    const APP: AppId = AppId::new(1);

    fn monitored() -> (EndpointTracker, Instant) {
        let mut tracker = tracker();
        tracker.monitor(APP).unwrap();
        (tracker, Instant::now())
    }

    #[test]
    fn a_new_tracker_watches_nothing() {
        let tracker = tracker();
        assert_eq!(tracker.app_count(), 0);
        assert!(!tracker.is_monitored(APP));
        assert_eq!(tracker.endpoints(APP).count(), 0);
    }

    #[test]
    fn rejects_a_policy_that_cannot_be_satisfied() {
        let zero_interval = LifecyclePolicy {
            active_interval: Duration::ZERO,
            ..LifecyclePolicy::default()
        };
        assert_eq!(
            EndpointTracker::new(zero_interval).unwrap_err(),
            Error::ZeroInterval
        );

        let no_slots = LifecyclePolicy {
            max_active_endpoints: 0,
            ..LifecyclePolicy::default()
        };
        assert_eq!(
            EndpointTracker::new(no_slots).unwrap_err(),
            Error::ZeroEndpointCap
        );

        // Forgetting an endpoint before it has even gone idle would make "idle" a state
        // no endpoint could ever be observed in.
        let forgetful = LifecyclePolicy {
            active_for: Duration::from_secs(60),
            retain_for: Duration::from_secs(30),
            ..LifecyclePolicy::default()
        };
        assert_eq!(
            EndpointTracker::new(forgetful).unwrap_err(),
            Error::RetentionShorterThanActivity
        );
    }

    #[test]
    fn monitors_up_to_the_cap_and_then_refuses() {
        let mut tracker = tracker();
        for raw in 0..MAX_MONITORED_APPS {
            tracker.monitor(AppId::new(raw)).unwrap();
        }
        assert_eq!(tracker.app_count(), 5);

        assert_eq!(
            tracker.monitor(AppId::new(99)).unwrap_err(),
            Error::TooManyApps { limit: 5 }
        );
        // Refusing must not have disturbed what was already there.
        assert_eq!(tracker.app_count(), 5);
        assert!(tracker.is_monitored(AppId::new(0)));
    }

    #[test]
    fn monitoring_the_same_app_twice_costs_no_slot() {
        let mut tracker = tracker();
        tracker.monitor(APP).unwrap();
        tracker.monitor(APP).unwrap();
        assert_eq!(tracker.app_count(), 1);
    }

    #[test]
    fn an_unmonitored_app_is_refused_rather_than_registered() {
        // Otherwise discovery would walk straight past the five-application cap.
        let mut tracker = tracker();
        let error = tracker
            .observe(APP, udp(1), None, Instant::now())
            .unwrap_err();
        assert_eq!(error, Error::UnknownApp { app: 1 });
        assert_eq!(tracker.app_count(), 0);
    }

    #[test]
    fn the_first_sighting_is_new_and_the_rest_are_known() {
        let (mut tracker, now) = monitored();
        assert_eq!(
            tracker.observe(APP, udp(1), None, now).unwrap(),
            Appearance::New
        );
        assert_eq!(
            tracker.observe(APP, udp(1), None, now).unwrap(),
            Appearance::Known
        );
        assert_eq!(tracker.endpoint_count(APP), 1);
    }

    #[test]
    fn transport_and_port_are_part_of_the_identity() {
        // A game's lobby over TCP and its play over UDP are two endpoints with two fates.
        let (mut tracker, now) = monitored();
        let address = socket(1, 27_015);
        tracker
            .observe(APP, EndpointKey::udp(address), None, now)
            .unwrap();
        tracker
            .observe(APP, EndpointKey::tcp(address), None, now)
            .unwrap();
        tracker
            .observe(APP, EndpointKey::udp(socket(1, 27_016)), None, now)
            .unwrap();

        assert_eq!(tracker.endpoint_count(APP), 3);
    }

    #[test]
    fn two_apps_talking_to_one_address_keep_separate_records() {
        let mut tracker = tracker();
        let other = AppId::new(2);
        tracker.monitor(APP).unwrap();
        tracker.monitor(other).unwrap();
        let now = Instant::now();

        tracker.observe(APP, udp(1), Some(100), now).unwrap();
        assert_eq!(
            tracker.observe(other, udp(1), Some(5), now).unwrap(),
            Appearance::New,
            "each application discovers the endpoint for itself"
        );

        assert_eq!(
            tracker
                .endpoints(APP)
                .next()
                .and_then(TrackedEndpoint::recent_bytes),
            Some(100)
        );
        assert_eq!(
            tracker
                .endpoints(other)
                .next()
                .and_then(TrackedEndpoint::recent_bytes),
            Some(5)
        );
    }

    #[test]
    fn an_endpoint_goes_idle_before_it_is_forgotten() {
        let (mut tracker, start) = monitored();
        tracker.observe(APP, udp(1), None, start).unwrap();
        let mut gone = Vec::new();

        tracker.sweep(start + Duration::from_secs(5), &mut gone);
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().liveness(),
            Liveness::Active
        );
        assert!(gone.is_empty());

        tracker.sweep(start + Duration::from_secs(30), &mut gone);
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().liveness(),
            Liveness::Idle
        );
        assert!(gone.is_empty(), "idle is not gone");
        assert_eq!(tracker.endpoint_count(APP), 1);
    }

    #[test]
    fn a_long_silent_endpoint_is_forgotten_and_reported() {
        let (mut tracker, start) = monitored();
        tracker.observe(APP, udp(1), None, start).unwrap();
        let mut gone = Vec::new();

        tracker.sweep(start + Duration::from_secs(121), &mut gone);

        assert_eq!(gone, vec![(APP, udp(1))]);
        assert_eq!(tracker.endpoint_count(APP), 0);
    }

    #[test]
    fn being_seen_again_rescues_an_idle_endpoint() {
        let (mut tracker, start) = monitored();
        tracker.observe(APP, udp(1), None, start).unwrap();
        let mut gone = Vec::new();

        tracker.sweep(start + Duration::from_secs(60), &mut gone);
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().liveness(),
            Liveness::Idle
        );

        // A loading screen ends and play resumes.
        tracker
            .observe(APP, udp(1), None, start + Duration::from_secs(100))
            .unwrap();
        tracker.sweep(start + Duration::from_secs(101), &mut gone);

        assert!(gone.is_empty());
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().liveness(),
            Liveness::Active
        );
    }

    #[test]
    fn a_sweep_reports_only_what_that_sweep_forgot() {
        let (mut tracker, start) = monitored();
        tracker.observe(APP, udp(1), None, start).unwrap();
        let mut gone = Vec::new();

        tracker.sweep(start + Duration::from_secs(121), &mut gone);
        assert_eq!(gone.len(), 1);

        // A caller reusing the vector must not see the same endpoint released twice.
        tracker.sweep(start + Duration::from_secs(122), &mut gone);
        assert!(gone.is_empty());
    }

    #[test]
    fn endpoints_past_the_cap_are_demoted_and_never_dropped() {
        let (mut tracker, now) = monitored();
        for last in 1..=20 {
            tracker.observe(APP, udp(last), None, now).unwrap();
        }
        let mut gone = Vec::new();
        tracker.sweep(now, &mut gone);

        assert_eq!(
            tracker.endpoint_count(APP),
            20,
            "the cap limits probing, not knowledge"
        );
        assert!(gone.is_empty());

        let active = tracker
            .endpoints(APP)
            .filter(|e| e.probing() == Probing::Active)
            .count();
        let demoted = tracker
            .endpoints(APP)
            .filter(|e| e.probing() == Probing::Demoted)
            .count();
        assert_eq!(active, 16);
        assert_eq!(demoted, 4);
    }

    #[test]
    fn the_busiest_endpoints_keep_the_probe_slots() {
        let policy = LifecyclePolicy {
            max_active_endpoints: 2,
            ..LifecyclePolicy::default()
        };
        let mut tracker = EndpointTracker::new(policy).unwrap();
        tracker.monitor(APP).unwrap();
        let now = Instant::now();

        tracker.observe(APP, udp(1), Some(10), now).unwrap();
        tracker.observe(APP, udp(2), Some(9_000), now).unwrap();
        tracker.observe(APP, udp(3), Some(5_000), now).unwrap();
        tracker.observe(APP, udp(4), Some(1), now).unwrap();

        tracker.sweep(now, &mut Vec::new());

        let promoted: Vec<_> = tracker
            .endpoints(APP)
            .filter(|e| e.probing() == Probing::Active)
            .map(TrackedEndpoint::key)
            .collect();
        assert_eq!(promoted, vec![udp(2), udp(3)]);
    }

    #[test]
    fn without_byte_counts_ranking_falls_back_to_recency() {
        // The state of a Windows machine that has not had its tracing setup: endpoints are
        // discovered from table polls, which see existence and not volume.
        let policy = LifecyclePolicy {
            max_active_endpoints: 2,
            ..LifecyclePolicy::default()
        };
        let mut tracker = EndpointTracker::new(policy).unwrap();
        tracker.monitor(APP).unwrap();
        let start = Instant::now();

        tracker.observe(APP, udp(1), None, start).unwrap();
        tracker
            .observe(APP, udp(2), None, start + Duration::from_secs(3))
            .unwrap();
        tracker
            .observe(APP, udp(3), None, start + Duration::from_secs(6))
            .unwrap();

        tracker.sweep(start + Duration::from_secs(6), &mut Vec::new());

        let promoted: Vec<_> = tracker
            .endpoints(APP)
            .filter(|e| e.probing() == Probing::Active)
            .map(TrackedEndpoint::key)
            .collect();
        assert_eq!(
            promoted,
            vec![udp(2), udp(3)],
            "the two most recently used win when volume is unknown"
        );
    }

    #[test]
    fn an_idle_endpoint_is_demoted_however_busy_it_was() {
        let (mut tracker, start) = monitored();
        tracker
            .observe(APP, udp(1), Some(9_000_000), start)
            .unwrap();
        tracker
            .observe(APP, udp(2), Some(1), start + Duration::from_secs(30))
            .unwrap();

        tracker.sweep(start + Duration::from_secs(30), &mut Vec::new());

        let heavy = tracker.endpoints(APP).next().unwrap();
        assert_eq!(heavy.key(), udp(1));
        assert_eq!(heavy.liveness(), Liveness::Idle);
        assert_eq!(
            heavy.probing(),
            Probing::Demoted,
            "past traffic does not keep a finished flow on the fast interval"
        );
    }

    #[test]
    fn unknown_throughput_is_none_and_measured_nothing_is_some_zero() {
        let (mut tracker, start) = monitored();
        tracker.observe(APP, udp(1), None, start).unwrap();
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().recent_bytes(),
            None,
            "a source that cannot count must not report zero throughput"
        );

        tracker.observe(APP, udp(2), Some(0), start).unwrap();
        let measured = tracker.endpoints(APP).nth(1).unwrap();
        assert_eq!(
            measured.recent_bytes(),
            Some(0),
            "a source that counted nothing has measured something"
        );
    }

    #[test]
    fn traffic_ages_out_of_the_window() {
        let (mut tracker, start) = monitored();
        tracker.observe(APP, udp(1), Some(1_000), start).unwrap();
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().recent_bytes(),
            Some(1_000)
        );

        // One window on, the bytes have shifted into the previous bucket but still count.
        tracker.sweep(start + Duration::from_secs(31), &mut Vec::new());
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().recent_bytes(),
            Some(1_000)
        );

        // Two windows on, they no longer describe current traffic.
        tracker.sweep(start + Duration::from_secs(75), &mut Vec::new());
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().recent_bytes(),
            Some(0),
            "the count is still known, it is simply nothing now"
        );
    }

    #[test]
    fn traffic_accumulates_within_a_window() {
        let (mut tracker, start) = monitored();
        for step in 0..5 {
            tracker
                .observe(
                    APP,
                    udp(1),
                    Some(100),
                    start + Duration::from_secs(step * 2),
                )
                .unwrap();
        }
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().recent_bytes(),
            Some(500)
        );
    }

    #[test]
    fn a_clock_that_goes_backwards_neither_panics_nor_pays_out() {
        let (mut tracker, start) = monitored();
        let later = start + Duration::from_secs(60);
        tracker.observe(APP, udp(1), Some(10), later).unwrap();

        // Sweeping with an earlier instant than the last sighting: saturating arithmetic
        // makes the silence zero rather than enormous, so nothing is forgotten.
        let mut gone = Vec::new();
        tracker.sweep(start, &mut gone);

        assert!(gone.is_empty());
        let endpoint = tracker.endpoints(APP).next().unwrap();
        assert_eq!(endpoint.liveness(), Liveness::Active);
        assert_eq!(endpoint.recent_bytes(), Some(10));
    }

    #[test]
    fn forgetting_an_app_releases_its_endpoints_and_its_slot() {
        let (mut tracker, now) = monitored();
        tracker.observe(APP, udp(1), None, now).unwrap();
        tracker.observe(APP, udp(2), None, now).unwrap();

        let released = tracker.forget(APP);

        assert_eq!(released, vec![udp(1), udp(2)]);
        assert_eq!(tracker.app_count(), 0);
        assert!(!tracker.is_monitored(APP));
        assert_eq!(tracker.forget(APP), Vec::new(), "forgetting twice is quiet");
    }

    #[test]
    fn a_new_endpoint_stays_conservative_until_it_is_ranked() {
        let (mut tracker, now) = monitored();
        tracker.observe(APP, udp(1), None, now).unwrap();
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().probing(),
            Probing::Demoted,
            "an unranked endpoint must not assume a fast interval"
        );

        tracker.sweep(now, &mut Vec::new());
        assert_eq!(
            tracker.endpoints(APP).next().unwrap().probing(),
            Probing::Active
        );
    }

    #[test]
    fn intervals_follow_the_probing_decision() {
        let policy = LifecyclePolicy::default();
        assert_eq!(policy.interval_for(Probing::Active), Duration::from_secs(1));
        assert_eq!(
            policy.interval_for(Probing::Demoted),
            Duration::from_secs(10)
        );
        assert!(
            policy.interval_for(Probing::Demoted) > policy.interval_for(Probing::Active),
            "demotion must stretch the interval, never stop probing"
        );
    }

    #[test]
    fn sweeping_an_empty_tracker_is_harmless() {
        let mut tracker = tracker();
        let mut gone = vec![(APP, udp(9))];
        tracker.sweep(Instant::now(), &mut gone);
        assert!(gone.is_empty());
    }

    #[test]
    fn first_seen_survives_later_sightings() {
        let (mut tracker, start) = monitored();
        tracker.observe(APP, udp(1), None, start).unwrap();
        tracker
            .observe(APP, udp(1), None, start + Duration::from_secs(5))
            .unwrap();

        let endpoint = tracker.endpoints(APP).next().unwrap();
        assert_eq!(endpoint.first_seen(), start);
        assert_eq!(endpoint.last_seen(), start + Duration::from_secs(5));
    }
}
