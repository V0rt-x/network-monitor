//! TCP handshake probing.
//!
//! The fallback for a host that ignores ICMP but keeps a port open. `connect` returns when
//! the peer's SYN-ACK arrives, so the elapsed time is one round trip — the same quantity an
//! echo measures, obtained from a host that refuses to echo.
//!
//! One platform behaviour shapes how this prober may be scheduled. **Windows does not
//! report a refused connection promptly**: measured on loopback, where the reset arrives
//! instantly, `connect` still takes ~2 s to surface `ConnectionRefused`, because the stack
//! retries the attempt before believing the reset. A probe deadline shorter than that turns
//! every closed port into a timeout — that is, into fabricated packet loss. Whatever
//! schedules this prober must therefore allow it several seconds, and treat a long-running
//! TCP probe as normal rather than as a stuck one.
//!
//! Its blind spot is the reason [`crate::probe::select_kind`] exists: when a local tunnel
//! terminates the connection itself, the handshake completes in a fraction of a millisecond
//! and this prober reports a number that is precise, reproducible, and about nothing but
//! the tunnel. Nothing here can detect that; the address classifier decides, and this type
//! is simply never selected for such an endpoint.

use async_trait::async_trait;
use nm_core::sample::{ProbeOutcome, Rtt};

use crate::probe::{ProbeKind, ProbeTarget, Prober};
use crate::socket::{connect, Connected};
use crate::Error;

/// Measures the time to complete a TCP handshake with the target.
#[derive(Debug, Clone, Copy, Default)]
pub struct TcpConnectProber;

impl TcpConnectProber {
    /// Creates a prober.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Prober for TcpConnectProber {
    fn kind(&self) -> ProbeKind {
        ProbeKind::TcpConnect
    }

    async fn probe(&self, target: &ProbeTarget) -> Result<ProbeOutcome, Error> {
        match tokio::time::timeout(target.timeout, connect(target, ProbeKind::TcpConnect)).await {
            Ok(Ok(Connected::Stream { stream, elapsed })) => {
                // Dropping the stream resets the connection; see `socket::open_socket`.
                drop(stream);
                Ok(ProbeOutcome::Success(Rtt::from_duration(elapsed)))
            }
            Ok(Ok(Connected::Settled(outcome))) => Ok(outcome),
            Ok(Err(error)) => Err(error),
            Err(_elapsed) => Ok(ProbeOutcome::Timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    use nm_core::target::TargetAddress;
    use tokio::net::TcpListener;

    use super::*;

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn target(port: u16) -> ProbeTarget {
        ProbeTarget::new(
            TargetAddress::with_port(LOOPBACK, port),
            Duration::from_secs(2),
        )
    }

    /// A listener bound to an ephemeral loopback port, returned with that port.
    ///
    /// The connection completes from the backlog without anyone calling `accept`, so the
    /// test has no race to lose.
    async fn listening() -> (TcpListener, u16) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    #[test]
    fn reports_its_own_kind() {
        assert_eq!(TcpConnectProber::new().kind(), ProbeKind::TcpConnect);
    }

    #[tokio::test]
    async fn a_completed_handshake_is_a_round_trip_time() {
        let (_listener, port) = listening().await;
        let outcome = TcpConnectProber::new().probe(&target(port)).await.unwrap();
        assert!(
            matches!(outcome, ProbeOutcome::Success(_)),
            "expected a measurement, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_refused_connection_is_a_negative_answer_rather_than_loss() {
        // Releasing the port first guarantees nothing is listening on it.
        let (listener, port) = listening().await;
        drop(listener);

        // Ten seconds because Windows sits on a refusal for ~2 s even on loopback; see the
        // module docs. The generous deadline is what makes this test portable, and the
        // reason it is written at all — with the product's usual one-second interval this
        // exact case would come back as a timeout.
        let probed = ProbeTarget::new(
            TargetAddress::with_port(LOOPBACK, port),
            Duration::from_secs(10),
        );
        assert_eq!(
            TcpConnectProber::new().probe(&probed).await.unwrap(),
            ProbeOutcome::Unreachable
        );
    }

    #[tokio::test]
    async fn probes_egress_from_the_requested_source_address() {
        let (_listener, port) = listening().await;
        let probed = target(port).from_source(LOOPBACK);
        let outcome = TcpConnectProber::new().probe(&probed).await.unwrap();
        assert!(matches!(outcome, ProbeOutcome::Success(_)));
    }

    #[tokio::test]
    async fn a_target_without_a_port_is_refused_rather_than_guessed() {
        let probed = ProbeTarget::new(TargetAddress::icmp(LOOPBACK), Duration::from_secs(1));
        assert_eq!(
            TcpConnectProber::new().probe(&probed).await.unwrap_err(),
            Error::PortRequired {
                kind: ProbeKind::TcpConnect
            }
        );
    }

    #[tokio::test]
    async fn a_mismatched_source_family_fails_instead_of_letting_the_os_choose() {
        // Falling back to an OS-chosen source would measure a different route than the one
        // being diagnosed, which is the failure source binding exists to prevent.
        let probed = target(1).from_source(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST));
        assert_eq!(
            TcpConnectProber::new().probe(&probed).await.unwrap_err(),
            Error::SourceFamilyMismatch
        );
    }

    #[tokio::test]
    async fn a_deadline_that_expires_yields_a_measurement_rather_than_an_error() {
        // A closed port with a deadline far shorter than Windows' refusal delay. Which
        // branch runs depends on the platform — the timeout on Windows, the refusal
        // elsewhere — and the point being asserted is the one both must satisfy: giving up
        // is something we report about the network, never a failure of our own.
        let (listener, port) = listening().await;
        drop(listener);
        let probed = ProbeTarget::new(
            TargetAddress::with_port(LOOPBACK, port),
            Duration::from_millis(50),
        );

        let outcome = TcpConnectProber::new().probe(&probed).await.unwrap();
        assert!(
            matches!(outcome, ProbeOutcome::Timeout | ProbeOutcome::Unreachable),
            "expected a measurement, got {outcome:?}"
        );
        #[cfg(windows)]
        assert_eq!(
            outcome,
            ProbeOutcome::Timeout,
            "Windows cannot report a refusal this quickly, so the deadline must win"
        );
    }
}
