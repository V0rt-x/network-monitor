//! One form per address, decided at the operating-system boundary.
//!
//! A dual-stack socket reports an IPv4 peer as an **IPv4-mapped IPv6 address** —
//! `::ffff:a.b.c.d`. Windows does this for every flow of a process that opened a
//! dual-stack socket, which is most of them, so the same host arrives as `a.b.c.d` from
//! the connection table and as `::ffff:a.b.c.d` from the flow events.
//!
//! Left alone, that one difference breaks four things at once, and each failure is silent:
//!
//! * **It cannot be probed.** A mapped address is [`std::net::IpAddr::V6`], so the ICMP
//!   backend refuses it as IPv6 — the endpoint is probed forever and never measured, which
//!   is exactly the "sits there mysteriously never updating" state this product forbids.
//! * **It is classified wrongly.** The private, loopback and tunnel-sentinel ranges are
//!   written as IPv4 CIDRs, so `::ffff:127.0.0.1` reads as an ordinary public address and a
//!   game's chatter with its own launcher becomes a probe target.
//! * **It is counted twice.** The same server discovered by both sources becomes two
//!   endpoints, each holding a slot against the per-application cap.
//! * **Its egress is a wildcard in disguise.** `::ffff:0.0.0.0` is not
//!   [`std::net::Ipv6Addr::is_unspecified`], so an unbound socket looks like a routing
//!   decision and a probe gets bound to nothing.
//!
//! So the mapping is undone here, once, on everything that crosses in. Above this crate
//! there is one address per host.

use std::net::{IpAddr, SocketAddr};

/// Rewrites an IPv4-mapped IPv6 address as the IPv4 address it stands for.
///
/// Only the `::ffff:0:0/96` mapping. IPv4-*compatible* addresses (`::a.b.c.d`) are
/// deliberately left alone: that form is deprecated, and `::1` sits inside it — folding it
/// would turn loopback into `0.0.0.1`.
pub(crate) fn unmap_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        IpAddr::V4(_) => address,
    }
}

/// The same, for a socket address. The port and any scope identifier are irrelevant once
/// the address is IPv4, which has neither.
pub(crate) fn unmap_socket(address: SocketAddr) -> SocketAddr {
    match unmap_ip(address.ip()) {
        IpAddr::V4(v4) if address.is_ipv6() => SocketAddr::new(IpAddr::V4(v4), address.port()),
        _ => address,
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    fn ip(raw: &str) -> IpAddr {
        raw.parse().expect("the literal must parse")
    }

    #[test]
    fn a_mapped_address_becomes_the_address_it_stands_for() {
        assert_eq!(
            unmap_ip(ip("::ffff:203.0.113.5")),
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5))
        );
    }

    #[test]
    fn a_mapped_wildcard_becomes_recognisably_unspecified() {
        // The bug this exists to stop: `::ffff:0.0.0.0` is not `is_unspecified`, so an
        // unbound socket looked like a routing decision and probes were bound to nothing.
        let unmapped = unmap_ip(ip("::ffff:0.0.0.0"));
        assert!(unmapped.is_unspecified());
    }

    #[test]
    fn a_mapped_private_address_can_be_classified_again() {
        // Left mapped, this reads as an ordinary public address, and a game's conversation
        // with its own router becomes a probe target.
        assert_eq!(
            unmap_ip(ip("::ffff:192.168.1.1")),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))
        );
        assert_eq!(
            unmap_ip(ip("::ffff:127.0.0.1")),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
    }

    #[test]
    fn a_genuine_ipv6_address_is_untouched() {
        let address = ip("2606:4700::1111");
        assert_eq!(unmap_ip(address), address);
    }

    #[test]
    fn an_ipv4_compatible_address_is_left_alone() {
        // The deprecated `::a.b.c.d` form, which contains `::1`. Folding it would turn
        // loopback into `0.0.0.1` — a real address belonging to someone else.
        assert_eq!(unmap_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)), ip("::1"));
        assert_eq!(unmap_ip(ip("::203.0.113.5")), ip("::203.0.113.5"));
    }

    #[test]
    fn an_ipv4_address_survives_unchanged() {
        let address = ip("203.0.113.5");
        assert_eq!(unmap_ip(address), address);
    }

    #[test]
    fn a_socket_address_keeps_its_port() {
        let mapped: SocketAddr = "[::ffff:203.0.113.5]:27015"
            .parse()
            .expect("the literal must parse");
        assert_eq!(
            unmap_socket(mapped),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)), 27_015)
        );
    }

    #[test]
    fn a_genuine_ipv6_socket_keeps_its_scope() {
        let scoped: SocketAddr = "[fe80::1%7]:443".parse().expect("the literal must parse");
        assert_eq!(unmap_socket(scoped), scoped);
    }
}
