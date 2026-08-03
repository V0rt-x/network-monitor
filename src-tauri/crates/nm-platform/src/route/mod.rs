//! Which adapter the packets to an address would actually leave by.
//!
//! [`crate::interface`] answers the neighbouring question — which adapter owns a *local
//! address* — and that is enough while every probe is bound to the address of the flow it
//! is diagnosing. It is not enough for the two thirds of this product that probe nothing of
//! the sort: a status-page check and a baseline have no application flow to borrow an
//! address from, so the operating system picks the route and nobody asks it what it picked.
//!
//! Asking turns out to matter a great deal. A TUN client — sing-box, and every other client
//! of that shape — does not announce itself in the addresses a user sees. It takes traffic
//! by installing routes: not a default route, which would be conspicuous, but a fan of
//! prefixes (`0.0.0.0/5`, `8.0.0.0/7`, … `240.0.0.0/5`) that covers the whole public
//! internet at a better metric while the real default route stays where it was. Every name
//! still resolves to the real public address of the real service. Nothing about the address
//! is unusual, and everything about the route is.
//!
//! The consequence is the one the product exists to prevent: the tunnel's own stack
//! completes a TCP handshake and answers an echo request without a packet leaving the
//! machine, so both report the tunnel's latency wearing the service's name. Measured on a
//! developer machine running one, the whole status page read between one and two
//! milliseconds — every storefront, every game platform, worldwide — and was green.
//!
//! # Read-only, unprivileged and cheap
//!
//! One call per lookup, no handle, no configuration, no administrator rights: it is the
//! same query the routing table answers for every connection the machine makes anyway.
//! Linux reads it from netlink (`RTM_GETROUTE` with the destination, then `RTM_GETLINK`
//! for the resulting interface's type), macOS from the `PF_ROUTE` socket or the
//! `NET_RT_DUMP` sysctl.

use std::net::IpAddr;

use crate::Error;

#[cfg(windows)]
pub mod windows;

/// What kind of adapter a route leaves the machine by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EgressKind {
    /// An ordinary adapter carrying frames to a real network.
    Ordinary,
    /// An adapter belonging to a tunnel running on this machine.
    LocalTunnel,
    /// The adapter could not be identified.
    #[default]
    Unknown,
}

/// The route to one destination, as far as deciding what may be measured needs it.
///
/// Carries no next hop and no prefix: those describe a real person's network, and neither
/// is needed to answer the one question asked here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// Index of the adapter the packets would leave by.
    pub interface_index: u32,
    /// What that adapter is.
    pub kind: EgressKind,
    /// The local address the operating system would send from.
    ///
    /// The same address [`crate::interface::InterfaceNames`] maps to an adapter name, so an
    /// endpoint nothing bound a probe for can still be labelled with the adapter it uses.
    pub source_address: Option<IpAddr>,
}

/// Looks up the route to a destination.
///
/// [`std::fmt::Debug`] is required because the probe engine holds one and derives `Debug`
/// for its own state; an implementation has nothing to print but its own name.
pub trait RouteTable: Send + Sync + std::fmt::Debug {
    /// Asks the operating system which adapter it would send to `destination` by.
    ///
    /// # Errors
    ///
    /// Returns an error when the lookup fails — including when the destination is
    /// unreachable, which is a failure of the *lookup* rather than a measurement and must
    /// not be confused with one.
    fn route_to(&self, destination: IpAddr) -> Result<Route, Error>;
}

impl<T: RouteTable + ?Sized> RouteTable for Box<T> {
    fn route_to(&self, destination: IpAddr) -> Result<Route, Error> {
        (**self).route_to(destination)
    }
}

/// The host's route table, if this build has one.
///
/// # Errors
///
/// Returns [`Error::UnsupportedPlatform`] where no backend exists yet.
pub fn system_table() -> Result<Box<dyn RouteTable>, Error> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsRouteTable))
    }
    #[cfg(not(windows))]
    {
        Err(Error::UnsupportedPlatform)
    }
}

/// `NdisMediumTunnel` — an adapter Windows itself calls a tunnel.
///
/// Written out rather than imported so the rule below stays compilable and testable on
/// every platform; a Windows-only test asserts it against the header constant, so the
/// value cannot rot silently.
const NDIS_MEDIUM_TUNNEL: i32 = 15;

/// `NdisMediumIP` — an adapter that carries bare IP rather than frames.
///
/// This is what a TUN device is, and what every userland tunnel client of the current
/// generation presents as: `Wintun`, `WireGuard`'s own driver, `OpenVPN`'s data-channel
/// offload.
const NDIS_MEDIUM_IP: i32 = 19;

/// `IF_TYPE_TUNNEL` from the IANA interface-type registry.
const IF_TYPE_TUNNEL: u32 = 131;

/// Whether an adapter of this media type and interface type belongs to a local tunnel.
///
/// Kept platform-free and fed plain numbers so the decision — the part that can be wrong —
/// is tested on any development machine, while the Windows module does nothing but read
/// the two fields and hand them over.
///
/// # What this deliberately does not try to do
///
/// It identifies adapters that carry IP rather than frames, which is every TUN-shaped
/// client. It does **not** recognise a layer-2 TAP adapter, which presents as ordinary
/// Ethernet and is indistinguishable here from a real network card. Chasing those with a
/// list of driver names would be a list that rots, and it is unnecessary: an unrecognised
/// tunnel is caught at measurement time instead, by `nm_core::forgery`, which reads the
/// hop limit of the reply and needs to know nothing about adapters at all. This is the
/// cheap answer, not the only one.
#[must_use]
pub const fn kind_of_adapter(media_type: i32, if_type: u32) -> EgressKind {
    if media_type == NDIS_MEDIUM_TUNNEL || media_type == NDIS_MEDIUM_IP {
        return EgressKind::LocalTunnel;
    }
    if if_type == IF_TYPE_TUNNEL {
        return EgressKind::LocalTunnel;
    }
    EgressKind::Ordinary
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `NdisMedium802_3`: ethernet, which every physical adapter and every bridge reports.
    const ETHERNET: i32 = 0;
    /// `IF_TYPE_ETHERNET_CSMACD`.
    const IF_TYPE_ETHERNET: u32 = 6;
    /// `IF_TYPE_PROPVIRTUAL`, which is what a Wintun adapter reports.
    const IF_TYPE_PROPVIRTUAL: u32 = 53;

    #[test]
    fn an_adapter_carrying_bare_ip_is_a_tunnel() {
        // The adapter that motivated all of this: a Wintun device, interface type 53,
        // media type IP. Read as ordinary, it takes the whole status page with it.
        assert_eq!(
            kind_of_adapter(NDIS_MEDIUM_IP, IF_TYPE_PROPVIRTUAL),
            EgressKind::LocalTunnel
        );
    }

    #[test]
    fn an_adapter_windows_itself_calls_a_tunnel_is_one() {
        // Teredo, 6to4 and IP-HTTPS all present this way.
        assert_eq!(
            kind_of_adapter(NDIS_MEDIUM_TUNNEL, IF_TYPE_TUNNEL),
            EgressKind::LocalTunnel
        );
    }

    #[test]
    fn either_field_alone_is_enough() {
        assert_eq!(
            kind_of_adapter(ETHERNET, IF_TYPE_TUNNEL),
            EgressKind::LocalTunnel
        );
        assert_eq!(
            kind_of_adapter(NDIS_MEDIUM_IP, IF_TYPE_ETHERNET),
            EgressKind::LocalTunnel
        );
    }

    #[test]
    fn an_ordinary_adapter_is_left_alone() {
        // The cost of a false positive here is every probe on the machine moving to the
        // expensive kind, so ethernet must stay ethernet — including the virtual ethernet
        // a hypervisor's switch presents, which is a bridge to a real network and not a
        // tunnel.
        assert_eq!(
            kind_of_adapter(ETHERNET, IF_TYPE_ETHERNET),
            EgressKind::Ordinary
        );
        assert_eq!(
            kind_of_adapter(ETHERNET, IF_TYPE_PROPVIRTUAL),
            EgressKind::Ordinary,
            "a Hyper-V switch is interface type 6, but a virtual type alone must not decide"
        );
    }

    #[test]
    fn an_unknown_pairing_is_read_as_ordinary_rather_than_refused() {
        // Wi-Fi, loopback, a modem: none of them are tunnels, and none of them should
        // silently lose the cheap probe kinds because this rule had not met them.
        for media in [1_i32, 9, 16, 17] {
            assert_eq!(kind_of_adapter(media, 71), EgressKind::Ordinary, "{media}");
        }
    }

    #[test]
    fn nothing_known_is_the_default() {
        // A route that could not be looked up must not read as either answer.
        assert_eq!(EgressKind::default(), EgressKind::Unknown);
    }

    #[cfg(windows)]
    #[test]
    fn the_written_out_constants_match_the_windows_headers() {
        // The rule above is fed plain numbers so it can be tested anywhere. That is only
        // safe while the numbers are right, and a header constant is the one thing that can
        // change under us without a compiler error.
        use windows_sys::Win32::NetworkManagement::Ndis::{NdisMediumIP, NdisMediumTunnel};

        assert_eq!(NDIS_MEDIUM_TUNNEL, NdisMediumTunnel);
        assert_eq!(NDIS_MEDIUM_IP, NdisMediumIP);
    }
}
