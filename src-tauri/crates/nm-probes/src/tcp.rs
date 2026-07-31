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

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

use async_trait::async_trait;
use nm_core::sample::{ProbeOutcome, Rtt};
use tokio::net::TcpSocket;

use crate::probe::{ProbeKind, ProbeTarget, Prober};
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
        let Some(port) = target.address.port else {
            return Err(Error::PortRequired {
                kind: ProbeKind::TcpConnect,
            });
        };
        if let Some(source) = target.source {
            if !families_match(source, target.address.ip) {
                return Err(Error::SourceFamilyMismatch);
            }
        }

        let socket = open_socket(target.address.ip)?;
        if let Some(source) = target.source {
            // Port 0: the OS picks the local port, we only pin the interface.
            socket
                .bind(SocketAddr::new(source, 0))
                .map_err(|error| local(&error))?;
        }

        let destination = SocketAddr::new(target.address.ip, port);
        let started = Instant::now();
        match tokio::time::timeout(target.timeout, socket.connect(destination)).await {
            Ok(Ok(stream)) => {
                let elapsed = started.elapsed();
                drop(stream);
                Ok(ProbeOutcome::Success(Rtt::from_duration(elapsed)))
            }
            Ok(Err(error)) => classify_connect_error(error.kind()),
            Err(_elapsed) => Ok(ProbeOutcome::Timeout),
        }
    }
}

/// Opens a socket of the right family, configured for repeated short-lived probes.
fn open_socket(target: IpAddr) -> Result<TcpSocket, Error> {
    let socket = match target {
        IpAddr::V4(_) => TcpSocket::new_v4(),
        IpAddr::V6(_) => TcpSocket::new_v6(),
    }
    .map_err(|error| local(&error))?;

    // Close with a reset rather than a graceful shutdown, so a probe socket never enters
    // TIME_WAIT. That state lasts minutes, and at the global cap of 32 probes per second it
    // would park thousands of ephemeral ports at once against a Windows dynamic range of
    // ~16k — the app would eventually be unable to open a socket at all, and so would every
    // other program on the machine. The peer sees an abortive close on a connection that
    // carried no application data, which costs it nothing.
    socket.set_zero_linger().map_err(|error| local(&error))?;
    Ok(socket)
}

/// Whether a source address can be bound to reach a target address.
const fn families_match(source: IpAddr, target: IpAddr) -> bool {
    matches!(
        (source, target),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

/// Decides what a failed `connect` says about the destination.
///
/// The split that matters is between a failure the *network* reported and a failure our own
/// machine produced. Only the first is a measurement; the second must surface as an error,
/// because a socket we could not open is not a packet someone else dropped. Anything the OS
/// did not classify falls to the cautious side.
fn classify_connect_error(kind: io::ErrorKind) -> Result<ProbeOutcome, Error> {
    match kind {
        // The destination answered, and its answer is "no". The path carries packets, so
        // this is not loss — and the round trip is discarded rather than reported, because a
        // middlebox forging a reset would make it a local number wearing a remote label.
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::HostUnreachable
        | io::ErrorKind::NetworkUnreachable => Ok(ProbeOutcome::Unreachable),
        // The stack gave up before our own deadline: nothing answered.
        io::ErrorKind::TimedOut => Ok(ProbeOutcome::Timeout),
        // A local firewall stopped the probe leaving. This probe kind is unusable here and
        // the sample carries no information about the link — exactly what `Blocked` means.
        io::ErrorKind::PermissionDenied => Ok(ProbeOutcome::Blocked),
        other => Err(Error::LocalFailure {
            kind: ProbeKind::TcpConnect,
            reason: other,
        }),
    }
}

/// Wraps an OS failure that measured nothing about the target.
fn local(error: &io::Error) -> Error {
    Error::LocalFailure {
        kind: ProbeKind::TcpConnect,
        reason: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
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

    #[test]
    fn source_binding_requires_a_matching_family() {
        let v4 = LOOPBACK;
        let v6 = IpAddr::V6(std::net::Ipv6Addr::LOCALHOST);
        assert!(families_match(v4, v4));
        assert!(families_match(v6, v6));
        assert!(!families_match(v4, v6));
        assert!(!families_match(v6, v4));
    }

    #[test]
    fn network_reported_failures_become_measurements() {
        for kind in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkUnreachable,
        ] {
            assert_eq!(
                classify_connect_error(kind).unwrap(),
                ProbeOutcome::Unreachable,
                "{kind:?}"
            );
        }
        assert_eq!(
            classify_connect_error(io::ErrorKind::TimedOut).unwrap(),
            ProbeOutcome::Timeout
        );
    }

    #[test]
    fn a_local_firewall_reads_as_a_filtered_probe_kind() {
        assert_eq!(
            classify_connect_error(io::ErrorKind::PermissionDenied).unwrap(),
            ProbeOutcome::Blocked,
            "a probe that never left the machine measures nothing and is not loss"
        );
    }

    #[test]
    fn unclassified_failures_stay_errors() {
        // The cautious side of the split: none of these prove anything about the
        // destination, so none may be reported as a dropped packet.
        for kind in [
            io::ErrorKind::AddrNotAvailable,
            io::ErrorKind::AddrInUse,
            io::ErrorKind::InvalidInput,
            io::ErrorKind::OutOfMemory,
            io::ErrorKind::Other,
        ] {
            assert_eq!(
                classify_connect_error(kind).unwrap_err(),
                Error::LocalFailure {
                    kind: ProbeKind::TcpConnect,
                    reason: kind
                },
                "{kind:?}"
            );
        }
    }
}
