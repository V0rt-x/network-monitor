//! ICMP echo probing.
//!
//! The trait is deliberately **blocking**. Windows' `IcmpSendEcho2Ex` has an
//! asynchronous form driven by event handles or APCs, but binding those to an async
//! runtime buys nothing here: the probe budget is at most 32 echoes per second, so
//! running each on a blocking thread costs a handful of mostly-idle threads. `nm-probes`
//! owns that decision and never calls an implementation from an async context directly.
//!
//! Everything in this module except [`windows`] is platform-free and unit-tested on any
//! host: the `IP_STATUS` classification and the `IPAddr` byte-order conversions are the
//! parts most likely to be subtly wrong, so they are kept out of the `unsafe` code that
//! only Windows can run.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use crate::Error;

#[cfg(windows)]
pub mod windows;

/// Payload size a plain `ping` uses, and a reasonable default here.
pub const DEFAULT_PAYLOAD_LEN: u16 = 32;

/// One ICMP echo request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoRequest {
    /// Address to probe.
    pub target: IpAddr,
    /// Local address the request must egress from.
    ///
    /// This is what makes a probe follow the same route as the application flow it is
    /// measuring: bind to the address the app's socket uses and the probe traverses the
    /// same interface, VPN tunnel or accelerator. [`None`] lets the OS choose.
    pub source: Option<IpAddr>,
    /// Time-to-live to set on the request.
    ///
    /// A deliberately small value makes this a *path* probe: a router that discards the
    /// packet answers with "TTL expired", revealing that hop and its round-trip time.
    /// [`None`] uses the system default.
    pub ttl: Option<u8>,
    /// Payload size in bytes.
    pub payload_len: u16,
    /// How long to wait for a reply.
    pub timeout: Duration,
}

impl EchoRequest {
    /// A plain echo to `target` with default payload and no routing constraints.
    #[must_use]
    pub const fn to(target: IpAddr, timeout: Duration) -> Self {
        Self {
            target,
            source: None,
            ttl: None,
            payload_len: DEFAULT_PAYLOAD_LEN,
            timeout,
        }
    }

    /// Same request, pinned to a local egress address.
    #[must_use]
    pub const fn from_source(mut self, source: IpAddr) -> Self {
        self.source = Some(source);
        self
    }

    /// Same request, limited to `ttl` hops.
    #[must_use]
    pub const fn with_ttl(mut self, ttl: u8) -> Self {
        self.ttl = Some(ttl);
        self
    }
}

/// What an echo request produced.
///
/// `from` is optional because Windows reports some outcomes through `GetLastError`
/// rather than through a reply structure, and in that form the responding address is not
/// available. Saying "a hop timed out the packet, we do not know which" is honest;
/// inventing an address would not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoOutcome {
    /// The target itself replied.
    Replied {
        /// Address that answered.
        from: IpAddr,
        /// Measured round-trip time.
        rtt: Duration,
    },
    /// A router on the path discarded the packet because its TTL ran out.
    ///
    /// For a path probe this is the *successful* case: it identifies a hop.
    TtlExpired {
        /// The hop that answered, when it is known.
        from: Option<IpAddr>,
        /// Round-trip time to that hop.
        rtt: Duration,
    },
    /// Something on the path reported the destination cannot be reached.
    Unreachable {
        /// The router that answered, when it is known.
        from: Option<IpAddr>,
    },
    /// Nothing answered before the deadline.
    TimedOut,
}

/// Sends ICMP echo requests.
///
/// Implementations block the calling thread until a reply arrives or the timeout
/// elapses.
pub trait IcmpProber: Send + Sync {
    /// Sends one echo request and waits for its outcome.
    ///
    /// # Errors
    ///
    /// Returns an error only when the request could not be carried out — a bad handle,
    /// an unsupported address family, a local resource failure. A target that does not
    /// answer is [`EchoOutcome::TimedOut`], not an error: silence is a measurement.
    fn echo(&self, request: &EchoRequest) -> Result<EchoOutcome, Error>;
}

impl<P: IcmpProber + ?Sized> IcmpProber for Box<P> {
    fn echo(&self, request: &EchoRequest) -> Result<EchoOutcome, Error> {
        (**self).echo(request)
    }
}

/// The host's ICMP implementation, if this build has one.
///
/// The seam that keeps `#[cfg]` out of every crate above: callers ask for the platform's
/// prober and handle the honest absence, rather than branching on the operating system
/// themselves. On a platform with no implementation yet this returns
/// [`Error::UnsupportedPlatform`], and the probe engine simply runs without ICMP —
/// degraded to its connecting probe kinds rather than broken.
///
/// # Errors
///
/// Returns [`Error::UnsupportedPlatform`] where no ICMP backend exists, or whatever the
/// platform backend reports if it cannot be created.
pub fn system_prober() -> Result<Box<dyn IcmpProber>, Error> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsIcmpProber))
    }
    #[cfg(not(windows))]
    {
        Err(Error::UnsupportedPlatform)
    }
}

/// `IP_STATUS` values from the Windows IP Helper API (`ipexport.h`).
///
/// Declared here rather than only in the Windows module so the classification below can
/// be tested on any host. A `#[cfg(windows)]` test asserts every one of these matches
/// the value `windows-sys` exposes, so the table cannot drift from the real headers.
pub mod ip_status {
    /// The echo reply arrived.
    pub const SUCCESS: u32 = 0;
    /// The reply buffer was too small.
    pub const BUF_TOO_SMALL: u32 = 11_001;
    /// No route to the destination network.
    pub const DEST_NET_UNREACHABLE: u32 = 11_002;
    /// The destination host did not respond to address resolution.
    pub const DEST_HOST_UNREACHABLE: u32 = 11_003;
    /// The destination does not speak the requested protocol.
    pub const DEST_PROT_UNREACHABLE: u32 = 11_004;
    /// The destination port is closed.
    pub const DEST_PORT_UNREACHABLE: u32 = 11_005;
    /// The local stack ran out of resources.
    pub const NO_RESOURCES: u32 = 11_006;
    /// An IP option was malformed.
    pub const BAD_OPTION: u32 = 11_007;
    /// A hardware error occurred.
    pub const HW_ERROR: u32 = 11_008;
    /// The packet exceeded the path MTU and could not be fragmented.
    pub const PACKET_TOO_BIG: u32 = 11_009;
    /// No reply arrived before the timeout.
    pub const REQ_TIMED_OUT: u32 = 11_010;
    /// The request itself was malformed.
    pub const BAD_REQ: u32 = 11_011;
    /// No route to the destination.
    pub const BAD_ROUTE: u32 = 11_012;
    /// A router discarded the packet because its TTL reached zero.
    pub const TTL_EXPIRED_TRANSIT: u32 = 11_013;
    /// Fragment reassembly timed out.
    pub const TTL_EXPIRED_REASSEM: u32 = 11_014;
    /// A router reported a problem with a header field.
    pub const PARAM_PROBLEM: u32 = 11_015;
    /// A router asked the sender to slow down.
    pub const SOURCE_QUENCH: u32 = 11_016;
    /// The IP options were too large.
    pub const OPTION_TOO_BIG: u32 = 11_017;
    /// The destination address was rejected as invalid.
    pub const BAD_DESTINATION: u32 = 11_018;
    /// An unspecified failure.
    pub const GENERAL_FAILURE: u32 = 11_050;
}

/// How an `IP_STATUS` maps onto a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusKind {
    /// The target answered.
    Replied,
    /// A hop answered that the TTL expired.
    TtlExpired,
    /// The destination was reported unreachable.
    Unreachable,
    /// Nothing answered in time.
    TimedOut,
    /// The attempt failed locally and measured nothing about the target.
    Unusable,
}

/// Classifies an `IP_STATUS` value.
///
/// The distinction that matters is between statuses that say something about the *path*
/// and statuses that say something about *our own machine*. A local resource failure or
/// a malformed request is not evidence that a game server is down, so it becomes
/// [`StatusKind::Unusable`] — an error — rather than a measurement.
pub(crate) const fn classify_status(status: u32) -> StatusKind {
    match status {
        ip_status::SUCCESS => StatusKind::Replied,
        ip_status::TTL_EXPIRED_TRANSIT | ip_status::TTL_EXPIRED_REASSEM => StatusKind::TtlExpired,
        ip_status::DEST_NET_UNREACHABLE
        | ip_status::DEST_HOST_UNREACHABLE
        | ip_status::DEST_PROT_UNREACHABLE
        | ip_status::DEST_PORT_UNREACHABLE
        | ip_status::BAD_ROUTE
        | ip_status::BAD_DESTINATION => StatusKind::Unreachable,
        ip_status::REQ_TIMED_OUT => StatusKind::TimedOut,
        _ => StatusKind::Unusable,
    }
}

/// Packs an IPv4 address into the IP Helper API's `IPAddr`.
///
/// `IPAddr` holds the four octets in network order inside a machine word, so the
/// conversion is a little-endian read of the octets — not [`Ipv4Addr::to_bits`], which
/// yields host order and would silently probe a reversed address.
pub(crate) const fn to_in_addr(ip: Ipv4Addr) -> u32 {
    u32::from_le_bytes(ip.octets())
}

/// Unpacks an `IPAddr` returned by the IP Helper API.
pub(crate) const fn from_in_addr(raw: u32) -> Ipv4Addr {
    Ipv4Addr::from_bits(u32::from_be_bytes(raw.to_le_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_a_reply_as_a_measurement() {
        assert_eq!(classify_status(ip_status::SUCCESS), StatusKind::Replied);
    }

    #[test]
    fn classifies_both_ttl_expiries_as_hop_responses() {
        assert_eq!(
            classify_status(ip_status::TTL_EXPIRED_TRANSIT),
            StatusKind::TtlExpired
        );
        assert_eq!(
            classify_status(ip_status::TTL_EXPIRED_REASSEM),
            StatusKind::TtlExpired
        );
    }

    #[test]
    fn classifies_every_unreachable_flavour_together() {
        for status in [
            ip_status::DEST_NET_UNREACHABLE,
            ip_status::DEST_HOST_UNREACHABLE,
            ip_status::DEST_PROT_UNREACHABLE,
            ip_status::DEST_PORT_UNREACHABLE,
            ip_status::BAD_ROUTE,
            ip_status::BAD_DESTINATION,
        ] {
            assert_eq!(classify_status(status), StatusKind::Unreachable, "{status}");
        }
    }

    #[test]
    fn classifies_a_timeout_as_silence_rather_than_an_error() {
        assert_eq!(
            classify_status(ip_status::REQ_TIMED_OUT),
            StatusKind::TimedOut
        );
    }

    #[test]
    fn local_failures_measure_nothing_about_the_target() {
        // None of these say anything about the remote host, so they must not become a
        // timeout — that would report packet loss caused by our own machine.
        for status in [
            ip_status::BUF_TOO_SMALL,
            ip_status::NO_RESOURCES,
            ip_status::BAD_OPTION,
            ip_status::HW_ERROR,
            ip_status::PACKET_TOO_BIG,
            ip_status::BAD_REQ,
            ip_status::PARAM_PROBLEM,
            ip_status::SOURCE_QUENCH,
            ip_status::OPTION_TOO_BIG,
            ip_status::GENERAL_FAILURE,
            0xDEAD_BEEF,
        ] {
            assert_eq!(classify_status(status), StatusKind::Unusable, "{status}");
        }
    }

    #[test]
    fn packs_addresses_in_network_order() {
        // 127.0.0.1 is 0x7F000001 in host order and 0x0100007F as an `IPAddr`.
        assert_eq!(to_in_addr(Ipv4Addr::LOCALHOST), 0x0100_007F);
        assert_eq!(to_in_addr(Ipv4Addr::new(1, 2, 3, 4)), 0x0403_0201);
        assert_eq!(to_in_addr(Ipv4Addr::UNSPECIFIED), 0);
    }

    #[test]
    fn address_packing_round_trips() {
        for address in [
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::BROADCAST,
            Ipv4Addr::new(203, 0, 113, 7),
            Ipv4Addr::new(8, 8, 8, 8),
        ] {
            assert_eq!(from_in_addr(to_in_addr(address)), address);
        }
    }

    #[test]
    fn a_request_carries_its_routing_constraints() {
        let target = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
        let source = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));

        let plain = EchoRequest::to(target, Duration::from_secs(1));
        assert_eq!(plain.source, None);
        assert_eq!(plain.ttl, None);
        assert_eq!(plain.payload_len, DEFAULT_PAYLOAD_LEN);

        let routed = plain.clone().from_source(source).with_ttl(3);
        assert_eq!(routed.source, Some(source));
        assert_eq!(routed.ttl, Some(3));
        assert_eq!(routed.target, plain.target);
    }

    /// The constants above are transcribed from `ipexport.h`; this proves the
    /// transcription against what `windows-sys` actually declares, so a typo cannot turn
    /// into a misclassified probe.
    #[cfg(windows)]
    #[test]
    fn ip_status_constants_match_the_windows_headers() {
        use windows_sys::Win32::NetworkManagement::IpHelper as sys;

        assert_eq!(ip_status::SUCCESS, sys::IP_SUCCESS);
        assert_eq!(ip_status::BUF_TOO_SMALL, sys::IP_BUF_TOO_SMALL);
        assert_eq!(
            ip_status::DEST_NET_UNREACHABLE,
            sys::IP_DEST_NET_UNREACHABLE
        );
        assert_eq!(
            ip_status::DEST_HOST_UNREACHABLE,
            sys::IP_DEST_HOST_UNREACHABLE
        );
        assert_eq!(
            ip_status::DEST_PROT_UNREACHABLE,
            sys::IP_DEST_PROT_UNREACHABLE
        );
        assert_eq!(
            ip_status::DEST_PORT_UNREACHABLE,
            sys::IP_DEST_PORT_UNREACHABLE
        );
        assert_eq!(ip_status::NO_RESOURCES, sys::IP_NO_RESOURCES);
        assert_eq!(ip_status::BAD_OPTION, sys::IP_BAD_OPTION);
        assert_eq!(ip_status::HW_ERROR, sys::IP_HW_ERROR);
        assert_eq!(ip_status::PACKET_TOO_BIG, sys::IP_PACKET_TOO_BIG);
        assert_eq!(ip_status::REQ_TIMED_OUT, sys::IP_REQ_TIMED_OUT);
        assert_eq!(ip_status::BAD_REQ, sys::IP_BAD_REQ);
        assert_eq!(ip_status::BAD_ROUTE, sys::IP_BAD_ROUTE);
        assert_eq!(ip_status::TTL_EXPIRED_TRANSIT, sys::IP_TTL_EXPIRED_TRANSIT);
        assert_eq!(ip_status::TTL_EXPIRED_REASSEM, sys::IP_TTL_EXPIRED_REASSEM);
        assert_eq!(ip_status::PARAM_PROBLEM, sys::IP_PARAM_PROBLEM);
        assert_eq!(ip_status::SOURCE_QUENCH, sys::IP_SOURCE_QUENCH);
        assert_eq!(ip_status::OPTION_TOO_BIG, sys::IP_OPTION_TOO_BIG);
        assert_eq!(ip_status::BAD_DESTINATION, sys::IP_BAD_DESTINATION);
        assert_eq!(ip_status::GENERAL_FAILURE, sys::IP_GENERAL_FAILURE);
    }
}
