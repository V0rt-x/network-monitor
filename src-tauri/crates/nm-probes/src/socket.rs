//! TCP socket setup shared by every prober that opens a connection.
//!
//! Both the TCP-connect prober and the TLS prober need the same things: a socket of the
//! right family, bound to the egress address the monitored flow uses, closed in a way that
//! does not accumulate state on the machine. The rules for reading a failure are shared
//! too, and they are the part worth keeping in one place — a second copy would eventually
//! disagree with the first about whether a failure was ours or the network's.
//!
//! Nothing here times anything out. The deadline belongs to the prober, which knows how
//! many round trips its measurement takes.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use nm_core::sample::ProbeOutcome;
use tokio::net::{TcpSocket, TcpStream};

use crate::probe::{ProbeKind, ProbeTarget};
use crate::Error;

/// What trying to open a connection produced.
pub(crate) enum Connected {
    /// The peer accepted, after this long.
    Stream {
        /// The open connection.
        stream: TcpStream,
        /// Time from the first SYN to the peer's acknowledgement: one round trip.
        elapsed: Duration,
    },
    /// The attempt ended in a measurement rather than a connection.
    Settled(ProbeOutcome),
}

/// Opens a connection to `target`, egressing from its source address when one is given.
pub(crate) async fn connect(target: &ProbeTarget, kind: ProbeKind) -> Result<Connected, Error> {
    let Some(port) = target.address.port else {
        return Err(Error::PortRequired { kind });
    };
    if let Some(source) = target.source {
        if !families_match(source, target.address.ip) {
            return Err(Error::SourceFamilyMismatch);
        }
    }

    let socket = open_socket(target.address.ip, kind)?;
    if let Some(source) = target.source {
        // Port 0: the OS picks the local port, we only pin the interface.
        socket
            .bind(SocketAddr::new(source, 0))
            .map_err(|error| local(&error, kind))?;
    }

    let destination = SocketAddr::new(target.address.ip, port);
    let started = Instant::now();
    match socket.connect(destination).await {
        Ok(stream) => Ok(Connected::Stream {
            stream,
            elapsed: started.elapsed(),
        }),
        Err(error) => classify_connect_error(error.kind(), kind).map(Connected::Settled),
    }
}

/// Opens a socket of the right family, configured for repeated short-lived probes.
fn open_socket(target: IpAddr, kind: ProbeKind) -> Result<TcpSocket, Error> {
    let socket = match target {
        IpAddr::V4(_) => TcpSocket::new_v4(),
        IpAddr::V6(_) => TcpSocket::new_v6(),
    }
    .map_err(|error| local(&error, kind))?;

    // Close with a reset rather than a graceful shutdown, so a probe socket never enters
    // TIME_WAIT. That state lasts minutes, and at the global cap of 32 probes per second it
    // would park thousands of ephemeral ports at once against a Windows dynamic range of
    // ~16k — the app would eventually be unable to open a socket at all, and so would every
    // other program on the machine. The peer sees an abortive close on a connection that
    // carried no application data worth finishing, which costs it nothing.
    socket
        .set_zero_linger()
        .map_err(|error| local(&error, kind))?;
    Ok(socket)
}

/// Whether a source address can be bound to reach a target address.
pub(crate) const fn families_match(source: IpAddr, target: IpAddr) -> bool {
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
pub(crate) fn classify_connect_error(
    reason: io::ErrorKind,
    kind: ProbeKind,
) -> Result<ProbeOutcome, Error> {
    match reason {
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
            kind,
            reason: other,
        }),
    }
}

/// Wraps an OS failure that measured nothing about the target.
pub(crate) fn local(error: &io::Error, kind: ProbeKind) -> Error {
    Error::LocalFailure {
        kind,
        reason: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    const KIND: ProbeKind = ProbeKind::TcpConnect;

    #[test]
    fn source_binding_requires_a_matching_family() {
        let v4 = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let v6 = IpAddr::V6(std::net::Ipv6Addr::LOCALHOST);
        assert!(families_match(v4, v4));
        assert!(families_match(v6, v6));
        assert!(!families_match(v4, v6));
        assert!(!families_match(v6, v4));
    }

    #[test]
    fn network_reported_failures_become_measurements() {
        for reason in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkUnreachable,
        ] {
            assert_eq!(
                classify_connect_error(reason, KIND).unwrap(),
                ProbeOutcome::Unreachable,
                "{reason:?}"
            );
        }
        assert_eq!(
            classify_connect_error(io::ErrorKind::TimedOut, KIND).unwrap(),
            ProbeOutcome::Timeout
        );
    }

    #[test]
    fn a_local_firewall_reads_as_a_filtered_probe_kind() {
        assert_eq!(
            classify_connect_error(io::ErrorKind::PermissionDenied, KIND).unwrap(),
            ProbeOutcome::Blocked,
            "a probe that never left the machine measures nothing and is not loss"
        );
    }

    #[test]
    fn unclassified_failures_stay_errors() {
        // The cautious side of the split: none of these prove anything about the
        // destination, so none may be reported as a dropped packet.
        for reason in [
            io::ErrorKind::AddrNotAvailable,
            io::ErrorKind::AddrInUse,
            io::ErrorKind::InvalidInput,
            io::ErrorKind::OutOfMemory,
            io::ErrorKind::Other,
        ] {
            assert_eq!(
                classify_connect_error(reason, KIND).unwrap_err(),
                Error::LocalFailure { kind: KIND, reason },
                "{reason:?}"
            );
        }
    }

    #[test]
    fn the_failing_probe_kind_is_carried_into_the_error() {
        // The chain tries several kinds against one endpoint; an error that did not say
        // which one failed would be useless for deciding what to try next.
        assert_eq!(
            classify_connect_error(io::ErrorKind::InvalidInput, ProbeKind::TlsHello).unwrap_err(),
            Error::LocalFailure {
                kind: ProbeKind::TlsHello,
                reason: io::ErrorKind::InvalidInput
            }
        );
    }
}
