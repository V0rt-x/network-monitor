//! Error type for [`crate`].

use thiserror::Error as ThisError;

/// Failures that prevent the desktop application from starting or running.
#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// The Tauri runtime failed to build or run the application.
    #[error("the application runtime failed: {0}")]
    Runtime(#[from] tauri::Error),
}
