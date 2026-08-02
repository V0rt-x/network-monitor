//! Proving, from the reply itself, that nothing on the internet sent it.
//!
//! A route lookup can tell us an endpoint leaves by a tunnel, and where the platform has
//! one that is the cheaper answer. But it is an inference from an adapter's type, it only
//! exists on Windows so far, and a tunnel wearing an adapter this build does not recognise
//! would slip past it. This module is the backstop, and it is stronger than the thing it
//! backs up: it does not infer that a reply is fake, it *proves* it, from a field the
//! operating system already hands us for free.
//!
//! # The proof
//!
//! Every IP packet carries a hop limit that each router decrements. Senders start it at one
//! of a handful of well-known values — 64 on Linux and macOS, 128 on Windows, 255 on much
//! network equipment — so the value that *arrives* says how many routers the packet crossed.
//! A reply from a service across an ocean arrives with its initial value less a dozen or so.
//!
//! A reply arriving with the initial value **untouched** crossed no router at all. It came
//! from something on this machine or on the wire beside it. For a public address that is not
//! a suspicious number, it is an impossible one, and it is what a local TUN client answering
//! echo requests on behalf of the whole internet produces. Measured on a developer machine:
//! a reply purporting to come from a public resolver, `TTL=128`, under a millisecond — beside
//! the same probe pinned to the physical adapter, `TTL=57`, and a real few milliseconds.
//!
//! # Where it deliberately stops
//!
//! The claim is only made for an address that ought to be far away
//! ([`AddressClass::Routable`]). A machine genuinely sharing a segment with a public address
//! — a server in a datacenter, not this product's user — would be zero hops from a real
//! peer, and would be called tunnelled here when it is not.
//!
//! That is accepted, because of which way it fails. Being wrong here moves an endpoint onto
//! the end-to-end probe, which measures the path correctly whether or not a tunnel is in it.
//! The cost of a false positive is a slightly more expensive probe; the cost of a false
//! negative is the product reporting one millisecond to every game server on earth. Only one
//! of those is worth guarding against.

use crate::address::AddressClass;

/// The hop limits senders start at, descending.
///
/// Not a guess at what a particular host uses: the arriving value is compared against the
/// smallest of these that is not below it, which is the standard way of reading a TTL
/// without knowing who sent it.
const INITIAL_HOP_LIMITS: [u8; 4] = [255, 128, 64, 32];

/// How many routers a packet crossed, given the hop limit it arrived with.
///
/// [`None`] when the value is above every initial limit, which cannot happen for a `u8`
/// against a list containing 255 but is expressed rather than assumed.
#[must_use]
pub fn hops_travelled(arrived_with: u8) -> Option<u8> {
    INITIAL_HOP_LIMITS
        .into_iter()
        .filter(|limit| *limit >= arrived_with)
        .min()
        .map(|limit| limit - arrived_with)
}

/// Whether a reply from an endpoint of `class` proves it was answered on this machine.
///
/// True only for a public address whose reply crossed no router — see the module
/// documentation for why that is a proof rather than a suspicion, and for the one case it
/// deliberately gets wrong.
#[must_use]
pub fn reply_was_answered_locally(class: AddressClass, arrived_with: u8) -> bool {
    class == AddressClass::Routable && hops_travelled(arrived_with) == Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_read_against_the_next_initial_limit_above_it() {
        // The real observation this module was written from, both halves of it: the same
        // probe answered by a tunnel and answered by the internet.
        assert_eq!(hops_travelled(128), Some(0));
        assert_eq!(hops_travelled(57), Some(7));
    }

    #[test]
    fn every_initial_limit_reads_as_no_router_crossed() {
        for limit in INITIAL_HOP_LIMITS {
            assert_eq!(hops_travelled(limit), Some(0), "{limit}");
        }
    }

    #[test]
    fn hop_counts_are_measured_from_the_limit_just_above() {
        assert_eq!(
            hops_travelled(254),
            Some(1),
            "network equipment starts at 255"
        );
        assert_eq!(hops_travelled(127), Some(1), "a Windows host starts at 128");
        assert_eq!(hops_travelled(63), Some(1), "a Linux host starts at 64");
        assert_eq!(hops_travelled(31), Some(1));
        // Just below a limit is a long path from the next one up, not a negative count.
        assert_eq!(hops_travelled(65), Some(63));
    }

    #[test]
    fn a_hop_limit_of_zero_is_read_rather_than_refused() {
        // It cannot arrive — a router discards the packet instead — but the arithmetic
        // must not depend on that being true.
        assert_eq!(hops_travelled(0), Some(32));
    }

    #[test]
    fn a_public_address_answering_from_zero_hops_away_is_proven_local() {
        // The finding: a TUN client answers echo requests for the whole internet itself,
        // and its replies carry the machine's own initial hop limit.
        assert!(reply_was_answered_locally(AddressClass::Routable, 128));
        assert!(reply_was_answered_locally(AddressClass::Routable, 64));
        assert!(reply_was_answered_locally(AddressClass::Routable, 255));
    }

    #[test]
    fn a_reply_that_crossed_even_one_router_proves_nothing() {
        // The user's own gateway is one hop out. The rule must not fire on a short path,
        // only on an impossible one.
        for arrived_with in [127, 63, 254, 57, 12] {
            assert!(
                !reply_was_answered_locally(AddressClass::Routable, arrived_with),
                "{arrived_with}"
            );
        }
    }

    #[test]
    fn the_claim_is_only_made_about_an_address_that_ought_to_be_far_away() {
        // A LAN peer or the user's own router really is zero hops away, and calling that
        // forgery would be false. An endpoint already known to be tunnelled needs no proof.
        for class in [
            AddressClass::Private,
            AddressClass::Loopback,
            AddressClass::CarrierGrade,
            AddressClass::Unusable,
            AddressClass::TunnelSentinel,
            AddressClass::TunnelledEgress,
        ] {
            assert!(!reply_was_answered_locally(class, 128), "{class:?}");
        }
    }
}
