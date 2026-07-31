//! Commands the UI may invoke on the Rust core.
//!
//! Commands stay thin: they translate an intent into a call on the core crates and
//! return a serializable DTO. All decision-making lives in `nm-core`/`nm-probes` where
//! it is unit-tested; the frontend contains no business logic.
//!
//! DTOs deliberately carry *enums*, not prose and not i18n key strings. The UI maps
//! each variant to a translation key in an exhaustive `switch`, so a new variant is a
//! TypeScript compile error rather than a missing-string bug at runtime.

// Tauri decides the signature of a command: handles and deserialized payloads arrive by
// value whether or not the body consumes them, so the usual advice to borrow does not
// apply here.
#![allow(clippy::needless_pass_by_value)]

use nm_platform::HostPlatform;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State, Wry};

use crate::settings::{
    Settings, SettingsProblem, MAX_BASELINE_INTERVAL_SECS, MIN_BASELINE_INTERVAL_SECS,
};
use crate::shell::TrayLabels;
use crate::state::AppState;
use crate::{baselines, shell};

/// Platform backend the core will use, as seen across the IPC boundary.
///
/// Mirrors [`HostPlatform`] plus the honest "we have no backend here" case, keeping the
/// OS-layer type out of the IPC contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PlatformKind {
    /// Windows.
    Windows,
    /// Linux.
    Linux,
    /// macOS.
    MacOs,
    /// An operating system `nm-platform` has no implementation for.
    Unsupported,
}

impl From<Result<HostPlatform, nm_platform::Error>> for PlatformKind {
    fn from(platform: Result<HostPlatform, nm_platform::Error>) -> Self {
        match platform {
            Ok(HostPlatform::Windows) => Self::Windows,
            Ok(HostPlatform::Linux) => Self::Linux,
            Ok(HostPlatform::MacOs) => Self::MacOs,
            Err(_) => Self::Unsupported,
        }
    }
}

/// Whether the core can actually monitor on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CoreReadiness {
    /// The core started and has a platform backend.
    Ready,
    /// The core started but there is no backend for this operating system.
    UnsupportedPlatform,
}

/// Static information about the monitoring core, requested by the UI on startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CoreStatus {
    /// Semantic version reported by `nm-core`.
    pub core_version: String,
    /// Platform backend in use.
    pub platform: PlatformKind,
    /// Readiness of the core.
    pub readiness: CoreReadiness,
}

impl CoreStatus {
    /// Builds the DTO from already-resolved inputs.
    ///
    /// Separate from [`core_status`] so the mapping — including the degraded path —
    /// can be exercised on any host OS without a Tauri runtime.
    #[must_use]
    pub fn describe(
        core_version: &str,
        platform: Result<HostPlatform, nm_platform::Error>,
    ) -> Self {
        let platform = PlatformKind::from(platform);
        Self {
            core_version: core_version.to_owned(),
            platform,
            readiness: match platform {
                PlatformKind::Windows | PlatformKind::Linux | PlatformKind::MacOs => {
                    CoreReadiness::Ready
                }
                PlatformKind::Unsupported => CoreReadiness::UnsupportedPlatform,
            },
        }
    }
}

/// Reports what the Rust core is and which platform backend it will use.
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn core_status() -> CoreStatus {
    CoreStatus::describe(nm_core::VERSION, HostPlatform::current())
}

/// The settings in force, together with what the UI needs to offer valid choices.
///
/// The bounds travel with the value rather than being duplicated in TypeScript: a slider
/// whose limits came from a hand-written constant would drift from the ones Rust enforces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// The settings in force.
    pub settings: Settings,
    /// What went wrong loading them, if anything.
    pub problem: Option<SettingsProblem>,
    /// Country codes with a bundled domestic baseline list.
    pub countries: Vec<String>,
    /// Shortest baseline interval the core accepts, in seconds.
    pub min_interval_secs: u32,
    /// Longest baseline interval the core accepts, in seconds.
    pub max_interval_secs: u32,
}

impl SettingsView {
    fn of(settings: Settings, problem: Option<SettingsProblem>) -> Self {
        Self {
            settings,
            problem,
            countries: baselines::countries()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            min_interval_secs: MIN_BASELINE_INTERVAL_SECS,
            max_interval_secs: MAX_BASELINE_INTERVAL_SECS,
        }
    }
}

/// Reports the current settings.
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn get_settings(state: State<'_, AppState>) -> SettingsView {
    SettingsView::of(state.settings(), state.problem())
}

/// Applies new settings and reports what actually took effect.
///
/// The return value is the sanitized result, not an echo of the request: an out-of-range
/// interval or an unknown country comes back corrected, and the autostart flag comes back
/// as the platform reports it. A UI that showed the request rather than the outcome would
/// be claiming something about the machine that may not be true.
#[tauri::command]
#[specta::specta]
pub fn set_settings(
    app: AppHandle<Wry>,
    state: State<'_, AppState>,
    settings: Settings,
) -> SettingsView {
    let mut wanted = settings;
    wanted.autostart = shell::apply_autostart(&app, wanted.autostart);
    let applied = state.update_settings(wanted);
    SettingsView::of(applied, state.problem())
}

/// Gives the tray menu its labels, translated by the UI.
///
/// Returns whether the menu is now in place. It is deliberately not an error type: a tray
/// menu that could not be built is visible as its own absence, and there is nothing the
/// user could do with a message about it — but the UI does need to know, because until it
/// succeeds, closing the window quits instead of minimizing.
#[tauri::command]
#[specta::specta]
pub fn apply_tray_labels(app: AppHandle<Wry>, labels: TrayLabels) -> bool {
    shell::apply_tray_labels(&app, &labels).is_ok()
}

/// Hides the window, leaving the core measuring.
#[tauri::command]
#[specta::specta]
pub fn hide_to_tray(app: AppHandle<Wry>) {
    shell::hide_window(&app);
}

/// Ends the application.
///
/// Exists so the UI can offer a quit that is always reachable, whatever state the tray menu
/// is in.
#[tauri::command]
#[specta::specta]
pub fn quit_app(app: AppHandle<Wry>) {
    app.exit(0);
}
