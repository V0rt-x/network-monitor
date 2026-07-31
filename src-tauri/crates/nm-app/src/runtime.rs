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

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nm_core::address::{AddressClass, AddressPolicy};
use nm_core::endpoint::{AppId, LifecyclePolicy};
use nm_core::health::HealthThresholds;
use nm_core::sample::ProbeSample;
use nm_core::target::{TargetId, TargetRegistry, TargetTag};
use nm_platform::interface::InterfaceNames;
use nm_platform::process::{Pid, ProcessInfo};
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

use crate::applications::{Application, Applications};
use crate::apps::{AppMonitor, TargetChange};
use crate::baselines::{self, BaselineGroup, BaselineTarget};
use crate::discovery::{Discovery, FlowStatus, Observation};
use crate::events::AppEndpoints;
use crate::monitor::{health_window, BaselineMonitor};
use crate::presets::PresetList;
use crate::settings::Settings;
use crate::view::{AppProcessView, AppView};
use crate::Error;

/// How often a snapshot is pushed to a visible window.
///
/// One hertz: the ≤ 4 Hz IPC budget is a ceiling, not a target, and baselines are probed
/// every few seconds — emitting faster would only resend the same numbers.
pub const EMIT_PERIOD: Duration = Duration::from_secs(1);

/// How often each application's set of processes is recomputed.
///
/// Much slower than the discovery beat, and the reason is measured rather than assumed: a
/// Toolhelp sweep of a real desktop — 284 processes — takes about 8 ms, so doing it once a
/// second would spend most of the product's whole 1 % CPU budget on bookkeeping. A test in
/// `nm-platform` pins that figure so it cannot quietly grow.
///
/// What five seconds costs is latency: a game started by its launcher joins the
/// application, and its endpoints start being measured, up to five seconds after it
/// appears. That is the right trade for the case this exists to serve — arming the monitor
/// *before* a match — and the sweep does not run at all while nothing is monitored.
///
/// A user action does not wait for it: choosing a process takes its own snapshot.
pub const MEMBERSHIP_PERIOD: Duration = Duration::from_secs(5);

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
    /// Start monitoring the application one running process belongs to.
    ///
    /// A process identifier is the *seed*, not the identity: the application is formed
    /// around it — its namesakes and its descendants — and keeps its own identity as those
    /// processes come and go. Refused past the five-application cap, and for a process that
    /// is already part of a monitored application.
    MonitorApp(Pid),
    /// Stop following an application and release the endpoints nothing else uses.
    ForgetApp(AppId),
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
/// The monitored applications live out here, above the session, so that changing the
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
    let presets = PresetList::bundled().unwrap_or_else(|error| {
        // The bundled file is validated by a test, so this is close to unreachable. If it
        // ever happens, grouping by executable name and by process tree still works and the
        // awkward titles are the only casualty — which is a far better outcome than
        // refusing to monitor anything.
        eprintln!("network-monitor: the application presets could not be loaded: {error}");
        PresetList::empty()
    });
    let mut applications = Applications::new(presets);

    loop {
        let end = session(
            &app,
            &settings,
            &mut commands,
            &visible,
            started,
            &mut applications,
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
    applications: &mut Applications,
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

    let (mut discovery, mut observations) = Discovery::start(
        policy,
        nm_platform::connection::system_table(),
        nm_platform::flow::system_flow_source(),
    );
    let mut pending: VecDeque<ProbeCommand> = VecDeque::new();
    // Applications the user chose before this session started — a settings change, most
    // likely — are picked up again rather than quietly forgotten.
    for id in applications.iter().map(Application::id).collect::<Vec<_>>() {
        let _ = apps.monitor(id);
    }
    discovery.watch(&applications.watched_pids());

    let mut ticker = tokio::time::interval(EMIT_PERIOD);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut membership = tokio::time::interval(MEMBERSHIP_PERIOD);
    membership.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Which adapter each local address belongs to. Re-read on the same beat rather than
    // once, because a VPN or an accelerator coming up is exactly the event these labels
    // exist to make visible — and it is 2.5 ms of read-only enumeration when it happens.
    let mut interfaces = read_interfaces();

    loop {
        flush(&mut pending, &probe_commands);

        tokio::select! {
            command = commands.recv() => match command {
                Some(MonitorCommand::Reconfigure(next)) => return SessionEnd::Reconfigure(*next),
                Some(MonitorCommand::WindowRevealed) => {
                    emit_health(app, &baselines, started);
                    emit_apps(app, &apps, applications, &interfaces, discovery.flow_status());
                }
                Some(MonitorCommand::MonitorApp(pid)) => {
                    // A fresh snapshot rather than the last periodic one: the user has just
                    // clicked a process the picker listed a moment ago, and a game that
                    // started since the last sweep must not be refused as "not running".
                    if let Some(snapshot) = snapshot_processes().await {
                        if let Some(id) = applications.adopt(pid, &snapshot) {
                            if apps.monitor(id).is_err() {
                                applications.forget(id);
                            }
                            discovery.watch(&applications.watched_pids());
                        }
                    }
                }
                Some(MonitorCommand::ForgetApp(id)) => {
                    if applications.forget(id) {
                        discovery.watch(&applications.watched_pids());
                        let changes = apps.forget(&mut registry, id);
                        queue(&mut pending, changes, &refusals);
                    }
                }
                None => return SessionEnd::Shutdown,
            },
            Some(completed) = reports.recv() => {
                let changes = fold_in(&mut baselines, &mut apps, &mut registry, &completed);
                queue(&mut pending, changes, &refusals);
            }
            Some(observation) = observations.recv() => observe(&mut apps, applications, observation),
            Some(id) = refused.recv() => apps.note_unmeasurable(id),
            _ = membership.tick(), if !applications.is_empty() => {
                // What an application consists of changes underneath it: a launcher exits
                // once the title is running, an anti-cheat re-launches the game, a helper
                // comes and goes. Nothing is asked of the operating system while no
                // application is monitored, which is what the guard is for.
                if let Some(snapshot) = snapshot_processes().await {
                    applications.refresh(&snapshot);
                    discovery.watch(&applications.watched_pids());
                }
                interfaces = read_interfaces();
            }
            _ = ticker.tick() => {
                // All of this runs whether the window is visible or not: hidden means "stop
                // drawing", never "stop measuring" — and a tracing session that fell over
                // while the app was in the tray must be back before the user looks again.
                discovery.revive_flow();
                let changes = apps.sweep(&mut registry, Instant::now());
                queue(&mut pending, changes, &refusals);
                if visible.load(Ordering::Relaxed) {
                    emit_health(app, &baselines, started);
                    emit_apps(app, &apps, applications, &interfaces, discovery.flow_status());
                }
            }
        }
    }
}

/// Takes a process snapshot without blocking the monitoring loop's thread.
///
/// A Toolhelp sweep is roughly 8 ms of synchronous system call — an eternity on an async
/// worker, which is why it goes to the blocking pool. The loop still waits for the answer,
/// because it has nothing useful to do without one; what it must not do is occupy a runtime
/// thread while the kernel copies a few hundred process entries.
async fn snapshot_processes() -> Option<Vec<ProcessInfo>> {
    tauri::async_runtime::spawn_blocking(|| {
        let enumerator = nm_platform::process::system_enumerator().ok()?;
        enumerator.processes().ok()
    })
    .await
    .ok()
    .flatten()
}

/// Names the adapters the machine's local addresses belong to.
///
/// Synchronous rather than on the blocking pool, unlike the process sweep: it is a single
/// read-only enumeration measured at ~2.5 ms, which is short enough to leave on the runtime
/// thread and far too short to justify a task hop. Failure is an empty snapshot — every
/// egress address then shows as an address with no adapter name, which is what a platform
/// with no backend does too, and is honest either way.
fn read_interfaces() -> InterfaceNames {
    nm_platform::interface::system_table()
        .and_then(|table| table.interfaces())
        .map(|interfaces| InterfaceNames::of(&interfaces))
        .unwrap_or_default()
}

/// Records one sighting from discovery, if it belongs to a monitored application.
///
/// The process-to-application mapping is applied here and nowhere else. A process the user
/// stopped watching between a poll being taken and its rows arriving maps to nothing and
/// the sighting is dropped — that race is normal rather than a fault, and so is a process
/// that has not yet been adopted into the application it will belong to.
fn observe(apps: &mut AppMonitor, applications: &Applications, observation: Observation) {
    let Some(app) = applications.app_of(observation.pid) else {
        return;
    };
    let _ = apps.observe(
        app,
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
            TargetChange::SetSource { id, source } => {
                pending.push_back(ProbeCommand::SetSource { id, source });
            }
            TargetChange::Unregister { id } => pending.push_back(ProbeCommand::Remove(id)),
            TargetChange::WalkNow { id } => pending.push_back(ProbeCommand::WalkNow(id)),
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
///
/// Returns whatever the probe engine must be told as a result, which is how a walked route
/// turns into the hops that will be probed along it.
fn fold_in(
    baselines: &mut BaselineMonitor,
    apps: &mut AppMonitor,
    registry: &mut TargetRegistry,
    completed: &Completed,
) -> Vec<TargetChange> {
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

    // Only a probe result becomes history *for the endpoint*. A path walk maps the route to
    // a silent endpoint but measures no round trip *to* it, so recording one would invent a
    // sample; a failure of our own is not the endpoint's packet loss. Neither belongs in what
    // the verdict reads, and neither does a future variant this build has never heard of.
    match &completed.report.measured {
        Measured::Probe { outcome, .. } => {
            let sample = ProbeSample::new(completed.report.at, *outcome);
            baselines.record(id, sample);
            apps.record(id, sample);
            Vec::new()
        }
        // The walk's own product: the hops that will stand in for an endpoint nothing can
        // measure directly, each probed from here on as an ordinary target.
        Measured::Path(trace) => apps.note_path_trace(registry, id, trace, completed.report.at),
        _ => Vec::new(),
    }
}

/// Pushes the general-health snapshot to the window.
fn emit_health(app: &AppHandle<Wry>, monitor: &BaselineMonitor, started: Instant) {
    let uptime = nm_core::time::elapsed_secs(started.elapsed());
    // A failed emit means the window is gone; the next tick finds that out too, and there
    // is nothing to recover.
    let _ = monitor.snapshot(Instant::now(), uptime).emit(app);
}

/// Pushes every monitored application's endpoints to the window.
///
/// Sent even with nothing monitored, because the page still has to say whether flow events
/// are available: without them there are no UDP endpoints and no byte counters anywhere,
/// and an empty list must not be mistaken for an application that is quiet.
fn emit_apps(
    app: &AppHandle<Wry>,
    apps: &AppMonitor,
    applications: &Applications,
    interfaces: &InterfaceNames,
    flow_status: FlowStatus,
) {
    let now = Instant::now();
    let views = applications
        .iter()
        .map(|application| {
            let reports = apps.endpoints(application.id(), now);
            let processes = application
                .members()
                .iter()
                .map(|member| AppProcessView {
                    pid: member.pid.get(),
                    name: member.name.clone(),
                })
                .collect();
            AppView::of(
                application.id().get(),
                application.label().to_owned(),
                processes,
                apps.chart_ages_secs(),
                interfaces,
                &reports,
            )
        })
        .collect();

    let payload = AppEndpoints {
        window_secs: nm_core::time::elapsed_secs(apps.window()),
        traffic_window_secs: nm_core::time::elapsed_secs(apps.traffic_window()),
        chart_step_secs: nm_core::time::elapsed_secs(crate::apps::CHART_STEP),
        flow_status: flow_status.into(),
        apps: views,
    };
    let _ = payload.emit(app);
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
