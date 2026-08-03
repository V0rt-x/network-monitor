//! The state behind the Network page.
//!
//! One monitor for every target the page draws — the domestic baseline, the foreign one, the
//! gaming platforms and the infrastructure. It holds one entry per *endpoint* and one row per
//! named thing, with a bounded history of samples each. It reads no clock, opens no socket
//! and knows nothing about tokio — callers pass `now` in — which is what lets a whole day of
//! a service going down and coming back replay in a test in microseconds. `crate::runtime` is
//! the part that actually probes.
//!
//! # What merged and what did not
//!
//! There used to be two monitors of nearly identical shape, `BaselineMonitor` and
//! `ServiceMonitor`, feeding two sets of view types and two sets of components, over two
//! target schemas — one of which held two entries that were literally copies of the other's.
//! The inventory, the view and the page are now one.
//!
//! **The measurement layer is not.** A baseline asks what the last several minutes have been
//! like, which is a window ([`nm_core::health`]); a platform asks whether it is reachable
//! *now*, which at a check every forty-odd seconds a window answers badly at both ends — a
//! service that died a minute ago still reads mostly green, and one that has just recovered
//! still reads mostly red ([`nm_core::status`]). One rule stretched across both would be
//! exactly the smoothing this product forbids. Which rule applies is a property of the
//! section, stated once in [`Section::judged_by_window`].
//!
//! # Why unresolved and unmeasurable targets stay in the list
//!
//! A list that quietly shrinks is a list that lies. If a foreign target's name stops
//! resolving, or every probe kind is exhausted against it, dropping it would leave the
//! section looking healthier than it is — the remaining members would all be green and the
//! verdict would agree. So an entry that cannot be measured stays, contributes
//! [`nm_core::health::Health::Unknown`] to its section, and says so on screen.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nm_core::diagnosis::{BaselineEvidence, Evidence};
use nm_core::health::{GroupHealth, HealthThresholds};
use nm_core::history::SampleHistory;
use nm_core::sample::{ProbeSample, Rtt};
use nm_core::stats::WindowStats;
use nm_core::status::StatusThresholds;
use nm_core::target::{TargetAddress, TargetId};
use nm_probes::probe::ProbeKind;

use crate::events::NetworkSnapshot;
use crate::targets::{ResolvedTarget, Section, SLOW_CHECK_INTERVAL};
use crate::view::{CheckView, NetworkRowView, NetworkSectionView, RowEndpointView};
use crate::Error;

/// How many probe intervals the health window spans.
const WINDOW_INTERVALS: u32 = 12;

/// Shortest health window, however fast the probes are.
const MIN_WINDOW: Duration = Duration::from_secs(60);

/// Longest health window, however slow the probes are.
const MAX_WINDOW: Duration = Duration::from_secs(600);

/// How many samples are retained per endpoint.
///
/// Fixed whatever the uptime: a few tens of kilobytes across the whole inventory.
pub const HISTORY_CAPACITY: usize = 240;

/// How many checks the strip draws.
///
/// Fewer than are retained: the statistics beside a row read the whole window, while the
/// strip shows the recent stretch a reader can actually take in at that width.
pub const TIMELINE_POINTS: usize = 24;

/// The span every windowed figure on the page is computed over.
///
/// Scaled to the probe interval rather than fixed: a window of a fixed sixty seconds holds
/// one sample when the user has set a sixty-second interval, which would leave every verdict
/// permanently "unknown". Twelve intervals is enough for a loss percentage to mean something
/// and short enough that a target going down is reported while the user is still looking at
/// the screen.
#[must_use]
pub fn health_window(interval: Duration) -> Duration {
    interval
        .saturating_mul(WINDOW_INTERVALS)
        .clamp(MIN_WINDOW, MAX_WINDOW)
}

/// The span a slowly checked section's figures cover.
///
/// Long enough to hold [`TIMELINE_POINTS`] checks, so the numbers and the strip beside them
/// describe the same stretch of time rather than quietly disagreeing.
#[must_use]
pub fn slow_window() -> Duration {
    // Bounded by two constants; the saturation is a formality that keeps this total.
    SLOW_CHECK_INTERVAL.saturating_mul(u32::try_from(TIMELINE_POINTS).unwrap_or(u32::MAX))
}

/// One endpoint of one target, and everything known about it.
#[derive(Debug, Clone)]
struct Entry {
    row: usize,
    key: String,
    written_address: String,
    address: Option<TargetAddress>,
    tunnelled: bool,
    measurable: bool,
    probe_kind: Option<ProbeKind>,
    filtering_confirmed: bool,
    history: SampleHistory,
}

/// One named thing on the page, as the list described it.
#[derive(Debug, Clone)]
struct Row {
    key: String,
    label: String,
    section: Section,
}

/// Every target the Network page draws, its history, and the verdicts they add up to.
#[derive(Debug, Clone)]
pub struct NetworkMonitor {
    rows: Vec<Row>,
    entries: Vec<Entry>,
    by_target: HashMap<TargetId, usize>,
    window: HealthThresholds,
    reaction: StatusThresholds,
    /// The span the windowed sections are judged over — the user's interval, scaled.
    baseline_window: Duration,
}

impl NetworkMonitor {
    /// Creates a monitor judging windowed sections over `baseline_window`.
    #[must_use]
    pub fn new(
        window: HealthThresholds,
        reaction: StatusThresholds,
        baseline_window: Duration,
    ) -> Self {
        Self {
            rows: Vec::new(),
            entries: Vec::new(),
            by_target: HashMap::new(),
            window,
            reaction,
            baseline_window,
        }
    }

    /// Adds a target and its endpoints.
    ///
    /// `handles` gives the probe engine's handle for each endpoint in order, absent where the
    /// name never resolved or no probe kind can honestly measure it. Such an endpoint stays
    /// on the page, unmeasured and saying so.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Probes`] if a history cannot be allocated, which only a zero capacity
    /// would cause.
    pub fn add(
        &mut self,
        target: &ResolvedTarget,
        handles: &[Option<TargetId>],
    ) -> Result<(), Error> {
        let index = self.rows.len();
        self.rows.push(Row {
            key: target.key.clone(),
            label: target.label.clone(),
            section: target.section,
        });

        for (position, endpoint) in target.endpoints.iter().enumerate() {
            let history = SampleHistory::new(HISTORY_CAPACITY).map_err(nm_probes::Error::Core)?;
            let handle = handles.get(position).copied().flatten();
            if let Some(id) = handle {
                self.by_target.insert(id, self.entries.len());
            }
            self.entries.push(Entry {
                row: index,
                key: endpoint.key.clone(),
                written_address: endpoint.written_address.clone(),
                address: endpoint.address,
                tunnelled: false,
                measurable: handle.is_some(),
                probe_kind: None,
                filtering_confirmed: false,
                history,
            });
        }
        Ok(())
    }

    /// Marks an endpoint as one a local tunnel remaps.
    ///
    /// Its figure is then end-to-end through that tunnel rather than a round trip to the
    /// target, and the row says so.
    pub fn note_tunnelled(&mut self, id: TargetId) {
        if let Some(entry) = self.entry_mut(id) {
            entry.tunnelled = true;
        }
    }

    /// Records a probe result. A handle that belongs to nothing here is ignored.
    pub fn record(&mut self, id: TargetId, sample: ProbeSample) {
        if let Some(entry) = self.entry_mut(id) {
            entry.history.record(sample);
        }
    }

    /// Notes what the probe engine is currently doing with an endpoint.
    pub fn note_probe_state(
        &mut self,
        id: TargetId,
        kind: Option<ProbeKind>,
        tunnelled: bool,
        filtering_confirmed: bool,
        measurable: bool,
    ) {
        if let Some(entry) = self.entry_mut(id) {
            entry.probe_kind = kind;
            // Refreshed on every report rather than fixed at registration: a tunnel can be
            // proven from a reply, and a user can switch a VPN on mid-session.
            entry.tunnelled = tunnelled;
            entry.filtering_confirmed = filtering_confirmed;
            entry.measurable = measurable;
        }
    }

    /// Marks an endpoint as one no probe kind can honestly measure.
    pub fn note_unmeasurable(&mut self, id: TargetId) {
        if let Some(entry) = self.entry_mut(id) {
            entry.measurable = false;
            entry.probe_kind = None;
        }
    }

    /// How many targets are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The whole page, as of `now`.
    ///
    /// Allocates: this runs at the emission rate — once a second — not once per sample, so
    /// the vectors it builds are off every hot path.
    #[must_use]
    pub fn snapshot(&self, now: Instant, uptime_secs: u32) -> NetworkSnapshot {
        let sections: Vec<NetworkSectionView> = Section::ALL
            .iter()
            .map(|section| self.section_view(*section, now))
            .collect();

        let (domestic, foreign) = self.evidence(now);
        NetworkSnapshot {
            uptime_secs,
            timeline_points: u32::try_from(TIMELINE_POINTS).unwrap_or(u32::MAX),
            diagnosis: nm_core::diagnosis::diagnose(&Evidence {
                domestic,
                foreign,
                app: None,
            })
            .into(),
            sections,
        }
    }

    /// What each verdict-bearing section contributes to a diagnosis, as of `now`.
    ///
    /// Exposed because the applications need it too: an application's endpoints failing
    /// while the whole network is failing says nothing about that application, and the
    /// diagnosis engine cannot apply that rule without both in front of it.
    ///
    /// The **distribution and the loss figure**, not the headline verdict. `GroupHealth`
    /// calls anything less than a clean sweep `Degraded`, which is right on a heading that
    /// shows the counts beside it and wrong as an input to a conclusion.
    #[must_use]
    pub fn evidence(&self, now: Instant) -> (BaselineEvidence, BaselineEvidence) {
        let of = |section| {
            let health = self.section_health(section, now);
            BaselineEvidence::of(health.counts).losing(health.loss_pct)
        };
        (of(Section::Domestic), of(Section::Foreign))
    }

    /// The window a section's figures are computed over.
    fn window_for(&self, section: Section) -> Duration {
        if section.judged_by_window() {
            self.baseline_window
        } else {
            slow_window()
        }
    }

    /// How one row is judged — by the window, or by the reaction rule.
    ///
    /// The one place the two measurement rules meet, and they meet by choosing rather than
    /// by averaging: a figure computed across both would be the smoothing this product
    /// exists not to perform.
    fn row_health(
        &self,
        section: Section,
        checks: &[Vec<ProbeSample>],
        stats: &[WindowStats],
    ) -> GroupHealth {
        if section.judged_by_window() {
            GroupHealth::of(stats.iter(), &self.window)
        } else {
            self.reaction
                .service_health(checks.iter().map(Vec::as_slice).zip(stats.iter()))
        }
    }

    fn section_health(&self, section: Section, now: Instant) -> GroupHealth {
        let window = self.window_for(section);
        let members: Vec<&Entry> = self.members_of_section(section).collect();
        let checks: Vec<Vec<ProbeSample>> = members
            .iter()
            .map(|entry| entry.history.recent(HISTORY_CAPACITY).copied().collect())
            .collect();
        let stats: Vec<WindowStats> = members
            .iter()
            .map(|entry| entry.history.stats_for_window(now, window))
            .collect();
        self.row_health(section, &checks, &stats)
    }

    fn members_of_section(&self, section: Section) -> impl Iterator<Item = &Entry> + '_ {
        self.entries.iter().filter(move |entry| {
            self.rows
                .get(entry.row)
                .is_some_and(|row| row.section == section)
        })
    }

    fn section_view(&self, section: Section, now: Instant) -> NetworkSectionView {
        let rows: Vec<NetworkRowView> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.section == section)
            .map(|(index, row)| self.row_view(index, row, now))
            .collect();

        let health = self.section_health(section, now);
        NetworkSectionView {
            section,
            read_by_verdict: section.read_by_verdict(),
            verdict: health.verdict.into(),
            counts: health.counts.into(),
            rtt_ms: health.rtt_ms,
            cadence_secs: nm_core::time::elapsed_secs(if section.read_by_verdict() {
                self.baseline_window / WINDOW_INTERVALS
            } else {
                SLOW_CHECK_INTERVAL
            }),
            window_secs: nm_core::time::elapsed_secs(self.window_for(section)),
            rows,
        }
    }

    fn row_view(&self, index: usize, row: &Row, now: Instant) -> NetworkRowView {
        let window = self.window_for(row.section);
        let members: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.row == index)
            .collect();

        let checks: Vec<Vec<ProbeSample>> = members
            .iter()
            .map(|entry| entry.history.recent(HISTORY_CAPACITY).copied().collect())
            .collect();
        let stats: Vec<WindowStats> = members
            .iter()
            .map(|entry| entry.history.stats_for_window(now, window))
            .collect();

        let health = self.row_health(row.section, &checks, &stats);

        // The newest check anywhere in the row. What "last checked" means for a row with two
        // endpoints is the more recent of them: the older one is visible on its own strip,
        // and a row claiming to be staler than its freshest evidence would be as wrong as one
        // claiming to be fresher.
        let last_checked_secs = members
            .iter()
            .filter_map(|entry| entry.history.latest())
            .map(|sample| now.saturating_duration_since(sample.at).as_secs_f64())
            .min_by(f64::total_cmp);

        let endpoints = members
            .iter()
            .zip(checks.iter())
            .zip(stats.iter())
            .map(|((entry, checks), stats)| {
                self.endpoint_view(entry, row.section, checks, stats, now)
            })
            .collect();

        NetworkRowView {
            key: row.key.clone(),
            label: row.label.clone(),
            health: health.verdict.into(),
            counts: health.counts.into(),
            rtt_ms: health.rtt_ms,
            last_checked_secs,
            endpoints,
        }
    }

    fn endpoint_view(
        &self,
        entry: &Entry,
        section: Section,
        checks: &[ProbeSample],
        stats: &WindowStats,
        now: Instant,
    ) -> RowEndpointView {
        let timeline = checks[checks.len().saturating_sub(TIMELINE_POINTS)..]
            .iter()
            .map(|sample| CheckView {
                age_secs: -now.saturating_duration_since(sample.at).as_secs_f64(),
                mark: self.reaction.mark(sample.outcome).into(),
            })
            .collect();

        // The same choice the row made, one level down: a windowed section's endpoint is
        // judged by its window, a slowly checked one by how its recent checks went.
        let health = if section.judged_by_window() {
            self.window.health_of(stats)
        } else {
            self.reaction.health_of(checks)
        };

        RowEndpointView {
            key: entry.key.clone(),
            written_address: entry.written_address.clone(),
            resolved_address: entry.address.map(describe_address),
            tunnelled: entry.tunnelled,
            measurable: entry.measurable,
            probe_kind: entry.probe_kind.map(Into::into),
            filtering_confirmed: entry.filtering_confirmed,
            health: health.into(),
            // The latest answer rather than the window's mean: a row says how the target is
            // responding now, and the strip beside it is what shows the spread. The mean is
            // the figure next to it, and both say which they are.
            rtt_ms: checks
                .iter()
                .rev()
                .find_map(|sample| sample.outcome.rtt())
                .map(Rtt::as_millis_f64),
            mean_rtt_ms: stats.rtt.map(|rtt| rtt.mean_ms),
            jitter_ms: stats.rtt.and_then(|rtt| rtt.jitter_ms),
            loss_pct: stats.loss_pct,
            checks: timeline,
        }
    }

    fn entry_mut(&mut self, id: TargetId) -> Option<&mut Entry> {
        let index = *self.by_target.get(&id)?;
        self.entries.get_mut(index)
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
