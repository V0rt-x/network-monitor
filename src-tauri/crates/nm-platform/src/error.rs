//! Error type for [`crate`].

use thiserror::Error as ThisError;

/// Failures raised by the OS abstraction layer.
#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// The binary was built for an operating system with no implementation here.
    #[error("this operating system is not supported")]
    UnsupportedPlatform,
}
