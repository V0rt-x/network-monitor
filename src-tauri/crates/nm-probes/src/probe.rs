//! The probe seam: what a probe is, and which kinds may be believed for a given address.

use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use nm_core::address::AddressClass;
use nm_core::sample::ProbeOutcome;
use nm_core::target::TargetAddress;

use crate::Error;

/// How a probe reaches its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ProbeKind {
    /// An ICMP echo request. Cheapest, and the only kind that needs no open port.
    IcmpEcho,
    /// A bare TCP connection attempt.
    TcpConnect,
    /// A full TLS handshake with the target.
    TlsHandshake,
}

impl ProbeKind {
    /// Whether this probe's round-trip time still means something when a local tunnel
    /// sits between the application and the network.
    ///
    /// Only a probe that exchanges data end to end does. A tunnel answers a bare TCP
    /// handshake itself, in a fraction of a millisecond, and never forwards an ICMP echo
    /// at all — see `docs/measurement-reality-check.md`.
    #[must_use]
    pub const fn survives_a_tunnel(self) -> bool {
        matches!(self, Self::TlsHandshake)
    }

    /// Whether this probe needs a port on the target.
    #[must_use]
    pub const fn needs_a_port(self) -> bool {
        matches!(self, Self::TcpConnect | Self::TlsHandshake)
    }
}

/// Preference order for an address whose transport measurements can be trusted:
/// cheapest first, so the probe budget stretches furthest.
const DIRECT_PREFERENCE: &[ProbeKind] = &[
    ProbeKind::IcmpEcho,
    ProbeKind::TcpConnect,
    ProbeKind::TlsHandshake,
];

/// Preference order behind a local tunnel: only an end-to-end exchange is honest.
const TUNNELLED_PREFERENCE: &[ProbeKind] = &[ProbeKind::TlsHandshake];

/// One thing to probe, and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeTarget {
    /// Where to probe.
    pub address: TargetAddress,
    /// Local address the probe must egress from.
    ///
    /// Set this to the address the monitored application's own flow uses, and the probe
    /// follows the same interface, tunnel or accelerator. Without it the OS picks, and a
    /// probe can silently measure a different route than the one being diagnosed.
    pub source: Option<IpAddr>,
    /// How long to wait before calling the attempt a timeout.
    pub timeout: Duration,
}

impl ProbeTarget {
    /// A probe to `address` with no routing constraint.
    #[must_use]
    pub const fn new(address: TargetAddress, timeout: Duration) -> Self {
        Self {
            address,
            source: None,
            timeout,
        }
    }

    /// The same probe, pinned to a local egress address.
    #[must_use]
    pub const fn from_source(mut self, source: IpAddr) -> Self {
        self.source = Some(source);
        self
    }
}

/// Something that can measure one target.
///
/// Implementations are the extensibility seam for new probe kinds: a new kind implements
/// this trait and appears in the preference lists, rather than growing a code path
/// somewhere else.
///
/// Implementations must be cancel-safe — the runner drops the future when a probe is
/// abandoned — and must treat silence as a measurement rather than an error.
#[async_trait]
pub trait Prober: Send + Sync {
    /// Which kind of probe this is.
    fn kind(&self) -> ProbeKind;

    /// Measures one target.
    ///
    /// # Errors
    ///
    /// Returns an error only when the probe could not be carried out at all. A target
    /// that stays silent, refuses the connection or is reported unreachable is a
    /// [`ProbeOutcome`], not an error: those are answers, and a failure of our own must
    /// never be recorded as someone else's packet loss.
    async fn probe(&self, target: &ProbeTarget) -> Result<ProbeOutcome, Error>;
}

/// Chooses the probe kind to use for an address, given what is available.
///
/// This is the gate that stops the engine lying. For an address a local tunnel will
/// remap, every kind whose timing the tunnel would fake is excluded — even if that
/// leaves nothing usable, in which case the caller must surface "cannot measure this
/// endpoint" rather than a number.
///
/// # Errors
///
/// Returns [`Error::NothingUsable`] when no available kind can honestly measure the
/// address.
pub fn select_kind(class: AddressClass, available: &[ProbeKind]) -> Result<ProbeKind, Error> {
    if !class.worth_probing() {
        return Err(Error::NothingUsable { class });
    }

    let preference = if class.trusts_transport_rtt() {
        DIRECT_PREFERENCE
    } else {
        TUNNELLED_PREFERENCE
    };

    preference
        .iter()
        .find(|kind| available.contains(kind))
        .copied()
        .ok_or(Error::NothingUsable { class })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[ProbeKind] = &[
        ProbeKind::IcmpEcho,
        ProbeKind::TcpConnect,
        ProbeKind::TlsHandshake,
    ];

    #[test]
    fn only_an_end_to_end_exchange_survives_a_tunnel() {
        assert!(ProbeKind::TlsHandshake.survives_a_tunnel());
        assert!(!ProbeKind::IcmpEcho.survives_a_tunnel());
        assert!(
            !ProbeKind::TcpConnect.survives_a_tunnel(),
            "a tunnel completes the handshake itself and reports its own latency"
        );
    }

    #[test]
    fn a_direct_address_prefers_the_cheapest_probe() {
        assert_eq!(
            select_kind(AddressClass::Routable, ALL).unwrap(),
            ProbeKind::IcmpEcho
        );
    }

    #[test]
    fn a_direct_address_falls_back_in_order() {
        assert_eq!(
            select_kind(
                AddressClass::Routable,
                &[ProbeKind::TcpConnect, ProbeKind::TlsHandshake]
            )
            .unwrap(),
            ProbeKind::TcpConnect
        );
        assert_eq!(
            select_kind(AddressClass::Routable, &[ProbeKind::TlsHandshake]).unwrap(),
            ProbeKind::TlsHandshake
        );
    }

    #[test]
    fn a_tunnelled_address_refuses_every_probe_a_tunnel_would_fake() {
        // Even though both are offered, neither may be used: ICMP never leaves the
        // tunnel and TCP reports the tunnel's own sub-millisecond handshake.
        let error = select_kind(
            AddressClass::TunnelSentinel,
            &[ProbeKind::IcmpEcho, ProbeKind::TcpConnect],
        )
        .unwrap_err();
        assert_eq!(
            error,
            Error::NothingUsable {
                class: AddressClass::TunnelSentinel
            }
        );
    }

    #[test]
    fn a_tunnelled_address_uses_tls_when_it_is_available() {
        assert_eq!(
            select_kind(AddressClass::TunnelSentinel, ALL).unwrap(),
            ProbeKind::TlsHandshake,
            "TLS must win over the cheaper kinds here, not merely be allowed"
        );
    }

    #[test]
    fn addresses_not_worth_probing_are_refused_outright() {
        for class in [
            AddressClass::Unusable,
            AddressClass::Loopback,
            AddressClass::Private,
        ] {
            assert!(select_kind(class, ALL).is_err(), "{class:?}");
        }
    }

    #[test]
    fn no_available_prober_is_an_error_rather_than_a_silent_skip() {
        assert!(select_kind(AddressClass::Routable, &[]).is_err());
    }

    #[test]
    fn a_target_carries_its_egress_binding() {
        let address = TargetAddress::with_port("203.0.113.7".parse().unwrap(), 443);
        let plain = ProbeTarget::new(address, Duration::from_secs(1));
        assert_eq!(plain.source, None);

        let source: IpAddr = "192.0.2.9".parse().unwrap();
        let bound = plain.clone().from_source(source);
        assert_eq!(bound.source, Some(source));
        assert_eq!(bound.address, address);
    }
}
