//! The tokio side of general network health: probing the baselines and pushing the result.
//!
//! One task owns everything mutable — the [`BaselineMonitor`], and the channel that keeps
//! the probe engine's own loop alive — so nothing is shared between threads and no lock is
//! ever held across an `.await`. It talks to the rest of the app through
//! [`MonitorCommand`].
//!
//! # Rust drives, the UI renders
//!
//! Sampling is never driven by the UI. The probe engine runs at whatever interval the
//! settings say, and this task pushes a snapshot at [`EMIT_PERIOD`] — well inside the
//! ≤ 4 Hz budget. **While the window is hidden nothing is emitted at all**: the core keeps
//! measuring and the history keeps filling, but a hidden `WebView` is never woken to lay
//! out a chart nobody can see. Showing the window emits at once rather than waiting out
//! the period, so it is never blank.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nm_core::address::{AddressClass, AddressPolicy};
use nm_core::health::HealthThresholds;
use nm_core::sample::ProbeSample;
use nm_core::target::{TargetRegistry, TargetTag};
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

use crate::baselines::{self, BaselineGroup, BaselineTarget};
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
    /// Boxed because it dwarfs the other variant, and an enum is as large as its largest
    /// member.
    Reconfigure(Box<Settings>),
    /// The window has just been shown; push a snapshot without waiting out the period.
    ///
    /// Visibility itself is read from the shared flag this task was given — this command
    /// only exists so a freshly shown window is never blank for a second.
    WindowRevealed,
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
async fn run(
    app: AppHandle<Wry>,
    mut settings: Settings,
    mut commands: mpsc::Receiver<MonitorCommand>,
    visible: Arc<AtomicBool>,
) {
    let started = Instant::now();

    loop {
        match session(&app, &settings, &mut commands, &visible, started).await {
            SessionEnd::Reconfigure(next) => settings = next,
            SessionEnd::Shutdown => return,
        }
    }
}

/// Runs one monitoring session for a fixed set of settings.
async fn session(
    app: &AppHandle<Wry>,
    settings: &Settings,
    commands: &mut mpsc::Receiver<MonitorCommand>,
    visible: &AtomicBool,
    started: Instant,
) -> SessionEnd {
    let interval = settings.baseline_interval();
    let mut monitor = BaselineMonitor::new(HealthThresholds::default(), health_window(interval));

    // Holding this sender for the session's lifetime is what keeps the probe engine's loop
    // alive; dropping it on the way out is what stops it.
    let (_probe_commands, probe_receiver) = mpsc::channel::<ProbeCommand>(COMMAND_QUEUE);
    let (report_sender, mut reports) = mpsc::channel::<Completed>(REPORT_QUEUE);

    match build(settings, &mut monitor).await {
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

    let mut ticker = tokio::time::interval(EMIT_PERIOD);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(MonitorCommand::Reconfigure(next)) => return SessionEnd::Reconfigure(*next),
                Some(MonitorCommand::WindowRevealed) => emit(app, &monitor, started),
                None => return SessionEnd::Shutdown,
            },
            Some(completed) = reports.recv() => fold_in(&mut monitor, &completed),
            _ = ticker.tick() => {
                if visible.load(Ordering::Relaxed) {
                    emit(app, &monitor, started);
                }
            }
        }
    }
}

/// Folds one completed probe into the monitor's state.
///
/// Split out from the loop because it is the one place a measurement becomes history, and
/// what it declines to record matters as much as what it records.
fn fold_in(monitor: &mut BaselineMonitor, completed: &Completed) {
    let id = completed.report.id;
    monitor.note_probe_state(
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
        monitor.record(id, ProbeSample::new(completed.report.at, *outcome));
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
async fn build(settings: &Settings, monitor: &mut BaselineMonitor) -> Result<Engine, Error> {
    let policy = AddressPolicy::default();
    let probers = probers();
    let now = Instant::now();
    let mut runner = ProbeRunner::new(policy.clone(), probers.kinds(), now)?
        .with_intervals(settings.baseline_interval(), MAX_INTERVAL);

    let domestic = baselines::domestic(&settings.country)?;
    let foreign = baselines::foreign()?;
    let mut targets = baselines::resolve_list(BaselineGroup::Domestic, &domestic).await;
    targets.extend(baselines::resolve_list(BaselineGroup::Foreign, &foreign).await);

    let mut registry = TargetRegistry::new();
    for target in &targets {
        register(&mut registry, &mut runner, monitor, &policy, target, now)?;
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
