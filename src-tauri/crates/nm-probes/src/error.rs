//! Error type for [`crate`].

use nm_core::address::AddressClass;
use thiserror::Error as ThisError;

/// Failures raised by the probe engine.
///
/// These describe our own inability to measure. A target that stays silent, refuses a
/// connection or is reported unreachable is an outcome, not an error — folding the two
/// together would let a local failure be displayed as someone else's packet loss.
#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// The OS abstraction layer could not carry out a probe.
    #[error(transparent)]
    Platform(#[from] nm_platform::Error),

    /// No available probe kind can honestly measure an address of this class.
    ///
    /// The caller must surface this as "this endpoint cannot be measured", never as a
    /// zero or a loss figure. It is the expected answer for an address a local tunnel
    /// remaps when no end-to-end prober is configured.
    #[error("no available probe kind can honestly measure a {class:?} address")]
    NothingUsable {
        /// What the address was classified as.
        class: AddressClass,
    },
}
