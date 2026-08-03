//! The state behind the service status page.
//!
//! [`ServiceMonitor`] holds one entry per service endpoint: what it is, what the probe
//! engine has learned about how to measure it, and a bounded history of its checks. Like
//! [`crate::monitor::BaselineMonitor`] it reads no clock, opens no socket and knows nothing
//! about tokio — callers pass `now` in — so a whole day of a service going down and coming
//! back replays in a test in microseconds. `crate::runtime` is the part that actually
//! probes.
//!
//! # Why the verdict is not the dashboard's
//!
//! The dashboard judges a window: what has this baseline been like for the last several
//! minutes. A status card asks whether a service is reachable **now**, and at a check every
//! forty-odd seconds a window rule answers that badly at both ends — a service that died a
//! minute ago still reads mostly green, and one that has just recovered still reads mostly
//! red. [`nm_core::status`] holds the rule that reads the most recent checks instead, and
//! the reasoning for every line it draws.
//!
//! The window statistics are still computed and still shown: they are where the latency
//! figure beside the card comes from. What they no longer decide is the colour.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nm_core::history::SampleHistory;
use nm_core::sample::{ProbeSample, Rtt};
use nm_core::stats::WindowStats;
use nm_core::status::StatusThresholds;
use nm_core::target::{TargetAddress, TargetId};
use nm_probes::probe::ProbeKind;

use crate::events::ServiceStatus;
use crate::services::{ResolvedService, ServiceGroup};
use crate::view::{CheckView, ServiceEndpointView, ServiceView};
use crate::Error;

/// How often each service endpoint is checked.
///
/// Deliberately far slower than anything else the engine probes. A status page is a
/// background question — "is the platform up" changes on the scale of minutes, not of a
/// packet — and this list is the one part of the product that is probed whether or not the
/// user is doing anything, so it must cost close to nothing: the bundled list's endpoints at
/// this cadence come to about half a probe a second against the product's cap of thirty-two.
/// A test asserts the whole list stays under one, which is what bounds how many services can
/// be added at all.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(45);

/// How many checks are retained per endpoint.
///
/// At [`CHECK_INTERVAL`] this is three quarters of an hour of history in about a kilobyte
/// per endpoint — fixed, whatever the uptime.
pub const HISTORY_CAPACITY: usize = 60;

/// How many checks the mini-timeline draws.
///
/// Fewer than are retained: the statistics beside the card read the whole history, while the
/// strip shows the recent stretch a user can actually take in at that width.
pub const TIMELINE_POINTS: usize = 24;

/// The span the latency figure beside a card is computed over.
///
/// Long enough to hold [`TIMELINE_POINTS`] checks, so the number and the strip describe the
/// same stretch of time rather than quietly disagreeing.
#[must_use]
pub fn stats_window() -> Duration {
    // Bounded by two constants; the saturation is a formality that keeps this total.
    CHECK_INTERVAL.saturating_mul(u32::try_from(TIMELINE_POINTS).unwrap_or(u32::MAX))
}

/// One service endpoint and everything known about it.
#[derive(Debug, Clone)]
struct Entry {
    service: usize,
    key: String,
    written_address: String,
    address: Option<TargetAddress>,
    tunnelled: bool,
    measurable: bool,
    probe_kind: Option<ProbeKind>,
    filtering_confirmed: bool,
    history: SampleHistory,
}

/// One service, as the list described it.
#[derive(Debug, Clone)]
struct Service {
    id: String,
    label: String,
    group: ServiceGroup,
}

/// Every service endpoint, its history, and the verdicts they add up to.
#[derive(Debug, Clone)]
pub struct ServiceMonitor {
    services: Vec<Service>,
    entries: Vec<Entry>,
    by_target: HashMap<TargetId, usize>,
    thresholds: StatusThresholds,
}

impl ServiceMonitor {
    /// Creates a monitor judging by `thresholds`.
    #[must_use]
    pub fn new(thresholds: StatusThresholds) -> Self {
        Self {
            services: Vec::new(),
            entries: Vec::new(),
            by_target: HashMap::new(),
            thresholds,
        }
    }

    /// Adds a service and its endpoints.
    ///
    /// `handles` gives the probe engine's handle for each endpoint in order, absent where
    /// the endpoint's name never resolved or no probe kind can honestly measure it. Such an
    /// endpoint stays on the page, unmeasured and saying so: a status page that quietly
    /// shrank to its working members would read as good news.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Probes`] if a check history cannot be allocated, which only a zero
    /// capacity would cause.
    pub fn add(
        &mut self,
        service: &ResolvedService,
        handles: &[Option<TargetId>],
    ) -> Result<(), Error> {
        let index = self.services.len();
        self.services.push(Service {
            id: service.id.clone(),
            label: service.label.clone(),
            group: service.group,
        });

        for (position, endpoint) in service.endpoints.iter().enumerate() {
            let history = SampleHistory::new(HISTORY_CAPACITY).map_err(nm_probes::Error::Core)?;
            let handle = handles.get(position).copied().flatten();
            if let Some(id) = handle {
                self.by_target.insert(id, self.entries.len());
            }
            self.entries.push(Entry {
                service: index,
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
    /// service, and the card says so — the same disclosure a tunnelled baseline gets.
    pub fn note_tunnelled(&mut self, id: TargetId) {
        if let Some(entry) = self.entry_mut(id) {
            entry.tunnelled = true;
        }
    }

    /// Records a check result. A handle that belongs to nothing here is ignored.
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
            // Refreshed on every report rather than fixed when the target was registered: a
            // tunnel can be proven from a reply, and a user can switch a VPN on mid-session.
            entry.tunnelled = tunnelled;
            entry.filtering_confirmed = filtering_confirmed;
            entry.measurable = measurable;
        }
    }

    /// Marks an endpoint as one no probe kind can honestly check.
    pub fn note_unmeasurable(&mut self, id: TargetId) {
        if let Some(entry) = self.entry_mut(id) {
            entry.measurable = false;
            entry.probe_kind = None;
        }
    }

    /// How many services are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.services.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    /// The whole page, as of `now`.
    ///
    /// Allocates: this runs at the emission rate — once a second — not once per check, so
    /// the vectors it builds are off every hot path.
    #[must_use]
    pub fn snapshot(&self, now: Instant) -> ServiceStatus {
        let services = self
            .services
            .iter()
            .enumerate()
            .map(|(index, service)| self.service_view(index, service, now))
            .collect();

        ServiceStatus {
            check_interval_secs: nm_core::time::elapsed_secs(CHECK_INTERVAL),
            window_secs: nm_core::time::elapsed_secs(stats_window()),
            timeline_points: u32::try_from(TIMELINE_POINTS).unwrap_or(u32::MAX),
            services,
        }
    }

    fn service_view(&self, index: usize, service: &Service, now: Instant) -> ServiceView {
        let members: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.service == index)
            .collect();

        let window = stats_window();
        let checks: Vec<Vec<ProbeSample>> = members
            .iter()
            .map(|entry| entry.history.recent(HISTORY_CAPACITY).copied().collect())
            .collect();
        let stats: Vec<WindowStats> = members
            .iter()
            .map(|entry| entry.history.stats_for_window(now, window))
            .collect();

        let health = self
            .thresholds
            .service_health(checks.iter().map(Vec::as_slice).zip(stats.iter()));

        // The newest check anywhere in the service. What "last checked" means for a card
        // with two endpoints is the more recent of them: the older one is visible on its own
        // strip, and a card claiming to be a minute staler than its freshest evidence would
        // be as wrong as one claiming to be fresher.
        let last_checked_secs = members
            .iter()
            .filter_map(|entry| entry.history.latest())
            .map(|sample| now.saturating_duration_since(sample.at).as_secs_f64())
            .min_by(f64::total_cmp);

        let endpoints = members
            .iter()
            .zip(checks.iter())
            .zip(stats.iter())
            .map(|((entry, checks), stats)| self.endpoint_view(entry, checks, stats, now))
            .collect();

        ServiceView::new(
            service.id.clone(),
            service.label.clone(),
            service.group,
            health,
            last_checked_secs,
            endpoints,
        )
    }

    fn endpoint_view(
        &self,
        entry: &Entry,
        checks: &[ProbeSample],
        stats: &WindowStats,
        now: Instant,
    ) -> ServiceEndpointView {
        let timeline = checks[checks.len().saturating_sub(TIMELINE_POINTS)..]
            .iter()
            .map(|sample| CheckView {
                age_secs: -now.saturating_duration_since(sample.at).as_secs_f64(),
                mark: self.thresholds.mark(sample.outcome).into(),
            })
            .collect();

        ServiceEndpointView {
            key: entry.key.clone(),
            written_address: entry.written_address.clone(),
            resolved_address: entry.address.map(describe_address),
            tunnelled: entry.tunnelled,
            measurable: entry.measurable,
            probe_kind: entry.probe_kind.map(Into::into),
            filtering_confirmed: entry.filtering_confirmed,
            health: self.thresholds.health_of(checks).into(),
            // The latest answer rather than the window's mean: a card says how the service
            // is responding now, and the strip beside it is what shows the spread.
            rtt_ms: checks
                .iter()
                .rev()
                .find_map(|sample| sample.outcome.rtt())
                .map(Rtt::as_millis_f64),
            mean_rtt_ms: stats.rtt.map(|rtt| rtt.mean_ms),
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
