//! The state behind the general-health dashboard.
//!
//! [`BaselineMonitor`] holds one entry per baseline target: what it is, what the probe
//! engine has learned about how to measure it, and a bounded history of its samples. It
//! reads no clock, opens no socket and knows nothing about tokio — callers pass `now` in —
//! which is what lets a whole session of degradation be replayed in a test in
//! microseconds. `crate::runtime` is the part that actually probes.
//!
//! # Why unresolved and unmeasurable targets stay in the list
//!
//! A baseline that quietly shrinks is a baseline that lies. If a foreign target's name
//! stops resolving, or every probe kind is exhausted against it, dropping it would leave
//! the group looking healthier than it is — the remaining members would all be green and
//! the verdict would agree. So an entry that cannot be measured stays, contributes
//! [`nm_core::health::Health::Unknown`] to its group, and says so on screen.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nm_core::health::{GroupHealth, Health, HealthThresholds};
use nm_core::history::SampleHistory;
use nm_core::sample::{ProbeSample, Rtt};
use nm_core::stats::WindowStats;
use nm_core::target::{TargetAddress, TargetId};
use nm_probes::probe::ProbeKind;

use crate::baselines::{BaselineGroup, BaselineTarget};
use crate::events::NetworkHealth;
use crate::view::{GroupView, ProbeKindView, TargetView, SERIES_POINTS};
use crate::Error;

/// How many samples are retained per baseline target.
///
/// At four targets per group and two groups this is at most ~1920 samples of 4-byte
/// round-trip times plus their stamps — tens of kilobytes, fixed, whatever the uptime.
pub const HISTORY_CAPACITY: usize = 240;

/// How many probe intervals the health window spans.
const WINDOW_INTERVALS: u32 = 12;

/// Shortest health window, however fast the probes are.
const MIN_WINDOW: Duration = Duration::from_secs(60);

/// Longest health window, however slow the probes are.
const MAX_WINDOW: Duration = Duration::from_secs(600);

/// The span of history every figure on the dashboard is computed over.
///
/// Scaled to the probe interval rather than fixed: a window of a fixed sixty seconds holds
/// one sample when the user has set a sixty-second interval, which would leave every
/// verdict permanently "unknown". Twelve intervals is enough for a loss percentage to mean
/// something and short enough that an endpoint going down is reported while the user is
/// still looking at the screen.
#[must_use]
pub fn health_window(interval: Duration) -> Duration {
    interval
        .saturating_mul(WINDOW_INTERVALS)
        .clamp(MIN_WINDOW, MAX_WINDOW)
}

/// One baseline target and everything known about it.
#[derive(Debug, Clone)]
struct Entry {
    group: BaselineGroup,
    key: String,
    label: String,
    written_address: String,
    address: Option<TargetAddress>,
    tunnelled: bool,
    measurable: bool,
    probe_kind: Option<ProbeKind>,
    filtering_confirmed: bool,
    history: SampleHistory,
}

impl Entry {
    fn view(&self, now: Instant, stats: &WindowStats, thresholds: &HealthThresholds) -> TargetView {
        let mut series_age_secs = Vec::with_capacity(SERIES_POINTS);
        let mut series_rtt_ms = Vec::with_capacity(SERIES_POINTS);
        for sample in self.history.recent(SERIES_POINTS) {
            // Negative seconds, ascending to zero at the right-hand edge: the chart's x
            // axis is real time, so a stretched interval shows as a gap rather than as an
            // evenly spaced lie.
            series_age_secs.push(-now.saturating_duration_since(sample.at).as_secs_f64());
            series_rtt_ms.push(sample.outcome.rtt().map(Rtt::as_millis_f64));
        }

        TargetView {
            key: self.key.clone(),
            label: self.label.clone(),
            written_address: self.written_address.clone(),
            resolved_address: self.address.map(describe_address),
            tunnelled: self.tunnelled,
            measurable: self.measurable,
            probe_kind: self.probe_kind.map(ProbeKindView::from),
            filtering_confirmed: self.filtering_confirmed,
            health: thresholds.health_of(stats).into(),
            rtt_ms: stats.rtt.map(|rtt| rtt.mean_ms),
            jitter_ms: stats.rtt.and_then(|rtt| rtt.jitter_ms),
            loss_pct: stats.loss_pct,
            series_age_secs,
            series_rtt_ms,
        }
    }
}

/// Renders an address for display, with its port when it has one.
fn describe_address(address: TargetAddress) -> String {
    match address.port {
        Some(port) if address.ip.is_ipv6() => format!("[{}]:{port}", address.ip),
        Some(port) => format!("{}:{port}", address.ip),
        None => address.ip.to_string(),
    }
}

/// Every baseline target, its history, and the verdicts they add up to.
#[derive(Debug, Clone)]
pub struct BaselineMonitor {
    entries: Vec<Entry>,
    by_target: HashMap<TargetId, usize>,
    thresholds: HealthThresholds,
    window: Duration,
}

impl BaselineMonitor {
    /// Creates a monitor judging by `thresholds` over a window of `window`.
    #[must_use]
    pub fn new(thresholds: HealthThresholds, window: Duration) -> Self {
        Self {
            entries: Vec::new(),
            by_target: HashMap::new(),
            thresholds,
            window,
        }
    }

    /// Adds a baseline target.
    ///
    /// `id` is the probe engine's handle, absent when the entry's address never resolved.
    /// `tunnelled` marks an endpoint a local tunnel remaps, whose figure is end-to-end
    /// through that tunnel rather than a round trip to the server.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Probes`] if the sample history cannot be allocated, which only a
    /// zero capacity would cause.
    pub fn add(
        &mut self,
        target: &BaselineTarget,
        id: Option<TargetId>,
        tunnelled: bool,
    ) -> Result<(), Error> {
        let history = SampleHistory::new(HISTORY_CAPACITY).map_err(nm_probes::Error::Core)?;
        if let Some(id) = id {
            self.by_target.insert(id, self.entries.len());
        }
        self.entries.push(Entry {
            group: target.group,
            key: target.key.clone(),
            label: target.label.clone(),
            written_address: target.written_address.clone(),
            address: target.address,
            tunnelled,
            // An entry with no address was never measurable; one with an address is until
            // the probe engine says otherwise.
            measurable: id.is_some(),
            probe_kind: None,
            filtering_confirmed: false,
            history,
        });
        Ok(())
    }

    /// Records a probe result. A handle that belongs to nothing here is ignored.
    pub fn record(&mut self, id: TargetId, sample: ProbeSample) {
        if let Some(entry) = self.entry_mut(id) {
            entry.history.record(sample);
        }
    }

    /// Notes what the probe engine is currently doing with a target.
    ///
    /// `kind` is [`None`] once every kind has been ruled out; `measurable` says whether
    /// anything honest is left to try at all.
    pub fn note_probe_state(
        &mut self,
        id: TargetId,
        kind: Option<ProbeKind>,
        filtering_confirmed: bool,
        measurable: bool,
    ) {
        if let Some(entry) = self.entry_mut(id) {
            entry.probe_kind = kind;
            entry.filtering_confirmed = filtering_confirmed;
            entry.measurable = measurable;
        }
    }

    /// Marks a target as one no probe kind can honestly measure.
    pub fn note_unmeasurable(&mut self, id: TargetId) {
        if let Some(entry) = self.entry_mut(id) {
            entry.measurable = false;
            entry.probe_kind = None;
        }
    }

    /// How many baseline targets are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The whole picture, as of `now`.
    ///
    /// Allocates: this runs at the event rate — once a second — not once per sample, so
    /// the vectors it builds are off every hot path.
    #[must_use]
    pub fn snapshot(&self, now: Instant, uptime_secs: u32) -> NetworkHealth {
        let groups: Vec<GroupView> = BaselineGroup::ALL
            .iter()
            .map(|group| self.group_view(*group, now))
            .collect();

        let (domestic, foreign) = self.verdicts(now);
        NetworkHealth {
            uptime_secs,
            window_secs: nm_core::time::elapsed_secs(self.window),
            diagnosis: nm_core::diagnosis::diagnose(&nm_core::diagnosis::Evidence::baselines(
                domestic, foreign,
            ))
            .into(),
            groups,
        }
    }

    /// Each baseline's headline verdict, as of `now`.
    ///
    /// Exposed because the applications need it too: an application's endpoints failing
    /// while the whole network is failing says nothing about that application, and the
    /// diagnosis engine cannot apply that rule without both baselines in front of it.
    #[must_use]
    pub fn verdicts(&self, now: Instant) -> (Health, Health) {
        (
            self.group_health(BaselineGroup::Domestic, now).verdict,
            self.group_health(BaselineGroup::Foreign, now).verdict,
        )
    }

    fn group_health(&self, group: BaselineGroup, now: Instant) -> GroupHealth {
        let stats: Vec<WindowStats> = self
            .entries
            .iter()
            .filter(|entry| entry.group == group)
            .map(|entry| entry.history.stats_for_window(now, self.window))
            .collect();
        GroupHealth::of(stats.iter(), &self.thresholds)
    }

    fn group_view(&self, group: BaselineGroup, now: Instant) -> GroupView {
        let members: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.group == group)
            .collect();

        let stats: Vec<WindowStats> = members
            .iter()
            .map(|entry| entry.history.stats_for_window(now, self.window))
            .collect();

        let health = GroupHealth::of(stats.iter(), &self.thresholds);
        let targets = members
            .iter()
            .zip(stats.iter())
            .map(|(entry, stats)| entry.view(now, stats, &self.thresholds))
            .collect();

        GroupView::new(group, health, targets)
    }

    fn entry_mut(&mut self, id: TargetId) -> Option<&mut Entry> {
        let index = *self.by_target.get(&id)?;
        self.entries.get_mut(index)
    }
}
