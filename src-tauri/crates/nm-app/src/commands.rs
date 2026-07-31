//! Commands the UI may invoke on the Rust core.
//!
//! Commands stay thin: they translate an intent into a call on the core crates and
//! return a serializable DTO. All decision-making lives in `nm-core`/`nm-probes` where
//! it is unit-tested; the frontend contains no business logic.
//!
//! DTOs deliberately carry *enums*, not prose and not i18n key strings. The UI maps
//! each variant to a translation key in an exhaustive `switch`, so a new variant is a
//! TypeScript compile error rather than a missing-string bug at runtime.

use nm_platform::HostPlatform;
use serde::{Deserialize, Serialize};
use specta::Type;

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
