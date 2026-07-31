//! Per-application endpoint monitoring: turning discovery into probe targets.
//!
//! [`AppMonitor`] is the join between three things that otherwise know nothing about each
//! other: [`EndpointTracker`], which decides what an application is talking to and how
//! interesting each endpoint is; [`TargetRegistry`], which hands out probe handles and
//! refuses to probe one address twice; and the probe runner, which is driven from here by
//! a list of [`TargetChange`] rather than being called directly.
//!
//! # The registry is borrowed, not owned
//!
//! One session has **one** registry, shared with the baselines, and it is passed into the
//! methods that need it. Two registries would hand out the same [`TargetId`] twice, and
//! since both features feed a single probe runner — there is one global probe budget, so
//! there can only be one runner — a baseline's measurement would land on an application's
//! endpoint. Sharing it also buys what the registry was built for: an address that is both
//! a baseline and a game server is probed once and answers both.
//!
//! Like [`crate::monitor`] it reads no clock and opens no socket — callers pass `now` in —
//! so a session of applications appearing, going quiet and disappearing is replayed in a
//! test in microseconds, on any operating system.
//!
//! # Why changes are returned instead of applied
//!
//! The runner lives inside its own async loop and is reachable only by message. Returning
//! the decisions makes them inspectable: a test asserts *what* would be asked of the probe
//! engine without a probe engine existing, which is the only practical way to check that
//! discovery does not, say, re-register an endpoint every second.
//!
//! # One address, several applications
//!
//! Two monitored applications can talk to the same endpoint — a shared CDN, a launcher and
//! its game. The registry deduplicates by address so it is probed once, and this module
//! reference-counts the users so that one application letting go does not stop a
//! measurement the other is still watching. The interval is the shortest any user wants:
//! if one application considers the endpoint important, it is probed at that rate and the
//! other benefits.
//!
//! **Egress is where deduplication can lie.** A probe follows the local address it is bound
//! to, so if two applications reach one endpoint through different interfaces — the normal
//! case for a per-process accelerator — a single probe cannot represent both. That is
//! recorded as a conflict on the endpoint rather than silently averaged away, because
//! CLAUDE.md's rule for this case is to disclose, never to mismeasure.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::IpAddr;
use std::time::{Duration, Instant};

use nm_core::address::{AddressClass, AddressPolicy};
use nm_core::endpoint::{
    AppId, EndpointKey, EndpointTracker, LifecyclePolicy, Liveness, Probing, TrackedEndpoint,
};
use nm_core::health::HealthThresholds;
use nm_core::history::SampleHistory;
use nm_core::sample::{ProbeSample, Rtt};
use nm_core::stats::WindowStats;
use nm_core::target::{TargetAddress, TargetId, TargetRegistry, TargetTag};
use nm_probes::probe::ProbeKind;

use crate::Error;

/// How many samples are retained per monitored endpoint.
///
/// At the enforced ceiling — five applications of sixteen actively probed endpoints — this
/// is at most 80 histories. Each is a fixed ring of 4-byte round-trip times and their
/// stamps, so the whole per-application history stays in the low hundreds of kilobytes
/// however long the session runs.
pub const HISTORY_CAPACITY: usize = 120;

/// How many samples of an endpoint's history the sparkline carries.
///
/// Fewer than a baseline's, and for two reasons. An endpoint is probed once a second
/// rather than once every few seconds, so thirty points is half a minute — the same span
/// the traffic ranking uses, and long enough to see a loss burst. And there can be eighty
/// of these at the enforced ceiling against the baselines' handful, so the series is the
/// one part of this payload whose size has to be argued for.
pub const SERIES_POINTS: usize = 30;

/// Something the probe engine must be told.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TargetChange {
    /// Begin probing a newly discovered endpoint.
    Register {
        /// Handle to track it by.
        id: TargetId,
        /// Where it lives.
        address: TargetAddress,
        /// Local address the probes must egress from, so they follow the application's
        /// own route through any tunnel or accelerator.
        source: Option<IpAddr>,
        /// How often to probe it to begin with.
        interval: Duration,
    },
    /// Probe an existing target at a different rate.
    SetInterval {
        /// Which target.
        id: TargetId,
        /// Its new interval.
        interval: Duration,
    },
    /// Probe an existing target from a different local address.
    ///
    /// The monitored flow moved — the user turned a VPN or an accelerator on mid-session —
    /// so the probe has to move with it or it measures a route the application is no longer
    /// taking.
    SetSource {
        /// Which target.
        id: TargetId,
        /// The address its probes must now leave from.
        source: Option<IpAddr>,
    },
    /// Stop probing an endpoint no monitored application uses any more.
    Unregister {
        /// Which target.
        id: TargetId,
    },
}

/// Everything known about one application's use of one endpoint.
// Four flags, and the lint that objects is aimed at positional arguments a caller can
// transpose. These are named fields set individually from four unrelated sources — the
// address policy, the registry, and two separate reports from the probe engine — and
// folding them into an enum would invent states that cannot occur, such as "tunnelled" and
// "unmeasurable" being mutually exclusive when an endpoint is routinely both.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
struct Entry {
    id: Option<TargetId>,
    source: Option<IpAddr>,
    /// How often *this* application would like the endpoint probed.
    ///
    /// Held per application rather than per target: the target's interval is the shortest
    /// of these across its users, and it has to be recomputed from all of them so that an
    /// endpoint can be demoted again once every user has lost interest.
    desired_interval: Duration,
    egress_conflict: bool,
    tunnelled: bool,
    measurable: bool,
    probe_kind: Option<ProbeKind>,
    filtering_confirmed: bool,
    history: SampleHistory,
}

/// The applications sharing one probe target.
#[derive(Debug, Clone, Default)]
struct TargetUsers {
    members: BTreeSet<(AppId, EndpointKey)>,
    /// The egress address last asked of the probe engine.
    ///
    /// What the engine currently believes, never what the applications want — the wanted
    /// value is recomputed from the members on every sweep, which is what lets a flow that
    /// moves take its probes with it.
    source: Option<IpAddr>,
    /// Whether a source has ever been stated for this target.
    ///
    /// Distinguishes "the engine was told to bind nowhere" from "the engine has not been
    /// told anything yet", which are the same [`None`] and mean different things.
    source_stated: bool,
    /// Whether the probe belongs to another feature, such as a baseline.
    ///
    /// Its binding was chosen for that feature's purpose and is not ours to change: a
    /// baseline silently re-bound to a game's interface would stop measuring what the
    /// dashboard claims it measures.
    foreign: bool,
    /// The interval last asked of the probe engine.
    ///
    /// Kept so that a sweep only speaks when something changed. Restating every endpoint's
    /// interval once a second would put a hundred messages a second onto a channel sized
    /// for occasional commands.
    interval: Option<Duration>,
}

/// Per-application endpoint state and the probe targets it implies.
#[derive(Debug)]
pub struct AppMonitor {
    tracker: EndpointTracker,
    entries: BTreeMap<(AppId, EndpointKey), Entry>,
    users: HashMap<TargetId, TargetUsers>,
    policy: AddressPolicy,
    thresholds: HealthThresholds,
    window: Duration,
    /// Reused by every sweep so the steady state allocates nothing for expiry.
    gone: Vec<(AppId, EndpointKey)>,
}

impl AppMonitor {
    /// Creates a monitor.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Core`] for a lifecycle policy that cannot be satisfied.
    pub fn new(
        policy: AddressPolicy,
        lifecycle: LifecyclePolicy,
        thresholds: HealthThresholds,
        window: Duration,
    ) -> Result<Self, Error> {
        Ok(Self {
            tracker: EndpointTracker::new(lifecycle)?,
            entries: BTreeMap::new(),
            users: HashMap::new(),
            policy,
            thresholds,
            window,
            gone: Vec::new(),
        })
    }

    /// Starts monitoring an application.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Core`] wrapping [`nm_core::Error::TooManyApps`] at the cap.
    pub fn monitor(&mut self, app: AppId) -> Result<(), Error> {
        self.tracker.monitor(app)?;
        Ok(())
    }

    /// Whether an application is being monitored.
    #[must_use]
    pub fn is_monitored(&self, app: AppId) -> bool {
        self.tracker.is_monitored(app)
    }

    /// How many applications are monitored.
    #[must_use]
    pub fn app_count(&self) -> usize {
        self.tracker.app_count()
    }

    /// Stops monitoring an application and releases the targets nobody else uses.
    pub fn forget(&mut self, registry: &mut TargetRegistry, app: AppId) -> Vec<TargetChange> {
        let mut changes = Vec::new();
        for key in self.tracker.forget(app) {
            self.release(registry, app, key, &mut changes);
        }
        changes
    }

    /// Records that `app` was seen using `endpoint`.
    ///
    /// `source` is the local address the application's own flow egresses from; probes are
    /// bound to it so they traverse the same interface, tunnel or accelerator. `bytes` is
    /// [`None`] where the discovery source cannot count them.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Core`] wrapping [`nm_core::Error::UnknownApp`] when the application
    /// is not monitored.
    pub fn observe(
        &mut self,
        app: AppId,
        endpoint: EndpointKey,
        source: Option<IpAddr>,
        bytes: Option<u64>,
        now: Instant,
    ) -> Result<(), Error> {
        self.tracker.observe(app, endpoint, bytes, now)?;

        if let Some(entry) = self.entries.get_mut(&(app, endpoint)) {
            // A flow that moved to a different local address has been re-routed — the user
            // turning a VPN on mid-session is exactly this. Take the new address so probes
            // follow the application rather than the route it used to have.
            if source.is_some() && entry.source != source {
                entry.source = source;
            }
            return Ok(());
        }

        let tunnelled = self.policy.classify(endpoint.address.ip()) == AddressClass::TunnelSentinel;
        let history = SampleHistory::new(HISTORY_CAPACITY).map_err(nm_probes::Error::Core)?;
        self.entries.insert(
            (app, endpoint),
            Entry {
                id: None,
                source,
                desired_interval: self.tracker.policy().active_interval,
                egress_conflict: false,
                tunnelled,
                measurable: true,
                probe_kind: None,
                filtering_confirmed: false,
                history,
            },
        );
        Ok(())
    }

    /// Ages everything, then reports what the probe engine must be told.
    ///
    /// Call once per discovery cycle, after the observations from that cycle.
    pub fn sweep(&mut self, registry: &mut TargetRegistry, now: Instant) -> Vec<TargetChange> {
        let mut changes = Vec::new();

        let mut gone = std::mem::take(&mut self.gone);
        self.tracker.sweep(now, &mut gone);
        for (app, key) in gone.drain(..) {
            self.release(registry, app, key, &mut changes);
        }
        self.gone = gone;

        // What each application wants, gathered before anything is registered so that the
        // interval of a shared target is decided by all of its users at once.
        let mut wanted: Vec<(AppId, EndpointKey, Duration)> = Vec::new();
        for app in self.monitored_apps() {
            for tracked in self.tracker.endpoints(app) {
                let interval = self.tracker.policy().interval_for(tracked.probing());
                wanted.push((app, tracked.key(), interval));
            }
        }

        for (app, key, interval) in wanted {
            self.register(registry, app, key, interval, &mut changes);
        }
        self.retune(&mut changes);
        changes
    }

    /// Recomputes each target's cadence and egress from every application that uses it.
    ///
    /// Separate from registration because both answers depend on *all* of a target's users:
    /// the shortest interval anyone wants wins, so one application caring about an endpoint
    /// keeps it well measured and the others benefit — and the endpoint can slow down again
    /// only once every user has lost interest.
    ///
    /// The egress is the same shape of question with a harder answer. Where the users agree,
    /// that address is what the probe binds to, and it is restated whenever it changes — a
    /// flow that moves takes its probes with it. Where they disagree, one probe cannot
    /// represent both routes, so the disagreement is recorded on the endpoints and the
    /// binding is left where it is rather than flipping between two applications' routes
    /// once a second.
    fn retune(&mut self, changes: &mut Vec<TargetChange>) {
        for (id, users) in &mut self.users {
            let mut shortest: Option<Duration> = None;
            let mut sources: BTreeSet<IpAddr> = BTreeSet::new();
            for member in &users.members {
                let Some(entry) = self.entries.get(member) else {
                    continue;
                };
                shortest = Some(match shortest {
                    Some(current) => current.min(entry.desired_interval),
                    None => entry.desired_interval,
                });
                if let Some(source) = entry.source {
                    sources.insert(source);
                }
            }

            let Some(shortest) = shortest else {
                continue;
            };
            if users.interval != Some(shortest) {
                users.interval = Some(shortest);
                changes.push(TargetChange::SetInterval {
                    id: *id,
                    interval: shortest,
                });
            }

            // One address, one probe, one binding — so the question is who that binding
            // serves. Where the users agree (the ordinary case, and the only case for a
            // single application) it follows them, moving when they move. Where they
            // disagree, or where the probe belongs to another feature entirely, it stays
            // put and the applications it does *not* serve are told so.
            let agreed = (!users.foreign && sources.len() <= 1)
                .then(|| sources.iter().next().copied())
                .filter(|_| !users.foreign);

            if let Some(agreed) = agreed {
                if !users.source_stated || users.source != agreed {
                    users.source = agreed;
                    users.source_stated = true;
                    changes.push(TargetChange::SetSource {
                        id: *id,
                        source: agreed,
                    });
                }
            }

            Self::disclose_mismatches(
                &mut self.entries,
                &users.members,
                users.source,
                users.foreign,
            );
        }
    }

    /// Tells each user whether the one probe actually follows its route.
    ///
    /// Only the applications the binding does not serve are marked. Flagging every user of
    /// a contested endpoint would warn the one whose figure is correct, and a warning that
    /// covers the innocent is one nobody reads. An application that asked for no particular
    /// egress is not mismeasured either — it never made a claim about its route.
    ///
    /// `foreign` marks a probe another feature owns: it was bound for that feature's
    /// purpose, so any application wanting its own egress is disclosed regardless of what
    /// the addresses happen to be.
    fn disclose_mismatches(
        entries: &mut BTreeMap<(AppId, EndpointKey), Entry>,
        members: &BTreeSet<(AppId, EndpointKey)>,
        bound: Option<IpAddr>,
        foreign: bool,
    ) {
        for member in members {
            let Some(entry) = entries.get_mut(member) else {
                continue;
            };
            entry.egress_conflict = match entry.source {
                None => false,
                Some(source) => foreign || Some(source) != bound,
            };
        }
    }

    /// Records what an application wants, and registers the endpoint if it is new.
    fn register(
        &mut self,
        registry: &mut TargetRegistry,
        app: AppId,
        key: EndpointKey,
        interval: Duration,
        changes: &mut Vec<TargetChange>,
    ) {
        let Some(entry) = self.entries.get_mut(&(app, key)) else {
            return;
        };
        entry.desired_interval = interval;

        let address = TargetAddress::with_port(key.address.ip(), key.address.port());
        // Asked before inserting: the registry hands back the existing handle for an
        // address something else already probes, and re-registering that with the runner
        // would reset a fallback chain and a failure history that belong to another
        // feature.
        let adopted = entry.id.is_none() && registry.find(address).is_some();
        let id = if let Some(id) = entry.id {
            id
        } else {
            let Ok(id) = registry.insert(address, TargetTag::AppEndpoint) else {
                // Only an exhausted 32-bit identifier space reaches this, which needs four
                // billion endpoints in one session. The endpoint stays tracked and unprobed
                // rather than taking the process down.
                return;
            };
            entry.id = Some(id);
            id
        };
        let source = entry.source;

        let users = self.users.entry(id).or_default();
        let first = users.members.is_empty();
        users.members.insert((app, key));

        if first {
            users.interval = Some(interval);
            users.foreign = adopted;
            if adopted {
                // Another feature already probes this address, so only its cadence is ours
                // to change. Re-registering would reset a fallback chain and a failure
                // history that belong to someone else.
                changes.push(TargetChange::SetInterval { id, interval });
            } else {
                users.source = source;
                users.source_stated = true;
                changes.push(TargetChange::Register {
                    id,
                    address,
                    source,
                    interval,
                });
            }
        }
        // Everything else — the cadence, the egress, and whether the users can agree on one
        // — is decided in `retune` from *all* of a target's members at once. Deciding it
        // here, from whichever application happened to be swept first, is what used to
        // report a single application as conflicting with itself the moment its own egress
        // address became known.
    }

    /// Drops one application's use of an endpoint, releasing the target if it was the last.
    fn release(
        &mut self,
        registry: &mut TargetRegistry,
        app: AppId,
        key: EndpointKey,
        changes: &mut Vec<TargetChange>,
    ) {
        let Some(entry) = self.entries.remove(&(app, key)) else {
            return;
        };
        let Some(id) = entry.id else {
            return;
        };

        let Some(users) = self.users.get_mut(&id) else {
            return;
        };
        users.members.remove(&(app, key));
        if !users.members.is_empty() {
            return;
        }

        self.users.remove(&id);
        // Only this feature's claim on the address is dropped. An address that is also on
        // a baseline list keeps its target, and its handle — so the probe carries on and
        // the dashboard never notices an application let go of it.
        let removed = registry.untag(id, TargetTag::AppEndpoint);
        if removed {
            changes.push(TargetChange::Unregister { id });
        }
    }

    /// Records a probe result against every application using that target.
    pub fn record(&mut self, id: TargetId, sample: ProbeSample) {
        for key in self.members_of(id) {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.history.record(sample);
            }
        }
    }

    /// Notes what the probe engine is currently doing with a target.
    pub fn note_probe_state(
        &mut self,
        id: TargetId,
        kind: Option<ProbeKind>,
        filtering_confirmed: bool,
        measurable: bool,
    ) {
        for key in self.members_of(id) {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.probe_kind = kind;
                entry.filtering_confirmed = filtering_confirmed;
                entry.measurable = measurable;
            }
        }
    }

    /// Marks a target as one no probe kind can honestly measure.
    pub fn note_unmeasurable(&mut self, id: TargetId) {
        for key in self.members_of(id) {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.measurable = false;
                entry.probe_kind = None;
            }
        }
    }

    /// The applications and endpoints sharing a target.
    fn members_of(&self, id: TargetId) -> Vec<(AppId, EndpointKey)> {
        self.users
            .get(&id)
            .map(|users| users.members.iter().copied().collect())
            .unwrap_or_default()
    }

    /// The monitored applications, in a stable order.
    fn monitored_apps(&self) -> Vec<AppId> {
        let mut apps: Vec<AppId> = self
            .entries
            .keys()
            .map(|(app, _)| *app)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        apps.retain(|app| self.tracker.is_monitored(*app));
        apps
    }

    /// Every endpoint of an application, with its measurements, in a stable order.
    #[must_use]
    pub fn endpoints(&self, app: AppId, now: Instant) -> Vec<EndpointReport> {
        self.tracker
            .endpoints(app)
            .filter_map(|tracked| {
                let entry = self.entries.get(&(app, tracked.key()))?;
                Some(EndpointReport::build(
                    tracked,
                    entry,
                    now,
                    self.window,
                    &self.thresholds,
                ))
            })
            .collect()
    }

    /// How many endpoints an application has, probed or demoted.
    #[must_use]
    pub fn endpoint_count(&self, app: AppId) -> usize {
        self.tracker.endpoint_count(app)
    }

    /// The span every figure in an [`EndpointReport`] is computed over.
    #[must_use]
    pub const fn window(&self) -> Duration {
        self.window
    }

    /// The span [`EndpointReport::recent_bytes`] accumulates over.
    ///
    /// Reported alongside the figure rather than divided into it: the count covers between
    /// one and two of these windows, so turning it into a rate would invent a precision the
    /// measurement does not have.
    #[must_use]
    pub fn traffic_window(&self) -> Duration {
        self.tracker.policy().traffic_window
    }
}

/// One endpoint of one application, as the layer above sees it.
///
/// Deliberately per endpoint and never rolled up: within one application some endpoints
/// stay clean while others lose packets or go unreachable, commonly at the same moment
/// because they sit in different networks. A single verdict per application would read as
/// "the game is broken" when the game is fine.
// Same reasoning as `Entry`: independent named facts, not transposable arguments.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EndpointReport {
    /// Which endpoint.
    pub key: EndpointKey,
    /// Whether the application is currently using it.
    pub liveness: Liveness,
    /// Whether it is probed at the normal interval or the long one.
    pub probing: Probing,
    /// Bytes seen recently, or [`None`] where no source counts them.
    pub recent_bytes: Option<u64>,
    /// Local address its probes egress from.
    pub source: Option<IpAddr>,
    /// Whether two applications reach it by different routes, so one probe cannot
    /// represent both.
    pub egress_conflict: bool,
    /// Whether a local tunnel remaps it, making the figure end-to-end through that tunnel
    /// rather than a round trip to the server.
    pub tunnelled: bool,
    /// Whether anything can honestly measure it at all.
    pub measurable: bool,
    /// The probe kind its figure came from.
    pub probe_kind: Option<ProbeKind>,
    /// Whether a probe kind has been *proven* filtered here.
    pub filtering_confirmed: bool,
    /// Its statistics over the health window.
    pub stats: WindowStats,
    /// The verdict those statistics imply.
    pub health: nm_core::health::Health,
    /// Seconds before `now` for each point of the series — negative, ascending.
    ///
    /// A real time axis rather than sample indices: an idle endpoint is probed ten times
    /// less often, so evenly spaced points would draw a chart that lies about when things
    /// happened.
    pub series_age_secs: Vec<f64>,
    /// Round-trip time at each point, or [`None`] where the probe did not come back.
    pub series_rtt_ms: Vec<Option<f64>>,
}

impl EndpointReport {
    fn build(
        tracked: &TrackedEndpoint,
        entry: &Entry,
        now: Instant,
        window: Duration,
        thresholds: &HealthThresholds,
    ) -> Self {
        let stats = entry.history.stats_for_window(now, window);
        // Passive evidence is folded in here, where the two sources of knowledge finally
        // meet: the probe engine cannot see the flow counters, and the endpoint tracker
        // cannot see the probes. A game's match server answers nothing and carries every
        // packet of the match, and only this join can tell that apart from a dead host.
        let carrying = tracked.recent_bytes().is_some_and(|bytes| bytes > 0);
        let health = nm_core::health::with_passive_evidence(
            thresholds.health_of(&stats),
            carrying,
            entry.measurable,
        );

        let mut series_age_secs = Vec::with_capacity(SERIES_POINTS);
        let mut series_rtt_ms = Vec::with_capacity(SERIES_POINTS);
        for sample in entry.history.recent(SERIES_POINTS) {
            series_age_secs.push(-now.saturating_duration_since(sample.at).as_secs_f64());
            series_rtt_ms.push(sample.outcome.rtt().map(Rtt::as_millis_f64));
        }

        Self {
            key: tracked.key(),
            series_age_secs,
            series_rtt_ms,
            liveness: tracked.liveness(),
            probing: tracked.probing(),
            recent_bytes: tracked.recent_bytes(),
            source: entry.source,
            egress_conflict: entry.egress_conflict,
            tunnelled: entry.tunnelled,
            measurable: entry.measurable,
            probe_kind: entry.probe_kind,
            filtering_confirmed: entry.filtering_confirmed,
            health,
            stats,
        }
    }
}
