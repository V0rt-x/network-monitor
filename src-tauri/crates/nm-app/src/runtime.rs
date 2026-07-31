//! The tokio side of measurement: probing the baselines and the monitored applications'
//! endpoints, and pushing the result.
//!
//! One task owns everything mutable — the [`BaselineMonitor`], the [`AppMonitor`], the
//! target registry they share, and the channel that keeps the probe engine's own loop
//! alive — so nothing is shared between threads and no lock is ever held across an
//! `.await`. It talks to the rest of the app through [`MonitorCommand`].
//!
//! # One probe engine, not one per feature
//!
//! The 32 probes/s cap in `CLAUDE.md` is global: it covers the baselines, every monitored
//! application and, later, the status page together. A second [`ProbeRunner`] would have a
//! second token bucket and quietly double the traffic the product promises not to send, so
//! there is exactly one — and therefore exactly one [`TargetRegistry`], because two
//! registries would hand the same handle to two different addresses and cross-feed their
//! measurements.
//!
//! # Rust drives, the UI renders
//!
//! Sampling is never driven by the UI. The probe engine runs at whatever interval the
//! settings say, and this task pushes a snapshot at [`EMIT_PERIOD`] — well inside the
//! ≤ 4 Hz budget. **While the window is hidden nothing is emitted at all**: the core keeps
//! measuring and the history keeps filling, but a hidden `WebView` is never woken to lay
//! out a chart nobody can see. Showing the window emits at once rather than waiting out
//! the period, so it is never blank. Discovery and probing do not pause with it.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nm_core::address::{AddressClass, AddressPolicy};
use nm_core::endpoint::LifecyclePolicy;
use nm_core::health::HealthThresholds;
use nm_core::sample::ProbeSample;
use nm_core::target::{TargetId, TargetRegistry, TargetTag};
use nm_platform::process::Pid;
use nm_probes::icmp::IcmpEchoProber;
use nm_probes::path::PathProbe;
use nm_probes::runner::{
    Command as ProbeCommand, Completed, Measured, ProbeRunner, ProberSet, MAX_INTERVAL,
};
use nm_probes::tcp::TcpConnectProber;
use nm_probes::tls::TlsHelloProber;
use tauri::{AppHandle, Wry};
use tauri_specta::Event as _;
use tokio::sync::mpsc;

use crate::apps::{AppMonitor, TargetChange};
use crate::baselines::{self, BaselineGroup, BaselineTarget};
use crate::discovery::{self, Discovery, Observation};
use crate::monitor::{health_window, BaselineMonitor};
use crate::settings::Settings;
use crate::Error;

/// How often a snapshot is pushed to a visible window.
///
/// One hertz: the ≤ 4 Hz IPC budget is a ceiling, not a target, and baselines are probed
/// every few seconds — emitting faster would only resend the same numbers.
pub const EMIT_PERIOD: Duration = Duration::from_secs(1);

/// How many completed probes may queue before the loop must drain them.
const REPORT_QUEUE: usize = 64;

/// How many commands may queue for the monitor task.
const COMMAND_QUEUE: usize = 16;

/// An instruction to the monitor task.
#[derive(Debug)]
#[non_exhaustive]
pub enum MonitorCommand {
    /// Settings changed; rebuild with the new country and interval.
    ///
    /// Boxed because it dwarfs the other variants, and an enum is as large as its largest
    /// member.
    Reconfigure(Box<Settings>),
    /// The window has just been shown; push a snapshot without waiting out the period.
    ///
    /// Visibility itself is read from the shared flag this task was given — this command
    /// only exists so a freshly shown window is never blank for a second.
    WindowRevealed,
    /// Start discovering and probing one process's endpoints.
    ///
    /// Refused past the five-application cap; the refusal is not reported back because no
    /// caller can act on it yet. The process picker, which is where a user can be told,
    /// arrives with the app-monitor page.
    MonitorApp(Pid),
    /// Stop following a process and release the endpoints nothing else uses.
    ForgetApp(Pid),
}

/// Handle for talking to the running monitor task.
#[derive(Debug, Clone)]
pub struct MonitorHandle {
    commands: mpsc::Sender<MonitorCommand>,
}

impl MonitorHandle {
    /// Sends a command from a synchronous context, such as a window event handler.
    ///
    /// A task that has already stopped is ignored: that only happens as the app shuts
    /// down, when there is nothing useful left to do about it.
    pub fn send(&self, command: MonitorCommand) {
        let commands = self.commands.clone();
        tauri::async_runtime::spawn(async move {
            let _ = commands.send(command).await;
        });
    }
}

/// Starts monitoring the baselines and pushing health snapshots to `app`.
///
/// `visible` is the single source of truth for whether the window can be drawn to; the task
/// reads it before every emission rather than keeping a copy that could drift.
#[must_use]
pub fn spawn(app: AppHandle<Wry>, settings: Settings, visible: Arc<AtomicBool>) -> MonitorHandle {
    let (commands, receiver) = mpsc::channel(COMMAND_QUEUE);
    tauri::async_runtime::spawn(run(app, settings, receiver, visible));
    MonitorHandle { commands }
}

/// Why a monitoring session ended.
enum SessionEnd {
    /// Settings changed; start again with these.
    Reconfigure(Settings),
    /// Every command sender is gone: the app is shutting down.
    Shutdown,
}

/// Runs one session after another, rebuilding whenever the settings change.
///
/// The set of monitored processes lives out here, above the session, so that changing the
/// country or the probe interval does not silently stop following the user's game. Their
/// measured history does not survive — the session that held it is gone — and that is the
/// same bargain the baselines make.
async fn run(
    app: AppHandle<Wry>,
    mut settings: Settings,
    mut commands: mpsc::Receiver<MonitorCommand>,
    visible: Arc<AtomicBool>,
) {
    let started = Instant::now();
    let mut watched: BTreeSet<Pid> = BTreeSet::new();

    loop {
        let end = session(
            &app,
            &settings,
            &mut commands,
            &visible,
            started,
            &mut watched,
        )
        .await;
        match end {
            SessionEnd::Reconfigure(next) => settings = next,
            SessionEnd::Shutdown => return,
        }
    }
}

/// Runs one monitoring session for a fixed set of settings.
// Long because it is a wiring function: everything it owns is created here so that ending
// the session drops all of it at once, and splitting the setup out would only move the
// declarations away from the loop that borrows them. The decisions themselves live in the
// helpers below and in the crates underneath.
#[allow(clippy::too_many_lines)]
async fn session(
    app: &AppHandle<Wry>,
    settings: &Settings,
    commands: &mut mpsc::Receiver<MonitorCommand>,
    visible: &AtomicBool,
    started: Instant,
    watched: &mut BTreeSet<Pid>,
) -> SessionEnd {
    let interval = settings.baseline_interval();
    let policy = AddressPolicy::default();
    let lifecycle = LifecyclePolicy::default();
    let mut baselines = BaselineMonitor::new(HealthThresholds::default(), health_window(interval));
    let mut apps = match AppMonitor::new(
        policy.clone(),
        lifecycle,
        HealthThresholds::default(),
        health_window(lifecycle.active_interval),
    ) {
        Ok(apps) => apps,
        Err(error) => {
            report_startup_failure(&error);
            return SessionEnd::Shutdown;
        }
    };
    // One registry for the whole session: see the module documentation on why there cannot
    // be two.
    let mut registry = TargetRegistry::new();

    // Holding this sender for the session's lifetime is what keeps the probe engine's loop
    // alive; dropping it on the way out is what stops it.
    let (probe_commands, probe_receiver) = mpsc::channel::<ProbeCommand>(COMMAND_QUEUE);
    let (report_sender, mut reports) = mpsc::channel::<Completed>(REPORT_QUEUE);
    let (refusals, mut refused) = mpsc::channel::<TargetId>(COMMAND_QUEUE);

    match build(settings, &mut baselines, &mut registry, &policy).await {
        Ok(engine) => {
            tauri::async_runtime::spawn(nm_probes::runner::drive(
                engine.runner,
                engine.probers,
                probe_receiver,
                report_sender,
            ));
        }
        Err(error) => report_startup_failure(&error),
    }

    let (discovery, mut observations) = Discovery::start(
        policy,
        nm_platform::connection::system_table(),
        nm_platform::flow::system_flow_source(),
    );
    let mut pending: VecDeque<ProbeCommand> = VecDeque::new();
    // Applications the user chose before this session started — a settings change, most
    // likely — are picked up again rather than quietly forgotten.
    watched.retain(|pid| apps.monitor(discovery::app_of(*pid)).is_ok());
    discovery.watch(&pids_of(watched));

    let mut ticker = tokio::time::interval(EMIT_PERIOD);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        flush(&mut pending, &probe_commands);

        tokio::select! {
            command = commands.recv() => match command {
                Some(MonitorCommand::Reconfigure(next)) => return SessionEnd::Reconfigure(*next),
                Some(MonitorCommand::WindowRevealed) => emit(app, &baselines, started),
                Some(MonitorCommand::MonitorApp(pid)) => {
                    if apps.monitor(discovery::app_of(pid)).is_ok() {
                        watched.insert(pid);
                        discovery.watch(&pids_of(watched));
                    }
                }
                Some(MonitorCommand::ForgetApp(pid)) => {
                    watched.remove(&pid);
                    discovery.watch(&pids_of(watched));
                    let changes = apps.forget(&mut registry, discovery::app_of(pid));
                    queue(&mut pending, changes, &refusals);
                }
                None => return SessionEnd::Shutdown,
            },
            Some(completed) = reports.recv() => fold_in(&mut baselines, &mut apps, &completed),
            Some(observation) = observations.recv() => observe(&mut apps, observation),
            Some(id) = refused.recv() => apps.note_unmeasurable(id),
            _ = ticker.tick() => {
                // Discovery is folded in on every tick, visible or not: hidden means "stop
                // drawing", never "stop measuring".
                let changes = apps.sweep(&mut registry, Instant::now());
                queue(&mut pending, changes, &refusals);
                if visible.load(Ordering::Relaxed) {
                    emit(app, &baselines, started);
                }
            }
        }
    }
}

/// The watched processes as the discovery sources want them.
fn pids_of(watched: &BTreeSet<Pid>) -> Vec<Pid> {
    watched.iter().copied().collect()
}

/// Records one sighting from discovery.
///
/// An observation for an application that is no longer monitored is dropped: the user can
/// stop watching a process between a poll being taken and its rows arriving, and that race
/// is normal rather than a fault.
fn observe(apps: &mut AppMonitor, observation: Observation) {
    let _ = apps.observe(
        observation.app,
        observation.endpoint,
        observation.source,
        observation.bytes,
        Instant::now(),
    );
}

/// Turns the app monitor's decisions into instructions for the probe engine.
///
/// Queued rather than sent, because the session must never block waiting for the engine to
/// take a command: the engine can be waiting for this task to take a report at the same
/// moment, and two full channels facing each other is a deadlock. [`flush`] empties the
/// queue as fast as the engine will accept, in order and without dropping anything.
fn queue(
    pending: &mut VecDeque<ProbeCommand>,
    changes: Vec<TargetChange>,
    refusals: &mpsc::Sender<TargetId>,
) {
    for change in changes {
        match change {
            TargetChange::Register {
                id,
                address,
                source,
                interval,
            } => {
                let (reply, answer) = tokio::sync::oneshot::channel();
                pending.push_back(ProbeCommand::Add {
                    id,
                    address,
                    source,
                    reply,
                });
                // The runner registers at the baselines' interval; an endpoint's cadence
                // comes from the lifecycle policy, so it is stated immediately after.
                pending.push_back(ProbeCommand::SetInterval { id, interval });
                watch_registration(id, answer, refusals.clone());
            }
            TargetChange::SetInterval { id, interval } => {
                pending.push_back(ProbeCommand::SetInterval { id, interval });
            }
            TargetChange::Unregister { id } => pending.push_back(ProbeCommand::Remove(id)),
        }
    }
}

/// Reports back an endpoint the probe engine refused to register.
///
/// The engine answers on a oneshot, and waiting for it inline would stall the session for
/// as long as the engine takes. The refusal means no available probe kind can honestly
/// measure the address, and the endpoint must then say so rather than sit in the list
/// mysteriously never updating.
fn watch_registration(
    id: TargetId,
    answer: tokio::sync::oneshot::Receiver<Result<(), nm_probes::Error>>,
    refusals: mpsc::Sender<TargetId>,
) {
    tauri::async_runtime::spawn(async move {
        // A dropped sender means the engine stopped before answering; the session is ending
        // and there is nothing left to record.
        if let Ok(Err(_)) = answer.await {
            let _ = refusals.send(id).await;
        }
    });
}

/// Hands as many queued instructions to the probe engine as it will take right now.
fn flush(pending: &mut VecDeque<ProbeCommand>, probe_commands: &mpsc::Sender<ProbeCommand>) {
    while let Some(command) = pending.pop_front() {
        match probe_commands.try_send(command) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(command)) => {
                // Back to the head, so ordering holds: an interval must never overtake the
                // registration it belongs to.
                pending.push_front(command);
                return;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // The engine never started, or has stopped. Nothing queued can be carried
                // out, and holding on to it would grow without bound.
                pending.clear();
                return;
            }
        }
    }
}

/// Folds one completed probe into the state of whatever asked for it.
///
/// Split out from the loop because it is the one place a measurement becomes history, and
/// what it declines to record matters as much as what it records. Both monitors are told:
/// each ignores a handle it does not know, and an address that is *both* a baseline and an
/// application's endpoint is one target whose single measurement legitimately answers for
/// both — which is what the shared registry exists to arrange.
fn fold_in(baselines: &mut BaselineMonitor, apps: &mut AppMonitor, completed: &Completed) {
    let id = completed.report.id;
    baselines.note_probe_state(
        id,
        completed.progress.kind,
        completed.progress.filtering_confirmed,
        completed.progress.measurable,
    );
    apps.note_probe_state(
        id,
        completed.progress.kind,
        completed.progress.filtering_confirmed,
        completed.progress.measurable,
    );

    // Only a probe result becomes history. A path walk maps the route to a silent endpoint
    // but measures no round trip *to* it, so recording one would invent a sample; a failure
    // of our own is not the endpoint's packet loss. Neither belongs in what the verdict
    // reads, and neither does a future variant this build has never heard of.
    if let Measured::Probe { outcome, .. } = &completed.report.measured {
        let sample = ProbeSample::new(completed.report.at, *outcome);
        baselines.record(id, sample);
        apps.record(id, sample);
    }
}

/// Pushes a snapshot to the window.
fn emit(app: &AppHandle<Wry>, monitor: &BaselineMonitor, started: Instant) {
    let uptime = nm_core::time::elapsed_secs(started.elapsed());
    // A failed emit means the window is gone; the next tick finds that out too, and there
    // is nothing to recover.
    let _ = monitor.snapshot(Instant::now(), uptime).emit(app);
}

/// The probe engine, built and populated for one session.
struct Engine {
    runner: ProbeRunner,
    probers: ProberSet,
}

/// Builds the probe engine and registers every baseline target with it.
async fn build(
    settings: &Settings,
    monitor: &mut BaselineMonitor,
    registry: &mut TargetRegistry,
    policy: &AddressPolicy,
) -> Result<Engine, Error> {
    let probers = probers();
    let now = Instant::now();
    let mut runner = ProbeRunner::new(policy.clone(), probers.kinds(), now)?
        .with_intervals(settings.baseline_interval(), MAX_INTERVAL);

    let domestic = baselines::domestic(&settings.country)?;
    let foreign = baselines::foreign()?;
    let mut targets = baselines::resolve_list(BaselineGroup::Domestic, &domestic).await;
    targets.extend(baselines::resolve_list(BaselineGroup::Foreign, &foreign).await);

    for target in &targets {
        register(registry, &mut runner, monitor, policy, target, now)?;
    }

    Ok(Engine { runner, probers })
}

/// Registers one baseline target with the registry, the runner and the monitor.
fn register(
    registry: &mut TargetRegistry,
    runner: &mut ProbeRunner,
    monitor: &mut BaselineMonitor,
    policy: &AddressPolicy,
    target: &BaselineTarget,
    now: Instant,
) -> Result<(), Error> {
    let tag = match target.group {
        BaselineGroup::Domestic => TargetTag::DomesticBaseline,
        BaselineGroup::Foreign => TargetTag::ForeignBaseline,
    };

    let Some(address) = target.address else {
        // The name never resolved. The entry stays on the list, visible and unmeasured,
        // because a foreign baseline that quietly shrank would read as good news.
        monitor.add(target, None, false)?;
        return Ok(());
    };

    let tunnelled = policy.classify(address.ip) == AddressClass::TunnelSentinel;
    let id = registry.insert(address, tag)?;
    monitor.add(target, Some(id), tunnelled)?;

    if runner.add(id, address, None, now).is_err() {
        // No probe kind can honestly measure this address — a tunnelled endpoint with no
        // end-to-end prober, or a range not worth probing at all. Said out loud rather
        // than shown as a target that mysteriously never updates.
        monitor.note_unmeasurable(id);
    }
    Ok(())
}

/// The probers this build has.
///
/// ICMP is present only where `nm-platform` has an implementation; without it the engine
/// runs on its connecting kinds rather than failing. That degradation is visible — every
/// target reports which kind produced its number — instead of silent.
fn probers() -> ProberSet {
    let mut set = ProberSet::new()
        .with(Arc::new(TcpConnectProber::new()))
        .with(Arc::new(TlsHelloProber::new()));

    if let Ok(platform) = nm_platform::icmp::system_prober() {
        set = set.with(Arc::new(IcmpEchoProber::new(platform)));
    }
    // A second handle: the path walk is a different consumer of the same capability, and
    // both own theirs rather than sharing one across tasks.
    if let Ok(platform) = nm_platform::icmp::system_prober() {
        set = set.with_walker(Arc::new(PathProbe::new(platform)));
    }
    set
}

/// Reports a session that could not start.
///
/// The bundled lists are validated by tests and the country is sanitized before it gets
/// here, so this is close to unreachable. It exists so that if it ever does happen there is
/// a trace — and the window still receives a snapshot saying "unknown" everywhere, which is
/// honest, rather than silence, which looks like a dead core.
fn report_startup_failure(error: &Error) {
    eprintln!("network-monitor: baseline monitoring could not start: {error}");
}
