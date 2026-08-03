//! Route lookup on Windows, via `GetBestRoute2` and `GetIfEntry2`.
//!
//! Two calls and no state. `GetBestRoute2` asks the routing table the same question the
//! stack asks itself before every connection — which adapter would carry a packet to this
//! address — and `GetIfEntry2` says what that adapter is. Neither needs a handle, a
//! privilege or administrator rights.
//!
//! The lookup is done per destination rather than once for the machine, because that is the
//! only form of the question with a true answer. A tunnel client installs a fan of prefixes
//! rather than a default route, so *some* destinations go through it and others do not, and
//! a single "is a VPN on" flag would be wrong for whichever half it did not describe.

use std::mem::{size_of, zeroed};
use std::net::IpAddr;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetBestRoute2, GetIfEntry2, MIB_IF_ROW2, MIB_IPFORWARD_ROW2,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR_INET};

use super::{kind_of_adapter, EgressKind, Route, RouteTable};
use crate::flow::decode_sockaddr;
use crate::Error;

/// Looks up routes through the Windows IP Helper API.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsRouteTable;

impl RouteTable for WindowsRouteTable {
    fn route_to(&self, destination: IpAddr) -> Result<Route, Error> {
        let target = encode(destination);

        // SAFETY: both output structures are fully zeroed and live for the call, the two
        // input pointers address `target` and nothing else, and the null LUID with a zero
        // interface index is the documented way to say "any adapter, you choose". No
        // pointer is retained past the call.
        let (status, route, source) = unsafe {
            let mut route: MIB_IPFORWARD_ROW2 = zeroed();
            let mut source: SOCKADDR_INET = zeroed();
            let status = GetBestRoute2(
                std::ptr::null(),
                0,
                std::ptr::null(),
                &raw const target,
                0,
                &raw mut route,
                &raw mut source,
            );
            (status, route, source)
        };

        if status != ERROR_SUCCESS {
            return Err(Error::Os {
                api: "GetBestRoute2",
                code: status,
            });
        }

        Ok(Route {
            interface_index: route.InterfaceIndex,
            kind: kind_of_interface(route.InterfaceIndex),
            source_address: decode_source(&source),
        })
    }
}

/// Asks what one adapter is.
///
/// Never fails: an adapter that cannot be described is [`EgressKind::Unknown`], which is a
/// real answer here rather than an error. The route itself was found, and refusing the
/// whole lookup because its adapter could not be named would throw away the part that
/// worked.
fn kind_of_interface(index: u32) -> EgressKind {
    // SAFETY: the row is fully zeroed and its `InterfaceIndex` set, which is the documented
    // way to identify the adapter to look up. The call fills the rest of the structure in
    // place and retains no pointer.
    let (status, media_type, if_type) = unsafe {
        let mut row: MIB_IF_ROW2 = zeroed();
        row.InterfaceIndex = index;
        let status = GetIfEntry2(&raw mut row);
        (status, row.MediaType, row.Type)
    };

    if status != ERROR_SUCCESS {
        // An adapter that vanished between the two calls — a tunnel going down is exactly
        // when that happens. Not knowing is a real answer here, and a better one than
        // guessing at the moment the routing is changing under us.
        return EgressKind::Unknown;
    }
    kind_of_adapter(media_type, if_type)
}

/// Reads the local address the stack said it would send from.
fn decode_source(source: &SOCKADDR_INET) -> Option<IpAddr> {
    // SAFETY: `SOCKADDR_INET` is a plain union of socket-address structures with no padding
    // requirements beyond its own alignment, and reading it as its own bytes is exactly
    // what a socket API does with it. The slice borrows `source` and does not outlive it.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(source).cast::<u8>(),
            size_of::<SOCKADDR_INET>(),
        )
    };
    decode_sockaddr(bytes).map(|socket| socket.ip())
}

/// Builds the socket address `GetBestRoute2` wants for a destination.
///
/// Only the family and the address are set: a route is chosen without reference to a port,
/// and the rest of the structure is zero.
fn encode(address: IpAddr) -> SOCKADDR_INET {
    // SAFETY: `SOCKADDR_INET` is a union of plain-old-data structures, so an all-zero value
    // is a valid one; every field that matters is assigned below.
    let mut encoded: SOCKADDR_INET = unsafe { zeroed() };

    // Writing a union field needs no `unsafe`; only reading one back does, and nothing here
    // reads. The family assigned in each arm is what makes that arm the live one.
    match address {
        IpAddr::V4(v4) => {
            encoded.Ipv4.sin_family = AF_INET;
            // Network byte order, which is what the octets already are in this order.
            encoded.Ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes(v4.octets());
        }
        IpAddr::V6(v6) => {
            encoded.Ipv6.sin6_family = AF_INET6;
            encoded.Ipv6.sin6_addr.u.Byte = v6.octets();
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn a_public_address_resolves_to_an_adapter_and_a_source() {
        // `1.1.1.1` is a constant used as an input rather than an observation: nothing about
        // this machine's own network is asserted, only that the lookup answered at all.
        let route = WindowsRouteTable
            .route_to(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)))
            .expect("a machine with a default route can route to a public address");

        // Printed rather than asserted: whether this developer's machine has a tunnel up is
        // not a property of the code, and pinning it either way would make the suite pass
        // or fail on whether a VPN happened to be running.
        eprintln!("route to a public address egresses by: {:?}", route.kind);

        assert!(route.interface_index > 0);
        assert_ne!(route.kind, EgressKind::Unknown);
        assert!(
            route.source_address.is_some(),
            "the stack names the address it would send from"
        );
    }

    #[test]
    fn loopback_routes_by_an_adapter_that_is_not_a_tunnel() {
        // The one destination whose route is the same on every machine, so this asserts the
        // adapter lookup really ran rather than that this developer has no VPN.
        let route = WindowsRouteTable
            .route_to(IpAddr::V4(Ipv4Addr::LOCALHOST))
            .expect("every machine routes to its own loopback");

        assert_eq!(route.kind, EgressKind::Ordinary);
        assert_eq!(
            route.source_address,
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            "loopback sends from itself"
        );
    }

    #[test]
    fn an_ipv6_destination_is_encoded_and_looked_up_as_one() {
        // Encoding the wrong arm of the union would silently look up a garbage address, so
        // the assertion is that the answer is coherent rather than that it succeeded: a
        // machine with no IPv6 route legitimately fails here.
        let route = WindowsRouteTable.route_to(IpAddr::V6(Ipv6Addr::LOCALHOST));
        if let Ok(route) = route {
            assert_eq!(route.source_address, Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        }
    }

    #[test]
    fn a_lookup_is_cheap_enough_to_do_per_probe() {
        // It runs once per endpoint per scheduling decision, at up to the global cap of 32
        // probes a second, so it has to cost approximately nothing. Measured rather than
        // assumed — the ceiling is loose enough for a busy machine and tight enough to
        // catch a call that turned into a table walk.
        let rounds = 200;
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            let _ = WindowsRouteTable.route_to(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        }
        let each = started.elapsed() / rounds;
        eprintln!("route lookup: {each:?}");
        assert!(
            each < std::time::Duration::from_micros(500),
            "a route lookup took {each:?}"
        );
    }
}
