//! Shared application state, held by Tauri and reached from commands.
//!
//! There is no mutex here. The current settings live inside a
//! [`tokio::sync::watch`] channel, which is both the store and the notification the
//! debounced writer waits on — one source of truth instead of a lock plus a copy that can
//! drift from it. Everything else is an atomic or a channel sender.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::watch;

use crate::runtime::{MonitorCommand, MonitorHandle};
use crate::settings::{Settings, SettingsProblem};

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
