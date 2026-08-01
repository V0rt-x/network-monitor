//! Error type for [`crate`].

use thiserror::Error as ThisError;

/// Everything that can go wrong inside the pure core.
///
/// Every variant describes a caller mistake caught at construction time, which is why
/// the core can then run without a single panicking operation on its hot paths.
#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// A ring buffer was asked for with no room to store anything.
    #[error("a sample buffer needs a capacity of at least one")]
    ZeroCapacity,

    /// A probe rate cap of zero would stall every target forever.
    #[error("the probe rate cap must be at least one probe per second")]
    ZeroProbeRate,

    /// A zero probe interval would make a target due again the instant it was probed.
    #[error("a probe interval must be greater than zero")]
    ZeroInterval,

    /// A zero flow window would make every passive rate a division by nothing.
    #[error("the flow window must be greater than zero")]
    ZeroFlowWindow,

    /// A CIDR block could not be parsed, or its prefix was wider than its address family.
    #[error("`{raw}` is not a valid CIDR block")]
    InvalidCidr {
        /// The rejected input, kept for diagnostics.
        raw: String,
    },

    /// More applications were monitored at once than the product allows.
    #[error("at most {limit} applications can be monitored at once")]
    TooManyApps {
        /// The cap that was reached.
        limit: u32,
    },

    /// A flow was reported for an application that is not being monitored.
    #[error("application {app} is not being monitored")]
    UnknownApp {
        /// The identifier that was not recognised.
        app: u32,
    },

    /// An active-endpoint cap of zero would leave every endpoint demoted forever.
    #[error("the active-endpoint cap must be at least one")]
    ZeroEndpointCap,

    /// Endpoints would be forgotten before they had even gone idle.
    #[error("endpoints must be retained at least as long as they count as active")]
    RetentionShorterThanActivity,

    /// The registry ran out of identifiers.
    ///
    /// Reaching this needs `u32::MAX` insertions in one session; it exists so the
    /// registry never has to panic or silently reuse an identifier.
    #[error("the target registry has no identifiers left")]
    TargetIdExhausted,
}
