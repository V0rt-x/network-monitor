//! Deciding what to probe, when, and folding the answers back in.
//!
//! The runner is split in two on purpose. [`ProbeRunner`] makes every *decision* — which
//! targets are due, which probe kind each one is on, how far its interval has stretched —
//! and reads no clock, opens no socket and spawns nothing; callers pass `now` in. [`drive`]
//! is the thin async loop that actually carries the decisions out. That split is what lets a
//! full day of scheduling, degradation and recovery be tested in a millisecond, on any
//! operating system, with no network.
//!
//! # A probe in flight is not a probe due again
//!
//! A TCP probe can legitimately take seconds (Windows sits on a refusal for about two), far
//! longer than the one-second interval it is scheduled at. So a dispatched target is
//! *unscheduled* and only put back when its answer arrives, one interval later. An expensive
//! probe therefore spaces itself out instead of queueing behind itself, and the global rate
//! cap is spent on probes that are actually happening.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use nm_core::address::{AddressClass, AddressPolicy};
use nm_core::backoff::Backoff;
use nm_core::path::PathTrace;
use nm_core::sample::ProbeOutcome;
use nm_core::scheduler::ProbeScheduler;
use nm_core::target::{TargetAddress, TargetId};

use crate::chain::{ChainStep, FallbackChain};
use crate::path::PathProbe;
use crate::probe::{ProbeKind, ProbeTarget, Prober};
use crate::{Error, GLOBAL_PROBE_RATE_CAP_PER_SEC};

/// Default interval between probes of one target.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);

/// How far backoff may stretch a target's interval.
///
/// Half a minute: long enough that a permanently dead endpoint stops costing budget, short
/// enough that its recovery is noticed while the user is still looking at the screen.
pub const MAX_INTERVAL: Duration = Duration::from_secs(30);

/// How long to wait for a probe of a given kind before calling it silence.
///
/// Not one number, because the kinds fail on different timescales. An echo that has not come
/// back in a second is not coming back, and waiting longer only delays reporting the loss.
/// A TCP handshake needs far more room: **Windows takes about two seconds to report a
/// refused connection**, verified on loopback where the reset is instant, because the stack
/// retries before believing it. A deadline under that would turn every closed port into
/// fabricated packet loss — the exact failure this crate exists to avoid — so the connecting
/// kinds get six seconds, and a slow TCP probe is normal rather than stuck.
#[must_use]
pub const fn timeout_for(kind: ProbeKind) -> Duration {
    match kind {
        ProbeKind::IcmpEcho => Duration::from_secs(1),
        ProbeKind::TcpConnect | ProbeKind::TlsHello => Duration::from_secs(6),
    }
}

/// One probe the runner wants carried out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedProbe {
    /// Which target it belongs to.
    pub id: TargetId,
    /// What to do.
    pub action: PlannedAction,
    /// Where to send it, with the deadline for this kind already applied.
    pub target: ProbeTarget,
}

/// What the runner wants done for a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlannedAction {
    /// Measure it with this probe kind.
    Probe(ProbeKind),
    /// Every kind is ruled out; walk the path to learn where it stops.
    WalkThePath,
}

/// What a carried-out probe produced.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Measured {
    /// A probe of this kind returned this outcome.
    Probe {
        /// The kind used.
        kind: ProbeKind,
        /// What it measured.
        outcome: ProbeOutcome,
    },
    /// A path walk mapped the route.
    Path(Box<PathTrace>),
    /// The probe could not be carried out at all.
    ///
    /// Kept distinct from every outcome above and passed on rather than swallowed: our own
    /// failure must never reach the user wearing the network's clothes.
    Failed(Error),
}

/// One completed piece of work, ready to fold back in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Which target it belongs to.
    pub id: TargetId,
    /// When it completed, on the monotonic clock.
    pub at: Instant,
    /// What it produced.
    pub measured: Measured,
}

/// What the runner currently believes about how to measure one target.
///
/// Reported alongside every result because the belief is half the answer: the same
/// round-trip time means something different depending on which probe kind produced it,
/// and "ICMP is filtered here" is a fact the UI must be able to state — but only once
/// [`TargetProgress::filtering_confirmed`] says it has been proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetProgress {
    /// The probe kind now in use, or [`None`] once every kind has been ruled out.
    pub kind: Option<ProbeKind>,
    /// Whether a probe kind has been *proven* filtered on this path.
    pub filtering_confirmed: bool,
    /// Whether anything honest is left to try — a probe kind, or a path walk.
    pub measurable: bool,
}

/// A completed piece of work and the runner's state for its target afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completed {
    /// What was measured.
    pub report: Report,
    /// What the runner believes about the target now that the result is folded in.
    pub progress: TargetProgress,
}

/// Everything the runner tracks for one target.
#[derive(Debug, Clone)]
struct TargetState {
    address: TargetAddress,
    source: Option<IpAddr>,
    chain: FallbackChain,
    backoff: Backoff,
    in_flight: bool,
}

/// Decides what to probe and when, and folds the answers back in.
#[derive(Debug)]
pub struct ProbeRunner {
    scheduler: ProbeScheduler,
    targets: BTreeMap<TargetId, TargetState>,
    unmeasurable: Vec<TargetId>,
    policy: AddressPolicy,
    available: Vec<ProbeKind>,
    interval: Duration,
    max_interval: Duration,
    due_buffer: Vec<TargetId>,
}

impl ProbeRunner {
    /// Creates a runner that will use `available` probe kinds, within the global rate cap.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Core`] if the rate cap is unusable.
    pub fn new(
        policy: AddressPolicy,
        available: Vec<ProbeKind>,
        now: Instant,
    ) -> Result<Self, Error> {
        Ok(Self {
            scheduler: ProbeScheduler::new(GLOBAL_PROBE_RATE_CAP_PER_SEC, now)?,
            targets: BTreeMap::new(),
            unmeasurable: Vec::new(),
            policy,
            available,
            interval: DEFAULT_INTERVAL,
            max_interval: MAX_INTERVAL,
            due_buffer: Vec::new(),
        })
    }

    /// Uses a different base and maximum interval.
    #[must_use]
    pub const fn with_intervals(mut self, interval: Duration, max_interval: Duration) -> Self {
        self.interval = interval;
        self.max_interval = max_interval;
        self
    }

    /// Starts measuring `address` under the handle `id`.
    ///
    /// `source` pins the probe to the local address the monitored flow egresses from, so it
    /// follows the same interface, tunnel or accelerator.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NothingUsable`] when no available probe kind can honestly measure an
    /// address of this class — a refusal at the point of registration rather than a target
    /// that would sit in the schedule producing nothing.
    pub fn add(
        &mut self,
        id: TargetId,
        address: TargetAddress,
        source: Option<IpAddr>,
        now: Instant,
    ) -> Result<(), Error> {
        let class = self.policy.classify(address.ip);
        let chain = FallbackChain::new(class, &self.available)?;
        let backoff = Backoff::new(self.interval, self.max_interval)?;

        self.targets.insert(
            id,
            TargetState {
                address,
                source,
                chain,
                backoff,
                in_flight: false,
            },
        );
        self.unmeasurable.retain(|seen| *seen != id);
        self.scheduler.schedule(id, self.interval, now)?;
        Ok(())
    }

    /// Stops measuring a target. Returns `true` if it was registered.
    ///
    /// A probe already in flight is not cancelled — nothing can un-send a packet — but its
    /// report will be discarded, because the target is gone by the time it arrives.
    pub fn remove(&mut self, id: TargetId) -> bool {
        self.scheduler.unschedule(id);
        self.unmeasurable.retain(|seen| *seen != id);
        self.targets.remove(&id).is_some()
    }

    /// How many targets are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.targets.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Targets that no probe kind can honestly measure any more.
    ///
    /// They are no longer scheduled, so they cost nothing, and they are listed rather than
    /// dropped so the UI can say "this endpoint cannot be measured" instead of showing an
    /// endpoint that quietly stopped updating. [`ProbeRunner::reconsider`] brings one back.
    #[must_use]
    pub fn unmeasurable(&self) -> &[TargetId] {
        &self.unmeasurable
    }

    /// Which probe kind a target is currently using, if it is still measurable.
    #[must_use]
    pub fn current_kind(&self, id: TargetId) -> Option<ProbeKind> {
        self.targets.get(&id)?.chain.current_kind()
    }

    /// The fallback state of a target, for the UI to report honestly.
    #[must_use]
    pub fn chain(&self, id: TargetId) -> Option<&FallbackChain> {
        self.targets.get(&id).map(|state| &state.chain)
    }

    /// What the runner believes about how to measure a target.
    ///
    /// [`None`] for a target that is not registered.
    #[must_use]
    pub fn progress(&self, id: TargetId) -> Option<TargetProgress> {
        let chain = &self.targets.get(&id)?.chain;
        Some(TargetProgress {
            kind: chain.current_kind(),
            filtering_confirmed: chain.filtering_confirmed(),
            // A path walk is still a measurement of something, so an endpoint that has
            // exhausted every probe kind is only *unmeasurable* when even that is ruled
            // out — which is the tunnelled case.
            measurable: chain.step() != ChainStep::Nothing,
        })
    }

    /// Gives a target's ruled-out probe kinds another chance.
    ///
    /// Filtering is not permanent, but re-testing costs probes and risks silence on an
    /// endpoint that was being measured perfectly well, so the runner never decides this on
    /// its own. Returns `true` if the target is registered.
    pub fn reconsider(&mut self, id: TargetId, now: Instant) -> bool {
        let Some(state) = self.targets.get_mut(&id) else {
            return false;
        };
        state.chain.reconsider();
        state.backoff.reset();
        self.unmeasurable.retain(|seen| *seen != id);
        if !state.in_flight {
            // Immediately due: the point of reconsidering is to find out now.
            let _ = self.scheduler.schedule(id, self.interval, now);
        }
        true
    }

    /// When the next probe becomes due.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.scheduler.next_deadline()
    }

    /// The probes to carry out right now, most overdue first.
    ///
    /// Each returned target is unscheduled until its report arrives, so a probe that outlives
    /// its own interval is never issued twice.
    pub fn due(&mut self, now: Instant) -> Vec<PlannedProbe> {
        let mut buffer = std::mem::take(&mut self.due_buffer);
        self.scheduler.due(now, &mut buffer);

        let mut planned = Vec::with_capacity(buffer.len());
        for id in buffer.iter().copied() {
            let Some(state) = self.targets.get_mut(&id) else {
                continue;
            };
            match state.chain.step() {
                ChainStep::Probe(kind) => {
                    state.in_flight = true;
                    self.scheduler.unschedule(id);
                    planned.push(PlannedProbe {
                        id,
                        action: PlannedAction::Probe(kind),
                        target: probe_target(state, timeout_for(kind)),
                    });
                }
                ChainStep::WalkThePath => {
                    state.in_flight = true;
                    self.scheduler.unschedule(id);
                    planned.push(PlannedProbe {
                        id,
                        action: PlannedAction::WalkThePath,
                        target: probe_target(state, timeout_for(ProbeKind::IcmpEcho)),
                    });
                }
                ChainStep::Nothing => {
                    self.scheduler.unschedule(id);
                    if !self.unmeasurable.contains(&id) {
                        self.unmeasurable.push(id);
                    }
                }
            }
        }

        buffer.clear();
        self.due_buffer = buffer;
        planned
    }

    /// Folds a completed probe back in and re-schedules its target.
    ///
    /// A report for a target that has since been removed is discarded.
    pub fn complete(&mut self, report: &Report) {
        let Some(state) = self.targets.get_mut(&report.id) else {
            return;
        };
        state.in_flight = false;

        match &report.measured {
            Measured::Probe { outcome, .. } => {
                let before = state.chain.current_kind();
                state.chain.record(*outcome);
                if state.chain.current_kind() == before {
                    state.backoff.record(*outcome);
                } else {
                    // A different kind is taking over. The previous kind's failures say
                    // nothing about it, so it starts at full rate like the first one did.
                    state.backoff.reset();
                }
            }
            // Neither changes what we believe about the endpoint. A walk is already the last
            // resort and maps the route without saying whether the destination recovered, so
            // its interval stays wherever backoff had stretched it to. A failure of our own is
            // reported outward but is not evidence against the probe kind or the endpoint, so
            // neither the chain nor the backoff hears of it.
            Measured::Path(_) | Measured::Failed(_) => {}
        }

        let _ = self
            .scheduler
            .schedule_after(report.id, state.backoff.interval(), report.at);
    }
}

fn probe_target(state: &TargetState, timeout: Duration) -> ProbeTarget {
    let mut target = ProbeTarget::new(state.address, timeout);
    if let Some(source) = state.source {
        target = target.from_source(source);
    }
    target
}

/// Walks the path to a target, behind a `dyn`-safe interface.
///
/// [`PathProbe`] is generic over the platform implementation, which a runner holding several
/// kinds of work cannot name. This is the seam that lets it hold one anyway.
#[async_trait]
pub trait PathWalker: Send + Sync {
    /// Maps the route towards a target.
    ///
    /// # Errors
    ///
    /// Returns an error if a probe could not be carried out at all.
    async fn walk(&self, target: &ProbeTarget) -> Result<PathTrace, Error>;
}

#[async_trait]
impl<P> PathWalker for PathProbe<P>
where
    P: nm_platform::icmp::IcmpProber + 'static,
{
    async fn walk(&self, target: &ProbeTarget) -> Result<PathTrace, Error> {
        self.trace(target).await
    }
}

/// The probers a runner has to work with.
#[derive(Clone, Default)]
pub struct ProberSet {
    by_kind: BTreeMap<ProbeKind, Arc<dyn Prober>>,
    walker: Option<Arc<dyn PathWalker>>,
}

impl ProberSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a prober, replacing any other of the same kind.
    #[must_use]
    pub fn with(mut self, prober: Arc<dyn Prober>) -> Self {
        self.by_kind.insert(prober.kind(), prober);
        self
    }

    /// Adds the path walker used once every probe kind is ruled out.
    #[must_use]
    pub fn with_walker(mut self, walker: Arc<dyn PathWalker>) -> Self {
        self.walker = Some(walker);
        self
    }

    /// Every kind present, in preference-independent order.
    #[must_use]
    pub fn kinds(&self) -> Vec<ProbeKind> {
        self.by_kind.keys().copied().collect()
    }

    /// Carries out one planned probe.
    ///
    /// # Errors
    ///
    /// Returns an error if the work could not be carried out — including a plan naming a kind
    /// this set does not hold, which is a wiring mistake and must be loud.
    pub async fn carry_out(&self, planned: &PlannedProbe) -> Measured {
        match planned.action {
            PlannedAction::Probe(kind) => match self.by_kind.get(&kind) {
                Some(prober) => match prober.probe(&planned.target).await {
                    Ok(outcome) => Measured::Probe { kind, outcome },
                    Err(error) => Measured::Failed(error),
                },
                None => Measured::Failed(Error::NoProberFor { kind }),
            },
            PlannedAction::WalkThePath => match &self.walker {
                Some(walker) => match walker.walk(&planned.target).await {
                    Ok(trace) => Measured::Path(Box::new(trace)),
                    Err(error) => Measured::Failed(error),
                },
                None => Measured::Failed(Error::NoPathWalker),
            },
        }
    }
}

impl std::fmt::Debug for ProberSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The probers themselves are trait objects with nothing useful to print; what a reader
        // needs is which kinds are wired up.
        f.debug_struct("ProberSet")
            .field("kinds", &self.kinds())
            .field("walker", &self.walker.is_some())
            .finish_non_exhaustive()
    }
}

/// A class that no available prober can measure, for callers checking their wiring.
#[must_use]
pub fn unmeasurable_classes(available: &[ProbeKind]) -> Vec<AddressClass> {
    [AddressClass::Routable, AddressClass::TunnelSentinel]
        .into_iter()
        .filter(|class| crate::probe::preferred_kinds(*class, available).is_empty())
        .collect()
}

/// An instruction to a running [`drive`] loop.
#[derive(Debug)]
#[non_exhaustive]
pub enum Command {
    /// Start measuring a target.
    Add {
        /// Handle to track it by.
        id: TargetId,
        /// Where it lives.
        address: TargetAddress,
        /// Local address the probes must egress from, matching the monitored flow.
        source: Option<IpAddr>,
        /// Where to send the registration result, which fails for an address no available
        /// probe kind can honestly measure.
        reply: tokio::sync::oneshot::Sender<Result<(), Error>>,
    },
    /// Stop measuring a target.
    Remove(TargetId),
    /// Give a target's ruled-out probe kinds another chance.
    Reconsider(TargetId),
}

/// How many completed probes may queue up before the loop must drain them.
///
/// Generous relative to the 32 probes/s cap, so a slow consumer of reports never stalls the
/// probing itself.
const COMPLETION_QUEUE: usize = 128;

/// Runs the schedule until the command channel closes, emitting every result on `reports`
/// together with what the runner believes about that target afterwards.
///
/// Shutdown is the caller dropping every [`Command`] sender: the loop finishes, and the
/// runner is handed back so its state can be inspected or reused. Probes still in flight are
/// abandoned — nothing can un-send a packet, and their answers are simply never folded in.
///
/// Returns the runner, and stops early if the report channel closes because nobody is
/// listening any more.
pub async fn drive(
    mut runner: ProbeRunner,
    probers: ProberSet,
    mut commands: tokio::sync::mpsc::Receiver<Command>,
    reports: tokio::sync::mpsc::Sender<Completed>,
) -> ProbeRunner {
    let (done, mut completed) = tokio::sync::mpsc::channel::<Report>(COMPLETION_QUEUE);

    loop {
        for planned in runner.due(Instant::now()) {
            let probers = probers.clone();
            let done = done.clone();
            tokio::spawn(async move {
                let measured = probers.carry_out(&planned).await;
                let report = Report {
                    id: planned.id,
                    at: Instant::now(),
                    measured,
                };
                // The loop has stopped; there is nothing to report to and nothing to do.
                let _ = done.send(report).await;
            });
        }

        tokio::select! {
            command = commands.recv() => match command {
                Some(command) => apply(&mut runner, command),
                None => break,
            },
            Some(report) = completed.recv() => {
                runner.complete(&report);
                // Read after folding the result in: the caller needs the belief that
                // *this* answer produced, not the one it replaced. A target removed
                // meanwhile has no state left, and its stale answer is dropped.
                if let Some(progress) = runner.progress(report.id) {
                    if reports.send(Completed { report, progress }).await.is_err() {
                        break;
                    }
                }
            }
            () = sleep_until(runner.next_deadline()) => {}
        }
    }

    runner
}

fn apply(runner: &mut ProbeRunner, command: Command) {
    match command {
        Command::Add {
            id,
            address,
            source,
            reply,
        } => {
            let outcome = runner.add(id, address, source, Instant::now());
            // The requester gave up waiting; the target is registered either way.
            let _ = reply.send(outcome);
        }
        Command::Remove(id) => {
            runner.remove(id);
        }
        Command::Reconsider(id) => {
            runner.reconsider(id, Instant::now());
        }
    }
}

/// Waits until `deadline`, or forever when nothing is scheduled.
///
/// Waiting forever rather than waking on a fixed tick is what keeps an idle app off the CPU:
/// with no targets there is nothing to poll for, and a command or a completion is what wakes
/// the loop instead.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use nm_core::sample::Rtt;

    use super::*;

    const ALL: &[ProbeKind] = &[
        ProbeKind::IcmpEcho,
        ProbeKind::TcpConnect,
        ProbeKind::TlsHello,
    ];

    /// Eight well-known public resolver addresses, used purely as routable constants.
    ///
    /// The documentation ranges would be the natural choice, but the address policy
    /// classifies them as unusable — correctly, which is exactly why they cannot stand in for
    /// an ordinary endpoint here. Nothing in these tests sends a packet anywhere.
    const ROUTABLE: [Ipv4Addr; 8] = [
        Ipv4Addr::new(1, 1, 1, 1),
        Ipv4Addr::new(1, 0, 0, 1),
        Ipv4Addr::new(8, 8, 8, 8),
        Ipv4Addr::new(8, 8, 4, 4),
        Ipv4Addr::new(9, 9, 9, 9),
        Ipv4Addr::new(149, 112, 112, 112),
        Ipv4Addr::new(208, 67, 222, 222),
        Ipv4Addr::new(208, 67, 220, 220),
    ];

    fn ip(index: u8) -> IpAddr {
        IpAddr::V4(ROUTABLE[usize::from(index)])
    }

    /// Eight distinct handles.
    ///
    /// Minted from a real registry because that is the only way to obtain one: the runner
    /// never interprets a handle, but `TargetId` deliberately has no public constructor, so
    /// nothing can invent one that maps to no address.
    fn ids() -> Vec<TargetId> {
        let mut registry = nm_core::target::TargetRegistry::new();
        (0..8)
            .map(|index| {
                registry
                    .insert(
                        TargetAddress::icmp(ip(index)),
                        nm_core::target::TargetTag::AppEndpoint,
                    )
                    .unwrap()
            })
            .collect()
    }

    fn id(index: u8) -> TargetId {
        ids()[usize::from(index)]
    }

    fn runner(now: Instant) -> ProbeRunner {
        ProbeRunner::new(AddressPolicy::default(), ALL.to_vec(), now).unwrap()
    }

    fn success() -> Measured {
        Measured::Probe {
            kind: ProbeKind::IcmpEcho,
            outcome: ProbeOutcome::Success(Rtt::from_micros(9_000)),
        }
    }

    fn timeout(kind: ProbeKind) -> Measured {
        Measured::Probe {
            kind,
            outcome: ProbeOutcome::Timeout,
        }
    }

    fn report(id: TargetId, at: Instant, measured: Measured) -> Report {
        Report { id, at, measured }
    }

    #[test]
    fn a_new_target_is_probed_at_once_with_the_cheapest_kind() {
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();

        let planned = runner.due(start);
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].id, id(0));
        assert_eq!(planned[0].action, PlannedAction::Probe(ProbeKind::IcmpEcho));
        assert_eq!(planned[0].target.timeout, timeout_for(ProbeKind::IcmpEcho));
    }

    #[test]
    fn a_probe_in_flight_is_not_issued_again_however_long_it_takes() {
        // The rule the whole dispatch design exists for: a six-second TCP probe on a
        // one-second interval must not pile up six copies of itself.
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();
        assert_eq!(runner.due(start).len(), 1);

        for second in 1..=6 {
            let later = start + Duration::from_secs(second);
            assert!(
                runner.due(later).is_empty(),
                "a second copy was issued at +{second}s"
            );
        }
    }

    #[test]
    fn the_interval_runs_from_the_answer_rather_than_the_request() {
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();
        runner.due(start);

        let finished = start + Duration::from_secs(5);
        runner.complete(&report(id(0), finished, success()));

        assert!(runner.due(finished).is_empty());
        assert_eq!(runner.due(finished + DEFAULT_INTERVAL).len(), 1);
    }

    #[test]
    fn a_target_carries_its_egress_binding_into_every_probe() {
        let start = Instant::now();
        let source = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9));
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), Some(source), start)
            .unwrap();

        assert_eq!(runner.due(start)[0].target.source, Some(source));
    }

    #[test]
    fn each_kind_gets_the_deadline_it_needs() {
        // A TCP deadline under Windows' ~2 s refusal delay would fabricate packet loss.
        assert_eq!(timeout_for(ProbeKind::IcmpEcho), Duration::from_secs(1));
        assert!(timeout_for(ProbeKind::TcpConnect) > Duration::from_secs(2));
        assert_eq!(
            timeout_for(ProbeKind::TlsHello),
            timeout_for(ProbeKind::TcpConnect)
        );
    }

    /// Runs `rounds` probes of one target, answering each with `measured`.
    fn run_rounds(runner: &mut ProbeRunner, start: Instant, rounds: u32, measured: &Measured) {
        let mut now = start;
        for _ in 0..rounds {
            let planned = runner.due(now);
            if planned.is_empty() {
                now += Duration::from_millis(100);
                continue;
            }
            runner.complete(&report(planned[0].id, now, measured.clone()));
            now += Duration::from_millis(100);
        }
    }

    #[test]
    fn sustained_silence_stretches_the_interval() {
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();

        // Enough silence to exhaust every kind and then keep failing on the path walk.
        run_rounds(&mut runner, start, 200, &timeout(ProbeKind::IcmpEcho));

        let deadline = runner.next_deadline().expect("still scheduled");
        assert!(
            deadline > start,
            "a permanently silent endpoint must stop costing full-rate budget"
        );
    }

    #[test]
    fn a_recovering_endpoint_returns_to_full_rate() {
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();
        run_rounds(&mut runner, start, 20, &timeout(ProbeKind::IcmpEcho));

        let now = start + Duration::from_secs(60);
        let planned = runner.due(now);
        // Whatever kind it has fallen to, one answer restores the base interval.
        if let Some(first) = planned.first() {
            runner.complete(&report(first.id, now, success()));
            assert_eq!(runner.next_deadline(), Some(now + DEFAULT_INTERVAL));
        }
    }

    #[test]
    fn switching_probe_kind_restores_full_rate() {
        // The failures belonged to the kind that was set aside; charging them to its
        // replacement would start the new attempt already throttled.
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();

        let mut now = start;
        for _ in 0..3 {
            let planned = runner.due(now);
            assert_eq!(planned[0].action, PlannedAction::Probe(ProbeKind::IcmpEcho));
            runner.complete(&report(id(0), now, timeout(ProbeKind::IcmpEcho)));
            now += DEFAULT_INTERVAL;
        }

        assert_eq!(runner.current_kind(id(0)), Some(ProbeKind::TcpConnect));
        assert_eq!(
            runner.next_deadline(),
            Some(now),
            "the new kind starts on the base interval"
        );
    }

    #[test]
    fn an_endpoint_with_no_kinds_left_falls_back_to_walking_the_path() {
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();

        let mut now = start;
        for _ in 0..3 {
            let planned = runner.due(now);
            let kind = match planned[0].action {
                PlannedAction::Probe(kind) => kind,
                PlannedAction::WalkThePath => unreachable!("kinds remain"),
            };
            runner.complete(&report(
                id(0),
                now,
                Measured::Probe {
                    kind,
                    outcome: ProbeOutcome::Blocked,
                },
            ));
            now += DEFAULT_INTERVAL;
        }

        assert_eq!(runner.due(now)[0].action, PlannedAction::WalkThePath);
    }

    #[test]
    fn a_tunnelled_endpoint_with_nothing_left_is_listed_rather_than_dropped() {
        // It stops costing budget, but the UI must be able to say why it went quiet.
        let start = Instant::now();
        let mut runner = runner(start);
        let tunnelled: IpAddr = "198.18.0.7".parse().unwrap();
        runner
            .add(id(1), TargetAddress::with_port(tunnelled, 443), None, start)
            .unwrap();

        let planned = runner.due(start);
        assert_eq!(planned[0].action, PlannedAction::Probe(ProbeKind::TlsHello));
        runner.complete(&report(
            id(1),
            start,
            Measured::Probe {
                kind: ProbeKind::TlsHello,
                outcome: ProbeOutcome::Blocked,
            },
        ));

        let now = start + Duration::from_secs(5);
        assert!(runner.due(now).is_empty());
        assert_eq!(runner.unmeasurable(), &[id(1)]);
        assert_eq!(runner.next_deadline(), None);
    }

    #[test]
    fn reconsidering_brings_an_unmeasurable_endpoint_straight_back() {
        let start = Instant::now();
        let mut runner = runner(start);
        let tunnelled: IpAddr = "198.18.0.7".parse().unwrap();
        runner
            .add(id(1), TargetAddress::with_port(tunnelled, 443), None, start)
            .unwrap();
        runner.due(start);
        runner.complete(&report(
            id(1),
            start,
            Measured::Probe {
                kind: ProbeKind::TlsHello,
                outcome: ProbeOutcome::Blocked,
            },
        ));
        let now = start + Duration::from_secs(5);
        runner.due(now);
        assert_eq!(runner.unmeasurable().len(), 1);

        assert!(runner.reconsider(id(1), now));
        assert!(runner.unmeasurable().is_empty());
        assert_eq!(
            runner.due(now)[0].action,
            PlannedAction::Probe(ProbeKind::TlsHello)
        );
    }

    #[test]
    fn an_endpoint_no_kind_can_measure_is_refused_registration() {
        let start = Instant::now();
        let mut runner = runner(start);
        assert!(runner
            .add(
                id(0),
                TargetAddress::icmp(IpAddr::V4(Ipv4Addr::LOCALHOST)),
                None,
                start,
            )
            .is_err());
        assert!(runner.is_empty());
    }

    #[test]
    fn a_tunnelled_endpoint_is_refused_when_no_end_to_end_prober_exists() {
        let start = Instant::now();
        let mut runner = ProbeRunner::new(
            AddressPolicy::default(),
            vec![ProbeKind::IcmpEcho, ProbeKind::TcpConnect],
            start,
        )
        .unwrap();
        let tunnelled: IpAddr = "198.18.0.7".parse().unwrap();

        assert_eq!(
            runner
                .add(id(1), TargetAddress::with_port(tunnelled, 443), None, start)
                .unwrap_err(),
            Error::NothingUsable {
                class: AddressClass::TunnelSentinel
            }
        );
    }

    #[test]
    fn removing_a_target_stops_it_and_discards_its_outstanding_report() {
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();
        runner.due(start);

        assert!(runner.remove(id(0)));
        assert!(runner.is_empty());
        // The probe was already in flight; its answer must not resurrect the schedule.
        runner.complete(&report(id(0), start + DEFAULT_INTERVAL, success()));
        assert_eq!(runner.next_deadline(), None);
        assert!(!runner.remove(id(0)));
    }

    #[test]
    fn our_own_failure_is_not_charged_to_the_endpoint() {
        // A local socket failure is not the endpoint's packet loss, so it must not set a
        // probe kind aside nor stretch the interval.
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();

        let mut now = start;
        for _ in 0..10 {
            runner.due(now);
            runner.complete(&report(
                id(0),
                now,
                Measured::Failed(Error::SourceFamilyMismatch),
            ));
            now += DEFAULT_INTERVAL;
        }

        assert_eq!(runner.current_kind(id(0)), Some(ProbeKind::IcmpEcho));
        assert_eq!(runner.next_deadline(), Some(now));
    }

    #[test]
    fn the_global_rate_cap_is_respected_across_targets() {
        let start = Instant::now();
        let mut runner = runner(start);
        for raw in 0..8 {
            runner
                .add(id(raw), TargetAddress::icmp(ip(raw)), None, start)
                .unwrap();
        }

        // Eight targets due at once, against a burst allowance of an eighth of a second's
        // budget. The rest stay due rather than being dropped.
        let first = runner.due(start);
        assert!(first.len() < 8, "the burst allowance was exceeded");
        assert!(!first.is_empty());

        let issued = first.len();
        let later = runner.due(start + Duration::from_secs(1));
        assert_eq!(
            issued + later.len(),
            8,
            "targets over budget must stay due, not be discarded"
        );
    }

    #[test]
    fn a_probe_set_reports_what_it_holds() {
        let set = ProberSet::new().with(Arc::new(crate::tcp::TcpConnectProber::new()));
        assert_eq!(set.kinds(), vec![ProbeKind::TcpConnect]);
        assert!(format!("{set:?}").contains("TcpConnect"));
    }

    #[tokio::test]
    async fn a_plan_naming_a_missing_prober_fails_loudly() {
        // A wiring mistake, not a network condition: silently reporting a timeout here would
        // blame the endpoint for the application's own bug.
        let set = ProberSet::new();
        let planned = PlannedProbe {
            id: id(0),
            action: PlannedAction::Probe(ProbeKind::IcmpEcho),
            target: ProbeTarget::new(TargetAddress::icmp(ip(0)), Duration::from_secs(1)),
        };
        assert_eq!(
            set.carry_out(&planned).await,
            Measured::Failed(Error::NoProberFor {
                kind: ProbeKind::IcmpEcho
            })
        );
    }

    #[tokio::test]
    async fn a_path_walk_without_a_walker_fails_loudly() {
        let set = ProberSet::new();
        let planned = PlannedProbe {
            id: id(0),
            action: PlannedAction::WalkThePath,
            target: ProbeTarget::new(TargetAddress::icmp(ip(0)), Duration::from_secs(1)),
        };
        assert_eq!(
            set.carry_out(&planned).await,
            Measured::Failed(Error::NoPathWalker)
        );
    }

    /// A prober that answers instantly with a fixed outcome, counting its calls.
    struct Canned {
        kind: ProbeKind,
        outcome: ProbeOutcome,
        calls: Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait]
    impl Prober for Canned {
        fn kind(&self) -> ProbeKind {
            self.kind
        }

        async fn probe(&self, _target: &ProbeTarget) -> Result<ProbeOutcome, Error> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(self.outcome)
        }
    }

    #[tokio::test]
    async fn the_loop_probes_what_it_is_told_to_and_reports_the_answer() {
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let probers = ProberSet::new().with(Arc::new(Canned {
            kind: ProbeKind::IcmpEcho,
            outcome: ProbeOutcome::Success(Rtt::from_micros(4_000)),
            calls: Arc::clone(&calls),
        }));

        let runner = ProbeRunner::new(
            AddressPolicy::default(),
            vec![ProbeKind::IcmpEcho],
            Instant::now(),
        )
        .unwrap();

        let (commands, command_rx) = tokio::sync::mpsc::channel(4);
        let (report_tx, mut reports) = tokio::sync::mpsc::channel(4);
        let loop_handle = tokio::spawn(drive(runner, probers, command_rx, report_tx));

        let (reply, registered) = tokio::sync::oneshot::channel();
        commands
            .send(Command::Add {
                id: id(0),
                address: TargetAddress::icmp(ip(0)),
                source: None,
                reply,
            })
            .await
            .unwrap();
        registered.await.unwrap().unwrap();

        let completed = reports.recv().await.expect("a report must arrive");
        assert_eq!(completed.report.id, id(0));
        assert_eq!(
            completed.report.measured,
            Measured::Probe {
                kind: ProbeKind::IcmpEcho,
                outcome: ProbeOutcome::Success(Rtt::from_micros(4_000)),
            }
        );
        // The belief travels with the measurement: the UI needs to know which kind
        // produced this number, and that nothing has been proven filtered.
        assert_eq!(
            completed.progress,
            TargetProgress {
                kind: Some(ProbeKind::IcmpEcho),
                filtering_confirmed: false,
                measurable: true,
            }
        );

        // Dropping the last command sender is the shutdown signal.
        drop(commands);
        let runner = loop_handle.await.unwrap();
        assert_eq!(runner.len(), 1);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn the_loop_refuses_a_target_nothing_can_measure_and_says_so() {
        let probers = ProberSet::new();
        let runner = ProbeRunner::new(
            AddressPolicy::default(),
            vec![ProbeKind::IcmpEcho],
            Instant::now(),
        )
        .unwrap();

        let (commands, command_rx) = tokio::sync::mpsc::channel(4);
        let (report_tx, _reports) = tokio::sync::mpsc::channel(4);
        let loop_handle = tokio::spawn(drive(runner, probers, command_rx, report_tx));

        let (reply, registered) = tokio::sync::oneshot::channel();
        commands
            .send(Command::Add {
                id: id(1),
                address: TargetAddress::with_port("198.18.0.7".parse().unwrap(), 443),
                source: None,
                reply,
            })
            .await
            .unwrap();

        assert_eq!(
            registered.await.unwrap().unwrap_err(),
            Error::NothingUsable {
                class: AddressClass::TunnelSentinel
            },
            "a caller must learn at once that this endpoint will never be measured"
        );

        drop(commands);
        assert!(loop_handle.await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_loop_stops_when_nobody_is_listening_to_reports() {
        let probers = ProberSet::new().with(Arc::new(Canned {
            kind: ProbeKind::IcmpEcho,
            outcome: ProbeOutcome::Timeout,
            calls: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }));
        let runner = ProbeRunner::new(
            AddressPolicy::default(),
            vec![ProbeKind::IcmpEcho],
            Instant::now(),
        )
        .unwrap();

        let (commands, command_rx) = tokio::sync::mpsc::channel(4);
        let (report_tx, reports) = tokio::sync::mpsc::channel(1);
        let loop_handle = tokio::spawn(drive(runner, probers, command_rx, report_tx));

        let (reply, registered) = tokio::sync::oneshot::channel();
        commands
            .send(Command::Add {
                id: id(0),
                address: TargetAddress::icmp(ip(0)),
                source: None,
                reply,
            })
            .await
            .unwrap();
        registered.await.unwrap().unwrap();
        drop(reports);

        // Probing on with nowhere to send the answers would be pure waste.
        loop_handle.await.unwrap();
        drop(commands);
    }

    #[test]
    fn progress_tracks_the_chain_through_a_fallback() {
        let start = Instant::now();
        let mut runner = runner(start);
        runner
            .add(id(0), TargetAddress::icmp(ip(0)), None, start)
            .unwrap();

        assert_eq!(
            runner.progress(id(0)),
            Some(TargetProgress {
                kind: Some(ProbeKind::IcmpEcho),
                filtering_confirmed: false,
                measurable: true,
            })
        );

        // Echoes are filtered; the next kind takes over and its first success is what
        // *proves* the filtering rather than merely suggesting it.
        runner.due(start);
        runner.complete(&report(
            id(0),
            start,
            Measured::Probe {
                kind: ProbeKind::IcmpEcho,
                outcome: ProbeOutcome::Blocked,
            },
        ));
        let stepped = runner.progress(id(0)).unwrap();
        assert_eq!(stepped.kind, Some(ProbeKind::TcpConnect));
        assert!(!stepped.filtering_confirmed, "silence alone proves nothing");

        let later = start + DEFAULT_INTERVAL;
        runner.due(later);
        runner.complete(&report(
            id(0),
            later,
            Measured::Probe {
                kind: ProbeKind::TcpConnect,
                outcome: ProbeOutcome::Success(Rtt::from_micros(12_000)),
            },
        ));
        assert!(runner.progress(id(0)).unwrap().filtering_confirmed);
    }

    #[test]
    fn a_tunnelled_endpoint_out_of_kinds_reports_itself_unmeasurable() {
        // A routable endpoint always keeps the path walk; a tunnelled one does not, and
        // that difference is what the UI has to show.
        let start = Instant::now();
        let mut runner = runner(start);
        let tunnelled: IpAddr = "198.18.0.7".parse().unwrap();
        runner
            .add(id(1), TargetAddress::with_port(tunnelled, 443), None, start)
            .unwrap();
        runner.due(start);
        runner.complete(&report(
            id(1),
            start,
            Measured::Probe {
                kind: ProbeKind::TlsHello,
                outcome: ProbeOutcome::Blocked,
            },
        ));

        let progress = runner.progress(id(1)).unwrap();
        assert_eq!(progress.kind, None);
        assert!(!progress.measurable);
        assert_eq!(
            runner.progress(id(0)),
            None,
            "an unregistered target has no state"
        );
    }

    #[test]
    fn classes_nothing_can_measure_are_reported() {
        assert!(unmeasurable_classes(ALL).is_empty());
        assert_eq!(
            unmeasurable_classes(&[ProbeKind::IcmpEcho]),
            vec![AddressClass::TunnelSentinel],
            "without an end-to-end prober a tunnelled endpoint cannot be measured at all"
        );
        assert_eq!(unmeasurable_classes(&[]).len(), 2);
    }
}
