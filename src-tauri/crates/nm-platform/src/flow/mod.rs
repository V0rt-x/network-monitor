//! Per-process flow events: who an application is actually talking to.
//!
//! The connection tables in [`crate::connection`] answer this for TCP and cannot answer it
//! for UDP — a datagram socket has no peer the kernel could report, even after `connect`.
//! Since everything a competitive game plays over is UDP, that gap is the whole reason
//! this module exists.
//!
//! **Discovery, not capture.** A flow event carries the endpoints and a byte count. It
//! never carries payload, and nothing here can read one: the operating system reports that
//! a process exchanged *n* bytes with an address, which is exactly the metadata needed to
//! decide what to probe and nothing more. Measurement remains the job of our own probes.
//!
//! **The events are filtered before they reach us, in three stages**, because this is the
//! one part of the design whose volume is set by the machine rather than by us:
//! the subscription names a narrow keyword, a level that excludes per-packet telemetry,
//! and the exact event numbers wanted — the kernel drops everything else. What survives is
//! then matched against the monitored processes here, so flows belonging to applications
//! the user did not select never enter this program's memory at all. That last stage is
//! data minimisation rather than performance: on a machine whose owner is under
//! surveillance, the app should hold as little of the network's shape as it can.
//!
//! Linux reaches the same information through `sock_diag` with inet-diag byte counters, or
//! eBPF where it is permitted; macOS through `proc_pidfdinfo` on socket descriptors.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

use crate::connection::Protocol;
use crate::process::Pid;
use crate::Error;

#[cfg(windows)]
pub mod windows;

/// Which way the bytes travelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowDirection {
    /// The application sent them.
    Sent,
    /// The application received them.
    Received,
}

/// One observation of an application exchanging data with a remote endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowEvent {
    /// The process the flow belongs to.
    pub pid: Pid,
    /// Transport protocol.
    pub protocol: Protocol,
    /// The local socket the application used.
    ///
    /// This is what a probe must bind to in order to follow the same route as the flow —
    /// the same interface, VPN tunnel or accelerator.
    pub local: SocketAddr,
    /// The endpoint the application was talking to.
    pub remote: SocketAddr,
    /// Which way this observation went.
    pub direction: FlowDirection,
    /// How many bytes this observation accounts for.
    pub bytes: u64,
}

/// Where flow events are delivered.
///
/// Called on the tracing thread, once per event, so an implementation of it must not
/// block: hand the event to a queue and return. It receives a borrow rather than an owned
/// value so that a consumer which only wants the addresses copies nothing.
pub type FlowSink = Box<dyn FnMut(&FlowEvent) + Send + 'static>;

/// A source of per-process flow events.
pub trait FlowEventSource: Send {
    /// Begins delivering events to `sink`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TracingNotPermitted`] when this account may not open a tracing
    /// session — the ordinary case on a machine that has not been set up for it, and the
    /// one the UI must explain rather than report as a fault. Other failures surface as
    /// [`Error::Os`].
    fn start(&mut self, sink: FlowSink) -> Result<(), Error>;

    /// Replaces the set of processes whose flows are reported.
    ///
    /// Takes effect on the next event; may be called before or during a session, so that
    /// the user adding an application does not restart tracing. Anything not named here is
    /// discarded before it becomes a [`FlowEvent`].
    fn watch(&self, pids: &[Pid]);

    /// Whether events are still being delivered.
    ///
    /// A tracing session is not owned by the process that opened it: it is a system object
    /// with a name, and anything that knows the name can stop it. So a source that started
    /// successfully can go quiet at any moment, and a consumer that remembered the answer
    /// from start-up would report a healthy session while discovering nothing.
    ///
    /// The distinction the UI depends on: no UDP endpoints because the application has
    /// none, against no UDP endpoints because nothing is looking.
    fn is_running(&self) -> bool;

    /// Ends the session. Idempotent.
    fn stop(&mut self);
}

/// The host's flow-event source, if this build has one.
///
/// # Errors
///
/// Returns [`Error::UnsupportedPlatform`] where no backend exists yet. Note that a source
/// existing says nothing about whether it may *run*: that is [`FlowEventSource::start`]'s
/// answer, and on Windows it commonly refuses.
pub fn system_flow_source() -> Result<Box<dyn FlowEventSource>, Error> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::EtwFlowSource::new()))
    }
    #[cfg(not(windows))]
    {
        Err(Error::UnsupportedPlatform)
    }
}

/// Address-family values as they appear in a Win32 `SOCKADDR`.
///
/// Declared here rather than only in the Windows module so the decoding below is testable
/// on any host; a `#[cfg(windows)]` test checks them against what `windows-sys` declares.
pub(crate) mod address_family {
    /// IPv4.
    pub const INET: u16 = 2;
    /// IPv6.
    pub const INET6: u16 = 23;
}

/// Smallest `sockaddr_in`: family, port, address.
const SOCKADDR_IN_LEN: usize = 8;
/// Offset of the address inside a `sockaddr_in6`, past family, port and flow info.
const SOCKADDR_IN6_ADDR_OFFSET: usize = 8;
/// Bytes needed to reach the end of a `sockaddr_in6`'s address.
const SOCKADDR_IN6_ADDR_END: usize = SOCKADDR_IN6_ADDR_OFFSET + 16;
/// Full `sockaddr_in6`, including the trailing scope identifier.
const SOCKADDR_IN6_LEN: usize = SOCKADDR_IN6_ADDR_END + 4;

/// Decodes a Win32 `SOCKADDR` blob into a socket address.
///
/// The tracing library hands these fields over as raw bytes, which is welcome: the layout
/// is the part worth testing, and doing it here keeps it out of the platform code.
///
/// Two hazards, both handled. The family is in host order while **the port is in network
/// order** — reading the port the same way as the family yields a byte-swapped port, and
/// that failure is silent, since every endpoint then looks plausible and simply never
/// answers. And a blob may be longer than the structure it holds, so lengths are checked
/// as lower bounds rather than for equality.
///
/// Returns [`None`] for a family this build does not understand rather than guessing —
/// an endpoint we cannot name is one we must not probe.
///
/// An IPv4-mapped result is handed back in its IPv4 form: a dual-stack socket reports every
/// IPv4 peer as `::ffff:a.b.c.d`, and leaving it that way makes the same host unprobeable,
/// unclassifiable and countable twice. See [`crate::address`].
pub(crate) fn decode_sockaddr(bytes: &[u8]) -> Option<SocketAddr> {
    decode_raw_sockaddr(bytes).map(crate::address::unmap_socket)
}

/// The decoding itself, before the address family is canonicalised.
fn decode_raw_sockaddr(bytes: &[u8]) -> Option<SocketAddr> {
    if bytes.len() < 4 {
        return None;
    }
    let family = u16::from_le_bytes([bytes[0], bytes[1]]);
    let port = u16::from_be_bytes([bytes[2], bytes[3]]);

    match family {
        address_family::INET if bytes.len() >= SOCKADDR_IN_LEN => Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7])),
            port,
        )),
        address_family::INET6 if bytes.len() >= SOCKADDR_IN6_ADDR_END => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&bytes[SOCKADDR_IN6_ADDR_OFFSET..SOCKADDR_IN6_ADDR_END]);
            // The scope identifier matters for link-local addresses, which are ambiguous
            // without it. A blob that stops short of it is still usable, just unscoped.
            let scope_id = if bytes.len() >= SOCKADDR_IN6_LEN {
                u32::from_le_bytes([
                    bytes[SOCKADDR_IN6_ADDR_END],
                    bytes[SOCKADDR_IN6_ADDR_END + 1],
                    bytes[SOCKADDR_IN6_ADDR_END + 2],
                    bytes[SOCKADDR_IN6_ADDR_END + 3],
                ])
            } else {
                0
            };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(octets),
                port,
                0,
                scope_id,
            )))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sockaddr_in(port: u16, octets: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&address_family::INET.to_le_bytes());
        bytes.extend_from_slice(&port.to_be_bytes());
        bytes.extend_from_slice(&octets);
        bytes
    }

    fn sockaddr_in6(port: u16, address: Ipv6Addr, scope_id: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&address_family::INET6.to_le_bytes());
        bytes.extend_from_slice(&port.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flow info
        bytes.extend_from_slice(&address.octets());
        bytes.extend_from_slice(&scope_id.to_le_bytes());
        bytes
    }

    #[test]
    fn decodes_an_ipv4_endpoint() {
        let decoded = decode_sockaddr(&sockaddr_in(27_015, [203, 0, 113, 5]));
        assert_eq!(
            decoded,
            Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)),
                27_015
            ))
        );
    }

    #[test]
    fn reads_the_port_in_network_order() {
        // The recurring hazard: 27015 is 0x6987, so a host-order read would give 0x8769
        // (34665). Every probe would then go to a plausible port that never answers.
        let blob = sockaddr_in(27_015, [203, 0, 113, 5]);
        assert_eq!(blob[2], 0x69, "the port must be stored big-endian");
        assert_eq!(blob[3], 0x87);
        assert_eq!(decode_sockaddr(&blob).map(|a| a.port()), Some(27_015));
    }

    #[test]
    fn a_dual_stack_socket_reports_its_ipv4_peer_as_ipv4() {
        // Windows hands every IPv4 peer of a dual-stack socket over as `::ffff:a.b.c.d`.
        // Left in that form the endpoint is refused by the ICMP backend as IPv6, escapes
        // classification against the IPv4 ranges, and is counted separately from the same
        // host seen in the connection table.
        let mapped = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xcb00, 0x7105);
        assert_eq!(
            decode_sockaddr(&sockaddr_in6(27_015, mapped, 0)),
            Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)),
                27_015
            ))
        );
    }

    #[test]
    fn an_unbound_dual_stack_socket_is_recognisably_unspecified() {
        // `::ffff:0.0.0.0` is not `is_unspecified`, so a caller looking for "this socket
        // named no address" would bind a probe to a wildcard in disguise.
        let mapped = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0, 0);
        let decoded = decode_sockaddr(&sockaddr_in6(0, mapped, 0)).expect("it must decode");
        assert!(decoded.ip().is_unspecified());
    }

    #[test]
    fn decodes_an_ipv6_endpoint_with_its_scope() {
        let address = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        let decoded = decode_sockaddr(&sockaddr_in6(443, address, 7));
        assert_eq!(
            decoded,
            Some(SocketAddr::V6(SocketAddrV6::new(address, 443, 0, 7)))
        );
    }

    #[test]
    fn an_ipv6_blob_without_a_scope_is_still_usable() {
        let address = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        let mut blob = sockaddr_in6(443, address, 9);
        blob.truncate(SOCKADDR_IN6_ADDR_END);
        assert_eq!(
            decode_sockaddr(&blob),
            Some(SocketAddr::V6(SocketAddrV6::new(address, 443, 0, 0)))
        );
    }

    #[test]
    fn a_longer_blob_than_the_structure_still_decodes() {
        // The field is a fixed-size buffer in the event, so trailing bytes are normal.
        let mut blob = sockaddr_in(443, [203, 0, 113, 5]);
        blob.extend_from_slice(&[0xAA; 20]);
        assert_eq!(decode_sockaddr(&blob).map(|a| a.port()), Some(443));
    }

    #[test]
    fn a_truncated_blob_is_rejected_rather_than_guessed() {
        assert_eq!(decode_sockaddr(&[]), None);
        assert_eq!(decode_sockaddr(&[2, 0]), None);
        // Family and port present, address missing.
        assert_eq!(decode_sockaddr(&[2, 0, 0x01, 0xBB]), None);
        // An IPv6 family with only an IPv4-sized body.
        assert_eq!(decode_sockaddr(&[23, 0, 0x01, 0xBB, 1, 2, 3, 4]), None);
    }

    #[test]
    fn an_unknown_family_yields_nothing() {
        // Windows uses AF_UNIX, AF_BTH and others on the same structure. An endpoint we
        // cannot name is one we must not probe.
        for family in [0u16, 1, 17, 32, 0xFFFF] {
            let mut blob = vec![0u8; 32];
            blob[0..2].copy_from_slice(&family.to_le_bytes());
            assert_eq!(decode_sockaddr(&blob), None, "family {family}");
        }
    }

    #[test]
    fn decodes_the_unspecified_address() {
        // A send that has not yet bound reports the wildcard; it must decode rather than
        // fail, so the caller can recognise and skip it.
        let decoded = decode_sockaddr(&sockaddr_in(0, [0, 0, 0, 0]));
        assert_eq!(
            decoded,
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))
        );
    }

    /// The transcribed family constants, checked against the real headers.
    #[cfg(windows)]
    #[test]
    fn address_family_constants_match_the_windows_headers() {
        use windows_sys::Win32::Networking::WinSock as sys;

        assert_eq!(address_family::INET, sys::AF_INET);
        assert_eq!(address_family::INET6, sys::AF_INET6);
    }
}
