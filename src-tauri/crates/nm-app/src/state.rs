//! Shared application state, held by Tauri and reached from commands.
//!
//! There is no mutex here. The current settings live inside a
//! [`tokio::sync::watch`] channel, which is both the store and the notification the
//! debounced writer waits on — one source of truth instead of a lock plus a copy that can
//! drift from it. Everything else is an atomic or a channel sender.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nm_core::endpoint::AppId;
use nm_platform::process::Pid;
use tokio::sync::{oneshot, watch};

use crate::runtime::{MonitorCommand, MonitorHandle};
use crate::settings::{Settings, SettingsProblem};
use crate::view::ChartHistoryView;

/// Everything a command may need to reach.
#[derive(Debug)]
pub struct AppState {
    settings: watch::Sender<Settings>,
    problem: Option<SettingsProblem>,
    monitor: MonitorHandle,
    visible: Arc<AtomicBool>,
    /// Whether the tray has a menu the user could bring the window back from.
    ///
    /// Until it does, closing the window must really close the application — hiding to a
    /// tray icon with no menu would leave no way to quit but the task manager.
    tray_ready: AtomicBool,
}

impl AppState {
    /// Builds the state around an already-loaded configuration.
    #[must_use]
    pub fn new(
        settings: watch::Sender<Settings>,
        problem: Option<SettingsProblem>,
        monitor: MonitorHandle,
        visible: Arc<AtomicBool>,
    ) -> Self {
        Self {
            settings,
            problem,
            monitor,
            visible,
            tray_ready: AtomicBool::new(false),
        }
    }

    /// The settings in force.
    #[must_use]
    pub fn settings(&self) -> Settings {
        self.settings.borrow().clone()
    }

    /// What went wrong loading them, if anything.
    #[must_use]
    pub const fn problem(&self) -> Option<SettingsProblem> {
        self.problem
    }

    /// Replaces the settings with a sanitized version of `next`, returning what took effect.
    ///
    /// Publishing to the watch channel is what wakes the debounced writer; the monitor is
    /// told separately, and only when something it actually cares about moved — rebuilding
    /// a probing session costs a fresh round of name resolution, so a language change must
    /// not trigger one.
    pub fn update_settings(&self, next: Settings) -> Settings {
        let sane = next.sanitized();
        let previous = self.settings.send_replace(sane.clone());

        if previous.country != sane.country
            || previous.baseline_interval_secs != sane.baseline_interval_secs
            || previous.name_networks != sane.name_networks
        {
            self.monitor
                .send(MonitorCommand::Reconfigure(Box::new(sane.clone())));
        }
        sane
    }

    /// Records whether the window can currently be drawn to.
    ///
    /// Becoming visible also asks the monitor for a snapshot at once, so a window that was
    /// hidden for an hour is not blank while it waits for the next tick.
    pub fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Relaxed);
        if visible {
            self.monitor.send(MonitorCommand::WindowRevealed);
        }
    }

    /// Starts monitoring the application a running process belongs to.
    ///
    /// The process is the seed the application is formed around — its namesakes and its
    /// descendants — not the identity it is followed by afterwards.
    pub fn monitor_app(&self, pid: Pid) {
        self.monitor.send(MonitorCommand::MonitorApp(pid));
    }

    /// Asks the monitor for one application's stored chart history.
    ///
    /// Fetched rather than pushed: the event carries the last forty slots at the emission
    /// rate, and the hour behind them is asked for a handful of times a session. A monitor
    /// that has already stopped answers with an empty history rather than an error, because
    /// there is nothing a reader could do about a core shutting down under them.
    pub async fn chart_history(&self, app: AppId) -> ChartHistoryView {
        let (reply, answer) = oneshot::channel();
        if self
            .monitor
            .request(MonitorCommand::ChartHistory { app, reply })
            .await
            .is_err()
        {
            return ChartHistoryView::default();
        }
        answer.await.unwrap_or_default()
    }

    /// Stops following one application's endpoints.
    pub fn forget_app(&self, app: AppId) {
        self.monitor.send(MonitorCommand::ForgetApp(app));
    }

    /// Records that the tray now has a menu.
    pub fn mark_tray_ready(&self) {
        self.tray_ready.store(true, Ordering::Relaxed);
    }

    /// Whether closing the window may hide it instead of ending the application.
    #[must_use]
    pub fn can_minimize_to_tray(&self) -> bool {
        self.tray_ready.load(Ordering::Relaxed)
    }
}
