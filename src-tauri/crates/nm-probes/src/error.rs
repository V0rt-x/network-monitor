//! Error type for [`crate`].

use std::io;

use nm_core::address::AddressClass;
use thiserror::Error as ThisError;

use crate::probe::ProbeKind;

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

    /// A probe kind that needs a port was handed a target that has none.
    ///
    /// A configuration mistake rather than a network condition: guessing a port would
    /// measure a service the user never asked about.
    #[error("a {kind:?} probe needs a port and this target has none")]
    PortRequired {
        /// The probe kind that was asked for.
        kind: ProbeKind,
    },

    /// The egress address and the target belong to different address families.
    ///
    /// Binding an IPv4 source to reach an IPv6 host cannot work, and letting the OS pick
    /// instead would silently measure a different route than the one being diagnosed —
    /// which defeats the point of source binding. The addresses are deliberately left out
    /// of the message: they describe the user's own network.
    #[error("the egress address family does not match the target's")]
    SourceFamilyMismatch,

    /// The probe could not be carried out on this machine.
    ///
    /// Distinct from a timeout on purpose: our own socket failing is not the destination
    /// dropping a packet, and must never reach the loss ratio.
    #[error("a {kind:?} probe failed locally: {reason:?}")]
    LocalFailure {
        /// The probe kind that failed.
        kind: ProbeKind,
        /// What the operating system reported.
        reason: io::ErrorKind,
    },

    /// A probe running on the blocking pool never returned its result.
    #[error("the {kind:?} probe task did not complete")]
    ProbeTaskLost {
        /// The probe kind whose task was lost.
        kind: ProbeKind,
    },

    /// The scheduling core rejected a configuration.
    #[error(transparent)]
    Core(#[from] nm_core::Error),

    /// A plan named a probe kind the runner was not given an implementation of.
    ///
    /// A wiring mistake in the application, not a network condition. Reporting it as a
    /// timeout would blame an endpoint for our own missing configuration.
    #[error("no {kind:?} prober is configured")]
    NoProberFor {
        /// The kind that was asked for.
        kind: ProbeKind,
    },

    /// Every probe kind was ruled out but no path walker was configured to take over.
    #[error("no path walker is configured")]
    NoPathWalker,
}
