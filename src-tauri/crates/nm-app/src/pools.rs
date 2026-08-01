//! Game reference pools: the bundled seeds, the endpoints this machine has learned, and
//! the trickle that keeps them measured.
//!
//! [`nm_core::pool`] holds the model — what a pool is, how it ages, what its answers add up
//! to. This module is the part that touches the world: it reads the bundled seed files,
//! remembers the servers the user's own games connected to, and turns the whole thing into
//! instructions for the probe engine.
//!
//! # What a pool is for
//!
//! Nothing else in the product can tell a game whose servers are down from a game the user
//! cannot reach. Both look like an endpoint that stopped answering, and the two call for
//! opposite actions — wait, or try a VPN. Several targets belonging to the same game's
//! infrastructure, in different places, answer it: one silent target is a path, all of them
//! silent while the baselines are clean is the game.
//!
//! # The trickle
//!
//! Pool targets are registered with the shared probe engine at [`POOL_INTERVAL`] and left
//! there while the game is monitored — a whole eight-target pool costs one probe every
//! thirty-eight seconds. The engine's own scheduler spreads them, its fallback chain gives
//! each one a history, and the global rate cap covers them like everything else. A pool
//! stops the moment the user stops watching the game.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use nm_core::endpoint::AppId;
use nm_core::health::HealthThresholds;
use nm_core::history::SampleHistory;
use nm_core::pool::{PoolEntry, PoolPolicy, PoolReading, PoolSource, ReferencePool};
use nm_core::sample::ProbeSample;
use nm_core::stats::WindowStats;
use nm_core::target::{TargetAddress, TargetId, TargetRegistry, TargetTag};
use serde::{Deserialize, Serialize};

use crate::apps::TargetChange;
use crate::Error;

/// The schema version this build understands, for both the bundled and the stored file.
const SUPPORTED_SCHEMA: u32 = 1;

/// Name of the learned-endpoint file inside the application's configuration directory.
pub const FILE_NAME: &str = "game-servers.json";

/// How often one pool target is probed.
///
/// Slow on purpose, and the arithmetic is the point: a pool answers a question that changes
/// on the scale of an outage, not of a packet. At five minutes each, the thirty-two entries
/// a pool may hold produce an answer every nine seconds between them — fast enough for the
/// share of the pool that is answering to mean something within a minute, and about a tenth
/// of a probe a second per game against the product's cap of thirty-two.
pub const POOL_INTERVAL: Duration = Duration::from_secs(300);

/// How many probes of one pool target are retained.
///
/// At [`POOL_INTERVAL`] this is an hour of history per target.
const HISTORY_CAPACITY: usize = 12;

/// The span a pool member's verdict is computed over.
///
/// Four intervals: enough that a lost probe is not an outage and short enough that a real
/// one is visible while the user is still looking at the screen.
const POOL_WINDOW: Duration = Duration::from_secs(300 * 4);

/// The bundled pool files.
///
/// A new pool is a new file and one line here — data, not a code path.
const BUNDLED_POOLS: &[&str] = &[include_str!(
    "../../../../assets/targets/pools/valve-sdr.json"
)];

/// One reference target exactly as written in a bundled pool file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedPoolTarget {
    /// Stable identifier, unique within its pool.
    pub id: String,
    /// The operator's own name for where it sits. A proper noun, shown as written.
    pub label: String,
    /// An IP literal. Pools are addresses rather than names on purpose — see
    /// `assets/targets/README.md`.
    pub address: String,
    /// Port for the probe kinds that need one, when the operator publishes a reachable one.
    pub port: Option<u16>,
}

/// A parsed and validated bundled pool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundledPool {
    /// Schema version of the file.
    pub schema_version: u32,
    /// Stable identifier of the pool.
    pub id: String,
    /// The operator's name for the infrastructure it describes.
    pub label: String,
    /// Which application presets this pool seeds, by preset identifier.
    ///
    /// A list rather than one, because one relay network really does serve several titles —
    /// Counter-Strike and Dota reach the player through the same one — and duplicating the
    /// file per game would guarantee the copies drift apart.
    pub applications: Vec<String>,
    /// The reference targets.
    pub targets: Vec<ListedPoolTarget>,
}

impl BundledPool {
    /// Parses and validates one pool file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetList`] for malformed JSON, an unsupported schema version, a
    /// pool that seeds nothing or holds nothing, a duplicate target identifier, or an
    /// address that is not an IP literal.
    pub fn parse(json: &str) -> Result<Self, Error> {
        let complain = |reason: String| Error::TargetList {
            list: "pool".to_owned(),
            reason,
        };

        let pool: Self =
            serde_json::from_str(json).map_err(|source| complain(source.to_string()))?;

        if pool.schema_version != SUPPORTED_SCHEMA {
            return Err(complain(format!(
                "schema version {} is not supported (this build understands {SUPPORTED_SCHEMA})",
                pool.schema_version
            )));
        }
        if pool.applications.is_empty() {
            return Err(complain(format!("pool {:?} seeds no application", pool.id)));
        }
        if pool.targets.is_empty() {
            return Err(complain(format!("pool {:?} has no targets", pool.id)));
        }
        for (index, target) in pool.targets.iter().enumerate() {
            if pool.targets[..index]
                .iter()
                .any(|seen| seen.id == target.id)
            {
                return Err(complain(format!("duplicate target id {:?}", target.id)));
            }
            if target.address.parse::<std::net::IpAddr>().is_err() {
                return Err(complain(format!(
                    "target {:?} is not an address literal",
                    target.id
                )));
            }
        }
        Ok(pool)
    }

    /// The seed entries this pool contributes.
    #[must_use]
    pub fn seeds(&self) -> Vec<PoolEntry> {
        self.targets
            .iter()
            .filter_map(|target| {
                let ip = target.address.parse().ok()?;
                Some(PoolEntry {
                    address: match target.port {
                        Some(port) => TargetAddress::with_port(ip, port),
                        None => TargetAddress::icmp(ip),
                    },
                    label: Some(target.label.clone()),
                    source: PoolSource::Bundled,
                    last_seen: None,
                })
            })
            .collect()
    }
}

/// Every bundled pool, indexed by the preset it seeds.
#[derive(Debug, Clone, Default)]
pub struct PoolSeeds {
    by_preset: HashMap<String, Vec<PoolEntry>>,
}

impl PoolSeeds {
    /// Loads every bundled pool.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetList`] when a bundled file does not validate, which a test
    /// makes sure cannot reach a release.
    pub fn bundled() -> Result<Self, Error> {
        let mut by_preset: HashMap<String, Vec<PoolEntry>> = HashMap::new();
        for json in BUNDLED_POOLS {
            let pool = BundledPool::parse(json)?;
            let seeds = pool.seeds();
            for preset in &pool.applications {
                by_preset
                    .entry(preset.clone())
                    .or_default()
                    .extend(seeds.iter().cloned());
            }
        }
        Ok(Self { by_preset })
    }

    /// An empty set, for a build whose pools could not be read.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The seeds for one application preset, if any are bundled.
    ///
    /// Most titles have none, and that is a real state rather than a gap to paper over:
    /// their operators publish no reachable reference address, so the pool is whatever this
    /// machine has learned, and until it has learned something the page says so.
    #[must_use]
    pub fn for_preset(&self, preset: &str) -> Vec<PoolEntry> {
        self.by_preset.get(preset).cloned().unwrap_or_default()
    }
}

/// One learned endpoint as it is written to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredEndpoint {
    address: String,
    port: Option<u16>,
    /// Seconds since the Unix epoch. Wall clock, because this span outlives the process.
    last_seen_unix: u64,
}

/// The learned-endpoint file as it may actually be found on disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPools {
    #[serde(default)]
    schema_version: u32,
    /// Learned endpoints by application preset identifier.
    ///
    /// Keyed on the preset rather than on the application's runtime identity, which is
    /// assigned per session and means nothing across a restart.
    #[serde(default)]
    pools: HashMap<String, Vec<StoredEndpoint>>,
}

/// What this machine has learned about where each game's servers are.
///
/// # This writes a record of where the user plays
///
/// It is a file of addresses a game connected to, kept on the local disk and sent nowhere —
/// but for this product's audience that is still worth being deliberate about, so it is
/// stated in Settings and it can be turned off. With it off the pools fall back to the
/// bundled seeds, which is a weaker answer for the titles that have them and no answer at
/// all for the ones that do not; the page says which.
#[derive(Debug, Clone, Default)]
pub struct LearnedPools {
    by_preset: HashMap<String, Vec<(TargetAddress, SystemTime)>>,
}

impl LearnedPools {
    /// Reads the learned endpoints from `path`.
    ///
    /// A missing file is a first run. A file that cannot be read or understood yields
    /// nothing learned and is **left exactly as it is** — the same bargain the settings file
    /// makes, for the same reason: a parsing bug of ours must not destroy something a fixed
    /// build would have read.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(stored) = serde_json::from_str::<StoredPools>(&raw) else {
            return Self::default();
        };
        if stored.schema_version != SUPPORTED_SCHEMA {
            return Self::default();
        }

        let mut by_preset = HashMap::new();
        for (preset, endpoints) in stored.pools {
            let entries: Vec<(TargetAddress, SystemTime)> = endpoints
                .into_iter()
                .filter_map(|endpoint| {
                    let ip = endpoint.address.parse().ok()?;
                    let address = match endpoint.port {
                        Some(port) => TargetAddress::with_port(ip, port),
                        None => TargetAddress::icmp(ip),
                    };
                    let seen = SystemTime::UNIX_EPOCH
                        .checked_add(Duration::from_secs(endpoint.last_seen_unix))?;
                    Some((address, seen))
                })
                .collect();
            if !entries.is_empty() {
                by_preset.insert(preset, entries);
            }
        }
        Self { by_preset }
    }

    /// What was learned for one preset.
    #[must_use]
    pub fn for_preset(&self, preset: &str) -> Vec<(TargetAddress, SystemTime)> {
        self.by_preset.get(preset).cloned().unwrap_or_default()
    }

    /// Replaces what is held for one preset.
    pub fn set(&mut self, preset: &str, entries: Vec<(TargetAddress, SystemTime)>) {
        if entries.is_empty() {
            self.by_preset.remove(preset);
        } else {
            self.by_preset.insert(preset.to_owned(), entries);
        }
    }

    /// Whether anything is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_preset.is_empty()
    }

    /// Writes the learned endpoints to `path`, creating the directory if it is missing.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Persistence`] if the directory or the file cannot be written. There
    /// is nothing useful to retry: the pools still work for this session, and the next one
    /// starts from the bundled seeds.
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        let mut pools = HashMap::new();
        for (preset, entries) in &self.by_preset {
            let stored: Vec<StoredEndpoint> = entries
                .iter()
                .filter_map(|(address, seen)| {
                    Some(StoredEndpoint {
                        address: address.ip.to_string(),
                        port: address.port,
                        last_seen_unix: seen.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs(),
                    })
                })
                .collect();
            pools.insert(preset.clone(), stored);
        }

        let stored = StoredPools {
            schema_version: SUPPORTED_SCHEMA,
            pools,
        };
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory).map_err(|_| Error::Persistence)?;
        }
        let json = serde_json::to_string_pretty(&stored).map_err(|_| Error::Persistence)?;
        std::fs::write(path, json).map_err(|_| Error::Persistence)
    }

    /// Where the file lives beside the settings.
    #[must_use]
    pub fn path_beside(settings: &Path) -> PathBuf {
        settings.with_file_name(FILE_NAME)
    }
}

/// One monitored application's pool, its handles and its histories.
#[derive(Debug, Clone)]
struct Tracked {
    preset: String,
    pool: ReferencePool,
    /// Handle and history per registered target, in the order the pool listed them.
    probed: Vec<(TargetId, SampleHistory)>,
    /// Whether the registered set still matches the pool. Set by [`ReferencePool::observe`]
    /// landing a new address, cleared once the change has been handed to the probe engine.
    dirty: bool,
}

/// Every monitored application's reference pool.
///
/// Reads no clock and opens no socket: callers pass both `now` (monotonic, for the probe
/// windows) and `wall` (for the ages that outlive the process). It returns
/// [`TargetChange`]s rather than talking to the probe engine, for the reason
/// `crate::apps::AppMonitor` does — the engine lives inside its own loop and is reachable
/// only by message, and returning the instructions is what makes them assertable in a test
/// with no engine at all.
#[derive(Debug, Clone)]
pub struct PoolMonitor {
    seeds: PoolSeeds,
    policy: PoolPolicy,
    thresholds: HealthThresholds,
    tracked: HashMap<AppId, Tracked>,
    /// Which application each registered handle belongs to.
    owners: HashMap<TargetId, AppId>,
    /// Whether learned endpoints may be recorded at all.
    remember: bool,
    /// Whether what is remembered has changed since it was last written out.
    learned_dirty: bool,
}

impl PoolMonitor {
    /// Creates a monitor over the bundled seeds.
    ///
    /// `remember` is the user's choice about whether endpoints their games connect to are
    /// recorded. With it off, [`PoolMonitor::observe`] does nothing at all and the pools are
    /// exactly the bundled seeds.
    #[must_use]
    pub fn new(seeds: PoolSeeds, thresholds: HealthThresholds, remember: bool) -> Self {
        Self {
            seeds,
            policy: PoolPolicy::default(),
            thresholds,
            tracked: HashMap::new(),
            owners: HashMap::new(),
            remember,
            learned_dirty: false,
        }
    }

    /// Whether what is remembered has changed since this was last asked, clearing the flag.
    ///
    /// Exists so the caller writes the file when there is something to write and not once
    /// every five seconds forever: `CLAUDE.md` allows no disk I/O on a hot path, and a
    /// monitored game that has learned nothing new must cost nothing at all.
    pub fn take_learned_change(&mut self) -> bool {
        std::mem::take(&mut self.learned_dirty)
    }

    /// Starts following the pool of a monitored application.
    ///
    /// `preset` is the application's bundled preset identifier — the only identity that
    /// means anything across restarts. An application without one gets no pool: nothing was
    /// bundled for it and nothing was learned under a name we could look it up by.
    /// `learned` is what the stored file held for that preset.
    ///
    /// Returns the targets to start probing.
    pub fn track(
        &mut self,
        app: AppId,
        preset: &str,
        learned: &[(TargetAddress, SystemTime)],
        registry: &mut TargetRegistry,
        wall: SystemTime,
    ) -> Vec<TargetChange> {
        if self.tracked.contains_key(&app) {
            return Vec::new();
        }

        let mut pool = ReferencePool::new(self.policy, self.seeds.for_preset(preset));
        for (address, seen) in learned {
            pool.observe(*address, *seen);
        }
        pool.expire(wall);

        self.tracked.insert(
            app,
            Tracked {
                preset: preset.to_owned(),
                pool,
                probed: Vec::new(),
                dirty: true,
            },
        );
        self.reconcile(app, registry)
    }

    /// Stops following an application's pool and releases its targets.
    pub fn forget(&mut self, app: AppId, registry: &mut TargetRegistry) -> Vec<TargetChange> {
        let Some(tracked) = self.tracked.remove(&app) else {
            return Vec::new();
        };
        let mut changes = Vec::new();
        for (id, _) in tracked.probed {
            self.owners.remove(&id);
            if registry.untag(id, TargetTag::GameReferencePool) {
                changes.push(TargetChange::Unregister { id });
            }
        }
        changes
    }

    /// Records that an application's game used `address`.
    ///
    /// Does nothing when the user has asked for nothing to be remembered, and nothing for an
    /// application with no pool. The new entry joins the trickle on the next
    /// [`PoolMonitor::sweep`] rather than immediately, so a burst of discovery cannot become
    /// a burst of registrations.
    pub fn observe(&mut self, app: AppId, address: TargetAddress, wall: SystemTime) {
        if !self.remember {
            return;
        }
        let Some(tracked) = self.tracked.get_mut(&app) else {
            return;
        };
        let before = tracked.pool.len();
        tracked.pool.observe(address, wall);
        if tracked.pool.len() != before {
            tracked.dirty = true;
            self.learned_dirty = true;
        }
    }

    /// Brings every pool's registrations back in line with what it holds.
    ///
    /// Called on the discovery beat. Cheap when nothing changed: a pool that has learned
    /// nothing new produces no instructions at all.
    pub fn sweep(&mut self, registry: &mut TargetRegistry, wall: SystemTime) -> Vec<TargetChange> {
        let apps: Vec<AppId> = self.tracked.keys().copied().collect();
        let mut changes = Vec::new();
        for app in apps {
            if let Some(tracked) = self.tracked.get_mut(&app) {
                let before = tracked.pool.len();
                tracked.pool.expire(wall);
                if tracked.pool.len() != before {
                    tracked.dirty = true;
                    self.learned_dirty = true;
                }
                if !tracked.dirty {
                    continue;
                }
            }
            changes.extend(self.reconcile(app, registry));
        }
        changes
    }

    /// Records a probe result. A handle that belongs to nothing here is ignored.
    pub fn record(&mut self, id: TargetId, sample: ProbeSample) {
        let Some(app) = self.owners.get(&id).copied() else {
            return;
        };
        let Some(tracked) = self.tracked.get_mut(&app) else {
            return;
        };
        if let Some((_, history)) = tracked.probed.iter_mut().find(|(seen, _)| *seen == id) {
            history.record(sample);
        }
    }

    /// What one application's pool says, as of `now`.
    ///
    /// [`None`] for an application with no pool at all — a title whose operator publishes no
    /// reference address and whose servers this machine has never seen. That is a real state
    /// and the caller must say so, because an absent pool can neither report an outage nor
    /// rule one out, and an empty reading must never read as a clean one.
    #[must_use]
    pub fn reading(&self, app: AppId, now: Instant) -> Option<PoolReport> {
        let tracked = self.tracked.get(&app)?;
        if tracked.pool.is_empty() {
            return None;
        }

        let stats: Vec<WindowStats> = tracked
            .probed
            .iter()
            .map(|(_, history)| history.stats_for_window(now, POOL_WINDOW))
            .collect();
        let entries = tracked.pool.entries();
        Some(PoolReport {
            seeded: entries
                .iter()
                .filter(|entry| entry.source == PoolSource::Bundled)
                .count(),
            learned: entries.len()
                - entries
                    .iter()
                    .filter(|e| e.source == PoolSource::Bundled)
                    .count(),
            reading: PoolReading::of(stats.iter(), &self.thresholds),
        })
    }

    /// Folds what the tracked applications have learned into the store that will be written.
    ///
    /// Merged rather than replacing the whole file: what was learned about a game the user
    /// is *not* watching right now is still true, and writing only the tracked applications
    /// would erase yesterday's memory of every other title on the first save.
    pub fn merge_learned_into(&self, store: &mut LearnedPools) {
        for tracked in self.tracked.values() {
            store.set(&tracked.preset, tracked.pool.learned().collect());
        }
    }

    /// Registers whatever the pool holds that is not registered yet.
    fn reconcile(&mut self, app: AppId, registry: &mut TargetRegistry) -> Vec<TargetChange> {
        let Some(tracked) = self.tracked.get_mut(&app) else {
            return Vec::new();
        };
        tracked.dirty = false;

        let wanted = tracked.pool.entries();
        let mut changes = Vec::new();
        let mut kept: Vec<(TargetId, SampleHistory)> = Vec::new();
        let mut owners = Vec::new();

        for entry in &wanted {
            // A registry out of handles — which only a session of billions of targets could
            // cause — leaves the pool smaller than it wanted rather than absent, and its
            // reading says how many members it has.
            let Ok(id) = registry.insert(entry.address, TargetTag::GameReferencePool) else {
                continue;
            };

            // An address already probed for this pool keeps its history: a sweep is
            // bookkeeping, and re-registering must not amnesty a target that has been
            // silent for an hour.
            if let Some((_, history)) = tracked.probed.iter().find(|(seen, _)| *seen == id) {
                kept.push((id, history.clone()));
            } else {
                let Ok(history) = SampleHistory::new(HISTORY_CAPACITY) else {
                    continue;
                };
                kept.push((id, history));
                changes.push(TargetChange::Register {
                    id,
                    address: entry.address,
                    // Never source-bound. A pool asks whether the game's infrastructure
                    // answers at all, not what one application's route to it looks like —
                    // and binding it to a route that moves would make the pool's own
                    // history discontinuous.
                    source: None,
                    interval: POOL_INTERVAL,
                });
            }
            owners.push(id);
        }

        // Anything the pool no longer holds — an expired or evicted entry — stops being
        // probed, unless something else still wants the address.
        let gone: Vec<TargetId> = tracked
            .probed
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !owners.contains(id))
            .collect();
        for id in gone {
            self.owners.remove(&id);
            if registry.untag(id, TargetTag::GameReferencePool) {
                changes.push(TargetChange::Unregister { id });
            }
        }

        if let Some(tracked) = self.tracked.get_mut(&app) {
            tracked.probed = kept;
        }
        for id in owners {
            self.owners.insert(id, app);
        }
        changes
    }
}

/// One application's pool, as the rest of the app reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolReport {
    /// How many members came from the bundled seeds.
    pub seeded: usize,
    /// How many members this machine learned from the game's own traffic.
    pub learned: usize,
    /// What the members say together.
    pub reading: PoolReading,
}
