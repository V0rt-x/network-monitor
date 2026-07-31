//! Error type for [`crate`].

use thiserror::Error as ThisError;

/// Failures raised by the OS abstraction layer.
///
/// These describe our *own* inability to carry out a measurement. A target that stays
/// silent or answers "unreachable" is a measurement, not an error, and never appears
/// here — otherwise a local resource failure would be indistinguishable from packet loss.
#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// The binary was built for an operating system with no implementation here.
    #[error("this operating system is not supported")]
    UnsupportedPlatform,

    /// An IPv6 address was given to a probe that only speaks IPv4.
    ///
    /// A known gap rather than a permanent limitation: Windows exposes `Icmp6SendEcho2`
    /// for this, with `sockaddr_in6` endpoints and a differently shaped reply. Failing
    /// loudly is deliberate — reporting an IPv6 host as unreachable would be a lie.
    #[error("IPv6 probing is not implemented yet")]
    Ipv6Unsupported,

    /// The ICMP request could not be carried out; the code is the OS status.
    #[error("the ICMP request failed with status {code}")]
    Icmp {
        /// Windows `IP_STATUS` or system error code.
        code: u32,
    },

    /// An operating-system call failed.
    ///
    /// `api` names the call rather than the operation, because these codes are only
    /// actionable next to the function that produced them: "access denied" means
    /// something different from `OpenProcess` than from `GetExtendedTcpTable`.
    #[error("the {api} call failed with code {code}")]
    Os {
        /// The OS entry point that failed.
        api: &'static str,
        /// The system error code it reported.
        code: u32,
    },
}
