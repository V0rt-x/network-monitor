//! Error type for [`crate`].

use thiserror::Error as ThisError;

/// Failures that prevent the desktop application from starting or running.
#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// The Tauri runtime failed to build or run the application.
    #[error("the application runtime failed: {0}")]
    Runtime(#[from] tauri::Error),

    /// A bundled or user-supplied target list is not usable.
    #[error("target list {list:?} is unusable: {reason}")]
    TargetList {
        /// Which list failed to load.
        list: String,
        /// What is wrong with it.
        reason: String,
    },

    /// No domestic baseline list is bundled for this country code.
    #[error("no baseline list is bundled for country {country:?}")]
    UnknownCountry {
        /// The code that was asked for.
        country: String,
    },

    /// The probe engine could not be started or configured.
    #[error("the probe engine failed: {0}")]
    Probes(#[from] nm_probes::Error),

    /// The monitoring core rejected a configuration.
    #[error(transparent)]
    Core(#[from] nm_core::Error),
}
