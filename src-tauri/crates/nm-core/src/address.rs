//! Deciding what an address *is*, before deciding how to measure it.
//!
//! An endpoint discovered from the OS connection table is not automatically something
//! worth probing. The reality check (`docs/measurement-reality-check.md`) found the case
//! that forces this module to exist: a router running sing-box with fake-IP answers DNS
//! with synthetic addresses out of a reserved range and remaps them to the real
//! destination when the connection is made. Probing such an address with ICMP measures
//! nothing, and a TCP connect to it returns in well under a millisecond because the
//! tunnel completes the handshake locally вЂ” a fake-*good* number, which is worse than a
//! fake-bad one, because it would tell the user their network is fine.
//!
//! So classification happens first, and it decides which probe kinds may be believed.

use std::net::IpAddr;

use crate::cidr::IpCidr;
use crate::Error;

/// What an address is, as far as measurement is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AddressClass {
    /// An ordinary public address. Every probe kind means what it says.
    Routable,
    /// A synthetic address a local tunnel will remap when the connection is made.
    ///
    /// ICMP measures nothing and a TCP connect reports the tunnel's own latency. Only a
    /// probe that exchanges data end to end вЂ” a TLS handshake, an HTTP request вЂ” travels
    /// the real path.
    TunnelSentinel,
    /// A private or carrier-shared address: a LAN peer, a router, a CGNAT hop.
    ///
    /// Measurable, but it says nothing about reaching the internet.
    Private,
    /// This machine.
    Loopback,
    /// Reserved, multicast, link-local or documentation space. Nothing to measure.
    Unusable,
}

impl AddressClass {
    /// Whether a round-trip time measured by ICMP or a bare TCP connect can be trusted.
    ///
    /// False for a tunnel sentinel, which is the whole point of this module.
    #[must_use]
    pub const fn trusts_transport_rtt(self) -> bool {
        matches!(self, Self::Routable | Self::Private | Self::Loopback)
    }

    /// Whether probing this address tells us anything about reaching the internet.
    #[must_use]
    pub const fn worth_probing(self) -> bool {
        matches!(self, Self::Routable | Self::TunnelSentinel)
    }
}

/// sing-box's default fake-IP range for IPv4, and the range RFC 2544 reserves for
/// benchmarking вЂ” which is why a tunnel can safely borrow it: no real service lives here.
const DEFAULT_SENTINEL_V4: &str = "198.18.0.0/15";

/// sing-box's default fake-IP range for IPv6.
///
/// It sits inside the `fc00::/7` unique-local space, so an address here is ambiguous on
/// its face: it could be a genuine ULA host. The tunnel reading is chosen because a
/// unique-local address is not something this product would be asked to measure, while a
/// fake-IP sentinel very much is.
const DEFAULT_SENTINEL_V6: &str = "fc00::/18";

/// Ranges that are never worth probing, whatever else is configured.
const UNUSABLE: &[&str] = &[
    "0.0.0.0/8",       // "this network"
    "169.254.0.0/16",  // link-local
    "192.0.2.0/24",    // documentation (TEST-NET-1)
    "198.51.100.0/24", // documentation (TEST-NET-2)
    "203.0.113.0/24",  // documentation (TEST-NET-3)
    "224.0.0.0/4",     // multicast
    "240.0.0.0/4",     // reserved, includes the broadcast address
    "::/128",          // unspecified
    "2001:db8::/32",   // documentation
    "fe80::/10",       // link-local
    "ff00::/8",        // multicast
];

/// Ranges that are real but local to the site.
const PRIVATE: &[&str] = &[
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "100.64.0.0/10", // carrier-grade NAT
    "fc00::/7",      // unique local
];

/// How addresses are classified, including which ranges a local tunnel uses.
///
/// The sentinel ranges are configurable because sing-box's defaults are only defaults:
/// a user who changed them would otherwise have their endpoints silently misclassified
/// as ordinary public addresses and measured with probes that lie.
#[derive(Debug, Clone)]
pub struct AddressPolicy {
    tunnel_sentinels: Vec<IpCidr>,
    private: Vec<IpCidr>,
    unusable: Vec<IpCidr>,
}

impl AddressPolicy {
    /// Builds a policy whose tunnel sentinel ranges replace the defaults.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidCidr`] if a range cannot be parsed.
    pub fn with_sentinels<I, S>(sentinels: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Ok(Self {
            tunnel_sentinels: parse_all(sentinels)?,
            private: parse_all(PRIVATE)?,
            unusable: parse_all(UNUSABLE)?,
        })
    }

    /// The ranges currently treated as tunnel sentinels.
    #[must_use]
    pub fn sentinels(&self) -> &[IpCidr] {
        &self.tunnel_sentinels
    }

    /// Classifies an address.
    ///
    /// Sentinels are tested before private ranges on purpose: sing-box's IPv6 default
    /// sits inside unique-local space, so the checks overlap and the more specific
    /// meaning has to win.
    #[must_use]
    pub fn classify(&self, address: IpAddr) -> AddressClass {
        if address.is_loopback() {
            return AddressClass::Loopback;
        }
        if contains_any(&self.tunnel_sentinels, address) {
            return AddressClass::TunnelSentinel;
        }
        if contains_any(&self.unusable, address) {
            return AddressClass::Unusable;
        }
        if contains_any(&self.private, address) {
            return AddressClass::Private;
        }
        AddressClass::Routable
    }
}

impl Default for AddressPolicy {
    /// The defaults, matching an unmodified sing-box installation.
    ///
    /// Falls back to an empty range list if a built-in constant fails to parse, which
    /// cannot happen вЂ” the constants are covered by a test вЂ” but must not be a panic in
    /// library code either.
    fn default() -> Self {
        Self::with_sentinels([DEFAULT_SENTINEL_V4, DEFAULT_SENTINEL_V6]).unwrap_or(Self {
            tunnel_sentinels: Vec::new(),
            private: Vec::new(),
            unusable: Vec::new(),
        })
    }
}

fn parse_all<I, S>(raw: I) -> Result<Vec<IpCidr>, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    raw.into_iter()
        .map(|entry| entry.as_ref().parse())
        .collect()
}

fn contains_any(blocks: &[IpCidr], address: IpAddr) -> bool {
    blocks.iter().any(|block| block.contains(address))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    fn classify(raw: &str) -> AddressClass {
        AddressPolicy::default().classify(ip(raw))
    }

    #[test]
    fn built_in_ranges_all_parse() {
        // Default() swallows a parse failure rather than panicking, so the constants need
        // a test of their own or a typo would silently disable classification.
        assert!(parse_all(UNUSABLE).is_ok());
        assert!(parse_all(PRIVATE).is_ok());
        assert!(parse_all([DEFAULT_SENTINEL_V4, DEFAULT_SENTINEL_V6]).is_ok());
        assert_eq!(AddressPolicy::default().sentinels().len(), 2);
    }

    #[test]
    fn ordinary_public_addresses_are_routable() {
        for raw in ["1.1.1.1", "8.8.8.8", "9.9.9.9", "2606:4700::1"] {
            assert_eq!(classify(raw), AddressClass::Routable, "{raw}");
        }
    }

    #[test]
    fn fakeip_sentinels_are_recognised() {
        // The range sing-box hands out by default, at both edges.
        assert_eq!(classify("198.18.0.0"), AddressClass::TunnelSentinel);
        assert_eq!(classify("198.19.255.255"), AddressClass::TunnelSentinel);
        assert_eq!(classify("fc00::1"), AddressClass::TunnelSentinel);
        // Just outside it.
        assert_eq!(classify("198.17.255.255"), AddressClass::Routable);
        assert_eq!(classify("198.20.0.0"), AddressClass::Routable);
    }

    #[test]
    fn the_ipv6_sentinel_range_wins_over_unique_local() {
        // fc00::/18 sits inside fc00::/7, so order of checks decides the answer.
        assert_eq!(classify("fc00::1"), AddressClass::TunnelSentinel);
        assert_eq!(classify("fd00::1"), AddressClass::Private);
    }

    #[test]
    fn private_and_carrier_ranges_are_private() {
        for raw in [
            "10.1.101.1",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "fd12:3456::1",
        ] {
            assert_eq!(classify(raw), AddressClass::Private, "{raw}");
        }
    }

    #[test]
    fn loopback_is_its_own_class() {
        assert_eq!(classify("127.0.0.1"), AddressClass::Loopback);
        assert_eq!(classify("127.10.20.30"), AddressClass::Loopback);
        assert_eq!(classify("::1"), AddressClass::Loopback);
    }

    #[test]
    fn reserved_space_is_unusable() {
        for raw in [
            "0.0.0.0",
            "169.254.1.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
        ] {
            assert_eq!(classify(raw), AddressClass::Unusable, "{raw}");
        }
    }

    #[test]
    fn transport_rtt_is_distrusted_only_for_tunnel_sentinels() {
        // The rule the whole module exists for.
        assert!(!AddressClass::TunnelSentinel.trusts_transport_rtt());
        assert!(AddressClass::Routable.trusts_transport_rtt());
        assert!(AddressClass::Private.trusts_transport_rtt());
        assert!(AddressClass::Loopback.trusts_transport_rtt());
        assert!(!AddressClass::Unusable.trusts_transport_rtt());
    }

    #[test]
    fn a_tunnel_sentinel_is_still_worth_probing_with_the_right_kind() {
        // It must not be discarded: a TLS or HTTP probe does travel the real path.
        assert!(AddressClass::TunnelSentinel.worth_probing());
        assert!(AddressClass::Routable.worth_probing());
        assert!(!AddressClass::Unusable.worth_probing());
        assert!(!AddressClass::Loopback.worth_probing());
    }

    #[test]
    fn sentinel_ranges_can_be_reconfigured() {
        // A user who changed sing-box's fake-IP range must not have their endpoints
        // silently reclassified as ordinary public addresses.
        let policy = AddressPolicy::with_sentinels(["100.64.0.0/10"]).unwrap();
        assert_eq!(
            policy.classify(ip("100.64.0.1")),
            AddressClass::TunnelSentinel
        );
        assert_eq!(
            policy.classify(ip("198.18.0.1")),
            AddressClass::Routable,
            "the default range is replaced, not extended"
        );
    }

    #[test]
    fn a_malformed_configured_range_is_rejected() {
        assert!(AddressPolicy::with_sentinels(["not-a-range"]).is_err());
        assert!(AddressPolicy::with_sentinels(["198.18.0.0/99"]).is_err());
    }

    #[test]
    fn an_empty_sentinel_list_disables_tunnel_detection() {
        let policy = AddressPolicy::with_sentinels(Vec::<String>::new()).unwrap();
        assert_eq!(policy.classify(ip("198.18.0.1")), AddressClass::Routable);
    }
}
