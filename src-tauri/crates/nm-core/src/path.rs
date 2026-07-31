//! Where a path to a silent target stops working.
//!
//! When a destination answers nothing — common for AWS- and GCP-hosted game servers, which
//! drop ICMP and expose no port — a probe can still walk outward hop by hop with a limited
//! TTL and see how far packets get. This module turns that walk into a statement about
//! *position*: the user's own network, their provider's, or somewhere past a long-distance
//! link.
//!
//! # What this deliberately does not decide
//!
//! It does not say whose fault the failure is, and it does not claim to have found a
//! national border. Naming a border needs corroboration this walk cannot supply on its own —
//! the domestic and foreign baselines the app already keeps. What the walk *can* establish
//! is that a long-haul link was crossed before the path died, which is the observation the
//! verdict layer combines with those baselines.
//!
//! Keeping the two apart is the point. A confident "the border is blocking you" built on one
//! traceroute would be wrong often enough to make the whole product untrustworthy for the
//! people who most need it to be right.

use std::net::IpAddr;

use crate::address::{AddressClass, AddressPolicy};
use crate::sample::Rtt;

/// A round-trip increase large enough to mean a long-distance link was crossed.
///
/// Links inside one country are typically a few milliseconds; an intercontinental leg costs
/// tens. Thirty milliseconds sits well above ordinary domestic variation and well below any
/// intercontinental hop, so a step this size is evidence of distance rather than of a busy
/// router. It is not evidence of a *border* — see the module docs.
pub const LONG_HAUL_STEP_MS: f64 = 30.0;

/// One step of a path walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hop {
    /// Time-to-live the probe carried, which is this hop's distance in routers.
    pub ttl: u8,
    /// Which router answered, when one did.
    ///
    /// [`None`] is a silent hop: many routers are configured not to generate TTL-expired
    /// messages. A silent hop in the middle of a walk means nothing at all, which is why a
    /// walk steps over gaps instead of stopping at the first one.
    pub address: Option<IpAddr>,
    /// Round trip to that router, when it answered.
    pub rtt: Option<Rtt>,
}

impl Hop {
    /// A hop that answered.
    #[must_use]
    pub const fn answered(ttl: u8, address: IpAddr, rtt: Rtt) -> Self {
        Self {
            ttl,
            address: Some(address),
            rtt: Some(rtt),
        }
    }

    /// A hop that stayed silent.
    #[must_use]
    pub const fn silent(ttl: u8) -> Self {
        Self {
            ttl,
            address: None,
            rtt: None,
        }
    }
}

/// The result of walking outward towards a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathTrace {
    hops: Vec<Hop>,
    reached_target: bool,
}

impl PathTrace {
    /// Records a completed walk.
    #[must_use]
    pub fn new(hops: Vec<Hop>, reached_target: bool) -> Self {
        Self {
            hops,
            reached_target,
        }
    }

    /// Every hop attempted, in increasing distance.
    #[must_use]
    pub fn hops(&self) -> &[Hop] {
        &self.hops
    }

    /// Whether the target itself answered.
    #[must_use]
    pub const fn reached_target(&self) -> bool {
        self.reached_target
    }

    /// The furthest hop that answered.
    #[must_use]
    pub fn last_answering(&self) -> Option<&Hop> {
        self.hops.iter().rev().find(|hop| hop.address.is_some())
    }
}

/// How far a path got before it stopped working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathEnd {
    /// The target answered. Nothing is wrong with the path.
    Reached,
    /// Not one hop answered, not even the first router.
    ///
    /// Usually means TTL-expired messages are filtered locally rather than that the network
    /// is dead — the walk measured nothing and must not be presented as a failure.
    NothingAnswered,
    /// The furthest answering hop was inside the user's own network.
    InsideThisNetwork {
        /// Distance in routers of that hop.
        last_hop: u8,
    },
    /// The furthest answering hop was the provider's carrier-NAT equipment.
    InsideTheAccessNetwork {
        /// Distance in routers of that hop.
        last_hop: u8,
    },
    /// The path died on public infrastructure, with no long-haul link crossed first.
    ///
    /// Consistent with a failure inside the user's provider or its immediate upstream.
    BeforeAnyLongHaulLink {
        /// Distance in routers of the furthest answering hop.
        last_hop: u8,
    },
    /// The path died after crossing a link long enough to be intercontinental.
    ///
    /// The observation the verdict layer needs to distinguish "your provider" from
    /// "something beyond it", once the baselines agree.
    BeyondALongHaulLink {
        /// Distance in routers of the furthest answering hop.
        last_hop: u8,
        /// Distance in routers of the hop where the long jump appeared.
        long_haul_at: u8,
    },
}

/// Decides where a walk stopped working.
#[must_use]
pub fn classify(trace: &PathTrace, policy: &AddressPolicy) -> PathEnd {
    if trace.reached_target() {
        return PathEnd::Reached;
    }
    let Some(last) = trace.last_answering() else {
        return PathEnd::NothingAnswered;
    };
    let Some(address) = last.address else {
        return PathEnd::NothingAnswered;
    };

    match policy.classify(address) {
        AddressClass::Private | AddressClass::Loopback => {
            return PathEnd::InsideThisNetwork { last_hop: last.ttl }
        }
        AddressClass::CarrierGrade => {
            return PathEnd::InsideTheAccessNetwork { last_hop: last.ttl }
        }
        AddressClass::Routable | AddressClass::TunnelSentinel | AddressClass::Unusable => {}
    }

    long_haul_hop(trace).map_or(
        PathEnd::BeforeAnyLongHaulLink { last_hop: last.ttl },
        |long_haul_at| PathEnd::BeyondALongHaulLink {
            last_hop: last.ttl,
            long_haul_at,
        },
    )
}

/// Finds the first hop where the path crosses a long-distance link.
///
/// A jump of [`LONG_HAUL_STEP_MS`] over everything before it is only a *candidate*. It counts
/// as a long-haul link when every answering hop beyond it stays high, because distance does
/// not come back: once packets have crossed an ocean, no router further along is suddenly
/// close again.
///
/// The distinction matters because hop round trips are not a clean staircase. A router that
/// deprioritises generating TTL-expired messages answers far later than its neighbours, which
/// looks exactly like a long link until the next hop comes back down. Requiring the rise to
/// persist is what separates a busy control plane from a continent.
///
/// A candidate at the last answering hop has nothing after it to contradict it, and is
/// accepted: the jump is real evidence, and a path that dies immediately after crossing a
/// long link is precisely the case worth reporting.
fn long_haul_hop(trace: &PathTrace) -> Option<u8> {
    let answering: Vec<(u8, f64)> = trace
        .hops()
        .iter()
        .filter_map(|hop| hop.rtt.map(|rtt| (hop.ttl, rtt.as_millis_f64())))
        .collect();

    let mut highest: Option<f64> = None;
    for (index, (ttl, millis)) in answering.iter().enumerate() {
        if let Some(previous) = highest {
            if millis - previous >= LONG_HAUL_STEP_MS {
                let floor = previous + LONG_HAUL_STEP_MS;
                if answering[index + 1..]
                    .iter()
                    .all(|(_, later)| *later >= floor)
                {
                    return Some(*ttl);
                }
                // A spike that came back down. Deliberately left out of the running maximum
                // as well: letting it raise the baseline would hide a genuine crossing later.
                continue;
            }
        }
        highest = Some(highest.map_or(*millis, |seen: f64| seen.max(*millis)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> AddressPolicy {
        AddressPolicy::default()
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    fn ms(millis: u32) -> Rtt {
        Rtt::from_micros(millis * 1_000)
    }

    /// A walk over `(address, milliseconds)` pairs, starting at TTL 1.
    fn walk(hops: &[(&str, u32)]) -> PathTrace {
        let hops = hops
            .iter()
            .enumerate()
            .map(|(index, (address, millis))| {
                let ttl = u8::try_from(index + 1).unwrap();
                if address.is_empty() {
                    Hop::silent(ttl)
                } else {
                    Hop::answered(ttl, ip(address), ms(*millis))
                }
            })
            .collect();
        PathTrace::new(hops, false)
    }

    #[test]
    fn a_reached_target_needs_no_diagnosis() {
        let trace = PathTrace::new(vec![Hop::answered(1, ip("192.168.1.1"), ms(1))], true);
        assert_eq!(classify(&trace, &policy()), PathEnd::Reached);
    }

    #[test]
    fn a_walk_where_nothing_answered_measured_nothing() {
        // Not a failure of the network: locally filtered TTL-expired messages look exactly
        // like this, and reporting them as an outage would be a lie.
        let trace = PathTrace::new((1..=5).map(Hop::silent).collect(), false);
        assert_eq!(classify(&trace, &policy()), PathEnd::NothingAnswered);
        assert_eq!(
            classify(&PathTrace::new(Vec::new(), false), &policy()),
            PathEnd::NothingAnswered
        );
    }

    #[test]
    fn a_path_dying_at_the_home_router_is_the_users_own_network() {
        let trace = walk(&[("192.168.1.1", 1), ("", 0), ("", 0)]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::InsideThisNetwork { last_hop: 1 }
        );
    }

    #[test]
    fn a_path_dying_at_carrier_nat_is_the_providers_network() {
        // The distinction that keeps the app from telling someone to reboot a router that is
        // working perfectly.
        let trace = walk(&[("192.168.1.1", 1), ("100.64.0.1", 4), ("", 0)]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::InsideTheAccessNetwork { last_hop: 2 }
        );
    }

    #[test]
    fn a_path_dying_on_nearby_public_infrastructure_crossed_no_long_link() {
        let trace = walk(&[
            ("192.168.1.1", 1),
            ("203.0.113.1", 6),
            ("198.51.100.1", 11),
            ("", 0),
            ("", 0),
        ]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeforeAnyLongHaulLink { last_hop: 3 }
        );
    }

    #[test]
    fn a_large_jump_marks_a_long_haul_link() {
        let trace = walk(&[
            ("192.168.1.1", 1),
            ("203.0.113.1", 8),
            ("198.51.100.1", 12),
            ("192.0.2.1", 95), // the long leg
            ("203.0.113.9", 98),
            ("", 0),
        ]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeyondALongHaulLink {
                last_hop: 5,
                long_haul_at: 4
            }
        );
    }

    #[test]
    fn a_jump_just_under_the_threshold_is_not_a_long_haul_link() {
        let trace = walk(&[
            ("203.0.113.1", 5),
            ("198.51.100.1", 34), // +29 ms
            ("", 0),
        ]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeforeAnyLongHaulLink { last_hop: 2 }
        );
    }

    #[test]
    fn a_jump_exactly_at_the_threshold_counts() {
        let trace = walk(&[("203.0.113.1", 5), ("198.51.100.1", 35), ("", 0)]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeyondALongHaulLink {
                last_hop: 2,
                long_haul_at: 2
            }
        );
    }

    #[test]
    fn a_single_slow_to_answer_router_does_not_invent_a_long_haul_link() {
        // Routers deprioritise generating TTL-expired messages, so one hop can report far
        // more than its neighbours. Comparing against the previous hop alone would read the
        // recovery after it as a jump; comparing against the running maximum does not.
        let trace = walk(&[
            ("203.0.113.1", 4),
            ("198.51.100.1", 40), // busy control plane, not a long link
            ("192.0.2.1", 9),
            ("203.0.113.9", 12),
            ("", 0),
        ]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeforeAnyLongHaulLink { last_hop: 4 },
            "the recovery from a slow hop must not be read as crossing a long link"
        );
    }

    #[test]
    fn a_spike_does_not_hide_a_genuine_crossing_further_out() {
        // The other half of the same rule: a rejected spike must not raise the baseline, or
        // the real long link after it would look like an ordinary step.
        let trace = walk(&[
            ("203.0.113.1", 4),
            ("198.51.100.1", 45), // busy control plane
            ("192.0.2.1", 8),
            ("203.0.113.9", 120), // the actual long leg
            ("192.0.2.5", 124),
            ("", 0),
        ]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeyondALongHaulLink {
                last_hop: 5,
                long_haul_at: 4
            }
        );
    }

    #[test]
    fn the_first_long_jump_is_the_one_reported() {
        let trace = walk(&[
            ("203.0.113.1", 5),
            ("198.51.100.1", 60), // first long leg
            ("192.0.2.1", 200),   // second, further out
            ("203.0.113.9", 205),
            ("", 0),
        ]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeyondALongHaulLink {
                last_hop: 4,
                long_haul_at: 2
            }
        );
    }

    #[test]
    fn silent_hops_in_the_middle_do_not_end_the_walk() {
        // A gap says nothing; stopping at it would report a failure several hops closer than
        // the real one.
        let trace = walk(&[
            ("203.0.113.1", 5),
            ("", 0),
            ("", 0),
            ("198.51.100.1", 70),
            ("", 0),
        ]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeyondALongHaulLink {
                last_hop: 4,
                long_haul_at: 4
            }
        );
    }

    #[test]
    fn the_furthest_answering_hop_is_the_one_diagnosed() {
        let trace = walk(&[("203.0.113.1", 5), ("", 0), ("198.51.100.1", 9), ("", 0)]);
        assert_eq!(trace.last_answering().map(|hop| hop.ttl), Some(3));
    }

    #[test]
    fn a_trace_with_no_answering_hop_has_none() {
        let trace = PathTrace::new(vec![Hop::silent(1), Hop::silent(2)], false);
        assert_eq!(trace.last_answering(), None);
        assert_eq!(trace.hops().len(), 2);
    }

    #[test]
    fn a_tunnel_sentinel_hop_is_not_mistaken_for_a_local_one() {
        // A synthetic address can only appear here through misconfiguration, but treating it
        // as the user's own network would blame the wrong party outright.
        let trace = walk(&[("192.168.1.1", 1), ("198.18.0.1", 3), ("", 0)]);
        assert_eq!(
            classify(&trace, &policy()),
            PathEnd::BeforeAnyLongHaulLink { last_hop: 2 }
        );
    }
}
