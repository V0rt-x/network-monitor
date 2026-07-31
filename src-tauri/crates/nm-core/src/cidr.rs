//! CIDR blocks.
//!
//! Hand-rolled rather than pulled from a crate: the product needs exactly two operations
//! — parse a block written by a human in a config file, and test whether an address falls
//! inside it — and both are a few lines of shifting. A dependency would be more surface
//! than substance.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use crate::Error;

/// A block of addresses, written `network/prefix`.
///
/// Host bits below the prefix are cleared on construction, so `10.1.2.3/8` and `10.0.0.0/8`
/// are the same block. Config files are edited by hand and the sloppier spelling is the
/// common one; [`IpCidr::network`] always reports the canonical form back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IpCidr {
    network: IpAddr,
    prefix_len: u8,
}

impl IpCidr {
    /// Builds a block, clearing any host bits in `network`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidCidr`] if `prefix_len` exceeds the address family's width
    /// (32 for IPv4, 128 for IPv6).
    pub fn new(network: IpAddr, prefix_len: u8) -> Result<Self, Error> {
        let width = match network {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix_len > width {
            return Err(Error::InvalidCidr {
                raw: format!("{network}/{prefix_len}"),
            });
        }
        Ok(Self {
            network: mask(network, prefix_len),
            prefix_len,
        })
    }

    /// The canonical network address, with host bits cleared.
    #[must_use]
    pub const fn network(&self) -> IpAddr {
        self.network
    }

    /// How many leading bits the block fixes.
    #[must_use]
    pub const fn prefix_len(&self) -> u8 {
        self.prefix_len
    }

    /// Whether `address` falls inside the block.
    ///
    /// Addresses of a different family never match: an IPv4 address is not "inside" an
    /// IPv6 block, however the bits line up.
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.network, address) {
            (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_)) => {
                mask(address, self.prefix_len) == self.network
            }
            _ => false,
        }
    }
}

/// Clears every bit below `prefix_len`.
fn mask(address: IpAddr, prefix_len: u8) -> IpAddr {
    match address {
        IpAddr::V4(v4) => {
            // A shift by the full width is undefined, so the "fixes nothing" case is
            // handled before any shifting happens.
            if prefix_len == 0 {
                return IpAddr::V4(Ipv4Addr::UNSPECIFIED);
            }
            let shift = 32 - u32::from(prefix_len);
            IpAddr::V4(Ipv4Addr::from_bits((v4.to_bits() >> shift) << shift))
        }
        IpAddr::V6(v6) => {
            if prefix_len == 0 {
                return IpAddr::V6(Ipv6Addr::UNSPECIFIED);
            }
            let shift = 128 - u32::from(prefix_len);
            IpAddr::V6(Ipv6Addr::from_bits((v6.to_bits() >> shift) << shift))
        }
    }
}

impl FromStr for IpCidr {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let invalid = || Error::InvalidCidr {
            raw: raw.to_owned(),
        };
        let (address, prefix) = raw.split_once('/').ok_or_else(invalid)?;
        let address: IpAddr = address.parse().map_err(|_| invalid())?;
        let prefix: u8 = prefix.parse().map_err(|_| invalid())?;
        Self::new(address, prefix)
    }
}

impl fmt::Display for IpCidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cidr(raw: &str) -> IpCidr {
        raw.parse().unwrap()
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    #[test]
    fn parses_and_displays_canonically() {
        let block = cidr("198.18.0.0/15");
        assert_eq!(block.network(), ip("198.18.0.0"));
        assert_eq!(block.prefix_len(), 15);
        assert_eq!(block.to_string(), "198.18.0.0/15");
    }

    #[test]
    fn clears_host_bits_written_by_hand() {
        // The spelling a person is likely to type must mean the block they meant.
        assert_eq!(cidr("10.1.2.3/8"), cidr("10.0.0.0/8"));
        assert_eq!(cidr("10.1.2.3/8").to_string(), "10.0.0.0/8");
        assert_eq!(cidr("2001:db8::1/32"), cidr("2001:db8::/32"));
    }

    #[test]
    fn rejects_malformed_input() {
        for raw in [
            "",
            "10.0.0.0",
            "10.0.0.0/",
            "/8",
            "10.0.0.0/33",
            "::/129",
            "not-an-address/8",
            "10.0.0.0/eight",
            "10.0.0.0/8/8",
        ] {
            assert!(raw.parse::<IpCidr>().is_err(), "{raw:?} should not parse");
        }
    }

    #[test]
    fn accepts_the_widest_and_narrowest_prefixes() {
        assert!(cidr("0.0.0.0/0").contains(ip("203.0.113.9")));
        assert!(cidr("::/0").contains(ip("2001:db8::1")));

        let single = cidr("203.0.113.9/32");
        assert!(single.contains(ip("203.0.113.9")));
        assert!(!single.contains(ip("203.0.113.10")));
    }

    #[test]
    fn contains_respects_block_boundaries() {
        let block = cidr("198.18.0.0/15");
        assert!(block.contains(ip("198.18.0.0")));
        assert!(block.contains(ip("198.18.5.114")));
        assert!(
            block.contains(ip("198.19.255.255")),
            "the block spans two /16s"
        );
        assert!(!block.contains(ip("198.17.255.255")));
        assert!(!block.contains(ip("198.20.0.0")));
    }

    #[test]
    fn contains_works_for_ipv6() {
        let block = cidr("fc00::/18");
        assert!(block.contains(ip("fc00::1")));
        assert!(block.contains(ip("fc00:3fff:ffff:ffff:ffff:ffff:ffff:ffff")));
        assert!(!block.contains(ip("fc00:4000::1")));
        assert!(!block.contains(ip("fd00::1")));
    }

    #[test]
    fn families_never_match_each_other() {
        // Both families reduce to integers; without an explicit family check a /0 block
        // would swallow everything.
        assert!(!cidr("0.0.0.0/0").contains(ip("::1")));
        assert!(!cidr("::/0").contains(ip("127.0.0.1")));
        assert!(!cidr("198.18.0.0/15").contains(ip("::ffff:198.18.0.1")));
    }

    #[test]
    fn rejects_a_prefix_wider_than_the_family() {
        assert!(IpCidr::new(ip("10.0.0.0"), 33).is_err());
        assert!(IpCidr::new(ip("::"), 129).is_err());
        assert!(IpCidr::new(ip("10.0.0.0"), 32).is_ok());
        assert!(IpCidr::new(ip("::"), 128).is_ok());
    }
}
