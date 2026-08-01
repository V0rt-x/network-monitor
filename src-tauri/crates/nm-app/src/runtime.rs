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
use std::time::{Duration, Instant, SystemTime};

use nm_core::address::{AddressClass, AddressPolicy};
use nm_core::endpoint::{AppId, LifecyclePolicy};
use nm_core::health::HealthThresholds;
use nm_core::sample::ProbeSample;
use nm_core::status::StatusThresholds;
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
use crate::discovery::{Discovery, FlowStatus, Sighting};
use crate::events::AppEndpoints;
use crate::monitor::{health_window, BaselineMonitor};
use crate::pools::{LearnedPools, PoolMonitor, PoolSeeds};
use crate::presets::PresetList;
use crate::services::{self, ResolvedService};
use crate::settings::Settings;
use crate::status::ServiceMonitor;
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

    let seeds = PoolSeeds::bundled().unwrap_or_else(|error| {
        // Validated by a test, so close to unreachable. Without seeds the pools are
        // whatever this machine has learned, which is the state most titles are in anyway.
        eprintln!("network-monitor: the bundled reference pools could not be loaded: {error}");
        PoolSeeds::empty()
    });
    let store_path = pools_path(&app);
    let mut store = match (&store_path, settings.remember_game_servers) {
        (Some(path), true) => LearnedPools::load(path),
        // Asked not to remember: nothing is read, and anything already written is removed
        // rather than left lying there. A setting that stopped *adding* to a record while
        // keeping the record would not be the promise the wording makes.
        (Some(path), false) => {
            let _ = std::fs::remove_file(path);
            LearnedPools::default()
        }
        (None, _) => LearnedPools::default(),
    };

    loop {
        let end = session(
            &app,
            &settings,
            &mut commands,
            &visible,
            started,
            &mut applications,
            &seeds,
            &mut store,
            store_path.as_deref(),
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
// Many arguments for the same reason it is long: everything the session borrows from the
// process above it arrives here, and bundling them into a struct would only move the list
// somewhere the reader has to go and find it.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn session(
    app: &AppHandle<Wry>,
    settings: &Settings,
    commands: &mut mpsc::Receiver<MonitorCommand>,
    visible: &AtomicBool,
    started: Instant,
    applications: &mut Applications,
    seeds: &PoolSeeds,
    store: &mut LearnedPools,
    store_path: Option<&std::path::Path>,
) -> SessionEnd {
    let interval = settings.baseline_interval();
    let policy = AddressPolicy::default();
    let lifecycle = LifecyclePolicy::default();
    let mut baselines = BaselineMonitor::new(HealthThresholds::default(), health_window(interval));
    let mut services = ServiceMonitor::new(StatusThresholds::default());
    let mut pools = PoolMonitor::new(
        seeds.clone(),
        HealthThresholds::default(),
        settings.remember_game_servers,
    );
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

    match build(
        settings,
        &mut baselines,
        &mut services,
        &mut registry,
        &policy,
    )
    .await
    {
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
    for application in applications.iter() {
        let changes = track_pool(&mut pools, application, store, &mut registry);
        queue(&mut pending, changes, &refusals);
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
                    emit_services(app, &services);
                    emit_apps(
                        app,
                        &apps,
                        &pools,
                        &baselines,
                        applications,
                        &interfaces,
                        discovery.flow_status(),
                    );
                }
                Some(MonitorCommand::MonitorApp(pid)) => {
                    // A fresh snapshot rather than the last periodic one: the user has just
                    // clicked a process the picker listed a moment ago, and a game that
                    // started since the last sweep must not be refused as "not running".
                    if let Some(snapshot) = snapshot_processes().await {
                        if let Some(id) = applications.adopt(pid, &snapshot) {
                            if apps.monitor(id).is_err() {
                                applications.forget(id);
                            } else if let Some(application) =
                                applications.iter().find(|entry| entry.id() == id)
                            {
                                let changes =
                                    track_pool(&mut pools, application, store, &mut registry);
                                queue(&mut pending, changes, &refusals);
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
                        // The pool's own targets go with it: a trickle is only justified
                        // while the user is watching the game it describes.
                        let released = pools.forget(id, &mut registry);
                        queue(&mut pending, released, &refusals);
                    }
                }
                None => return SessionEnd::Shutdown,
            },
            Some(completed) = reports.recv() => {
                let changes = fold_in(
                    &mut baselines,
                    &mut services,
                    &mut pools,
                    &mut apps,
                    &mut registry,
                    &completed,
                );
                queue(&mut pending, changes, &refusals);
            }
            Some(observation) = observations.recv() => {
                observe(&mut apps, &mut pools, applications, observation);
            }
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
                // Wall clock, and only here: a pool entry ages across restarts, which a
                // monotonic clock cannot express. Nothing it decides is a measurement.
                let changes = pools.sweep(&mut registry, SystemTime::now());
                queue(&mut pending, changes, &refusals);
                if pools.take_learned_change() {
                    pools.merge_learned_into(store);
                    persist_pools(store, store_path);
                }
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
                    emit_services(app, &services);
                    emit_apps(
                        app,
                        &apps,
                        &pools,
                        &baselines,
                        applications,
                        &interfaces,
                        discovery.flow_status(),
                    );
                }
            }
        }
    }
}

/// Starts a pool for one application, if it is a title we can look one up for.
///
/// An application with no bundled preset gets no pool, and that is honest rather than
/// lazy: a pool is keyed on an identity that survives a restart, and an application the
/// user grouped by executable name has no such identity to remember it under.
fn track_pool(
    pools: &mut PoolMonitor,
    application: &Application,
    store: &LearnedPools,
    registry: &mut TargetRegistry,
) -> Vec<TargetChange> {
    let Some(preset) = application.preset() else {
        return Vec::new();
    };
    let learned = store.for_preset(preset);
    pools.track(
        application.id(),
        preset,
        &learned,
        registry,
        SystemTime::now(),
    )
}

/// Writes the learned endpoints out, if there is anywhere to write them.
///
/// Failure is reported once and then dropped: everything this file holds is a convenience
/// the running session already has in memory, and refusing to monitor because a disk is
/// full would be a far worse answer than starting from the bundled seeds next time.
fn persist_pools(store: &LearnedPools, path: Option<&std::path::Path>) {
    let Some(path) = path else {
        return;
    };
    if store.is_empty() {
        // Nothing left to remember — every entry expired, or the user asked for none. The
        // file goes rather than being left as an empty shell of what it used to hold.
        let _ = std::fs::remove_file(path);
        return;
    }
    if store.store(path).is_err() {
        eprintln!("network-monitor: the learned game servers could not be written");
    }
}

/// Where the learned-endpoint file lives, when the platform can tell us.
fn pools_path(app: &AppHandle<Wry>) -> Option<std::path::PathBuf> {
    use tauri::Manager as _;
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join(crate::pools::FILE_NAME))
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
fn observe(
    apps: &mut AppMonitor,
    pools: &mut PoolMonitor,
    applications: &Applications,
    sighting: Sighting,
) {
    match sighting {
        Sighting::Endpoint(observation) => {
            let Some(app) = applications.app_of(observation.pid) else {
                return;
            };
            // The pool learns from the same sighting, without a port: what it wants to know
            // later is whether that *machine* answers, and the port the game happened to
            // play over carries nothing we can send.
            pools.observe(
                app,
                nm_core::target::TargetAddress::icmp(observation.endpoint.address.ip()),
                SystemTime::now(),
            );
            let _ = apps.observe(
                app,
                observation.endpoint,
                observation.source,
                observation.flow,
                Instant::now(),
            );
        }
        // No process to map: the event names none, and what let it through was its local
        // port belonging to a connection of a monitored application. It therefore reaches
        // every application using that endpoint, which is the right answer — two
        // applications talking to one address are on the same path to the same server.
        Sighting::Rtt(rtt) => apps.note_passive_rtt(&rtt, Instant::now()),
    }
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
    services: &mut ServiceMonitor,
    pools: &mut PoolMonitor,
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
    services.note_probe_state(
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
            services.record(id, sample);
            pools.record(id, sample);
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

/// Pushes the status page to the window.
///
/// Sent on the ordinary beat even though checks are minutes apart: the page must be filled
/// the moment it is opened, and a card whose only news is that its last check is now a
/// minute older is still news — a status page whose data quietly stopped arriving looks
/// exactly like one reporting that everything is fine.
fn emit_services(app: &AppHandle<Wry>, monitor: &ServiceMonitor) {
    let _ = monitor.snapshot(Instant::now()).emit(app);
}

/// Pushes every monitored application's endpoints to the window.
///
/// Sent even with nothing monitored, because the page still has to say whether flow events
/// are available: without them there are no UDP endpoints and no byte counters anywhere,
/// and an empty list must not be mistaken for an application that is quiet.
#[allow(clippy::too_many_arguments)]
fn emit_apps(
    app: &AppHandle<Wry>,
    apps: &AppMonitor,
    pools: &PoolMonitor,
    baselines: &BaselineMonitor,
    applications: &Applications,
    interfaces: &InterfaceNames,
    flow_status: FlowStatus,
) {
    let now = Instant::now();
    // Read once for the whole emission rather than per application: it is the same answer
    // for all of them, and it is what stops an application being blamed for a network that
    // is failing underneath it.
    let verdicts = baselines.verdicts(now);
    let views = applications
        .iter()
        .map(|application| {
            let reports = apps.endpoints(application.id(), now);
            let pool = pools
                .reading(application.id(), now)
                .map(|report| (report.seeded, report.learned, report.reading));
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
                pool,
                verdicts,
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

/// Builds the probe engine and registers every baseline and status-page target with it.
async fn build(
    settings: &Settings,
    monitor: &mut BaselineMonitor,
    services: &mut ServiceMonitor,
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

    // The status page shares this engine and its rate cap, which is the whole reason it is
    // registered here rather than given a runner of its own: a second token bucket would
    // quietly double the traffic the product promises not to send.
    let list = services::ServiceList::bundled()?;
    for service in &services::resolve_list(&list).await {
        register_service(registry, &mut runner, services, policy, service, now)?;
    }

    Ok(Engine { runner, probers })
}

/// Registers one status-page service with the registry, the runner and the monitor.
///
/// Each endpoint is checked at [`crate::status::CHECK_INTERVAL`] rather than the baselines'
/// cadence: a platform being up changes on the scale of minutes, and this list is probed
/// whether or not the user is doing anything.
fn register_service(
    registry: &mut TargetRegistry,
    runner: &mut ProbeRunner,
    monitor: &mut ServiceMonitor,
    policy: &AddressPolicy,
    service: &ResolvedService,
    now: Instant,
) -> Result<(), Error> {
    let mut handles = Vec::with_capacity(service.endpoints.len());
    let mut tunnelled = Vec::new();

    for endpoint in &service.endpoints {
        // The name never resolved. The endpoint stays on the page, visible and unchecked,
        // because under censorship a lookup that fails is itself the finding.
        let Some(address) = endpoint.address else {
            handles.push(None);
            continue;
        };

        let id = registry.insert(address, TargetTag::StatusService)?;
        if policy.classify(address.ip) == AddressClass::TunnelSentinel {
            tunnelled.push(id);
        }
        // The list's probe-kind hint only reorders the kinds the address class already
        // allows — see `FallbackChain::starting_with`. It is here to save a status page
        // several whole check intervals of silence on a front door that does not answer
        // echoes, not to let a data file choose a figure a tunnel would invent.
        if runner
            .add_preferring(id, address, None, service.probe_kind, now)
            .is_err()
        {
            handles.push(None);
            continue;
        }
        let _ = runner.set_interval(id, crate::status::CHECK_INTERVAL, now);
        handles.push(Some(id));
    }

    monitor.add(service, &handles)?;
    for id in tunnelled {
        monitor.note_tunnelled(id);
    }
    Ok(())
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
