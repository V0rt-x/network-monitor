//! The kernel's connection tables, attributed to the process that owns each socket.
//!
//! This is passive *discovery*, and only discovery: the tables say which endpoints a
//! process has sockets for, never what it sent to them. Measurement stays the job of our
//! own probes, which is what keeps the product free of a packet-capture driver.
//!
//! **What a table can and cannot tell us.** A TCP row carries both ends of the
//! connection, so a game's TCP endpoints are visible from a poll alone. A UDP row carries
//! only the *local* socket — UDP is connectionless, so the kernel has no peer to report
//! even when a socket has been `connect`ed. Everything a competitive game actually plays
//! over is UDP, which is why polling is only half the story and the ETW flow source is
//! the other half. Reporting a UDP row with [`Connection::remote`] set to [`None`] is the
//! honest form of that gap; inventing a peer from a listening port would not be.
//!
//! Windows implements this with `GetExtendedTcpTable`/`GetExtendedUdpTable`. Linux reads
//! the same information from `sock_diag` netlink (with `/proc/net/*` as the fallback) and
//! macOS from `proc_pidfdinfo`; in all three the shape above holds.

use std::net::{IpAddr, SocketAddr};

use crate::process::Pid;
use crate::Error;

#[cfg(windows)]
pub mod windows;

/// Transport protocol of a connection-table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// TCP — the row names both ends of the connection.
    Tcp,
    /// UDP — the row names only the local socket.
    Udp,
}

/// TCP connection state, as the OS reports it.
///
/// Kept whole rather than reduced to "established or not": a socket stuck in `SynSent` is
/// a connection attempt that is getting no answer, which is a symptom worth showing, and
/// `TimeWait` rows are corpses that must not be probed as if they were live endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TcpState {
    /// No connection.
    Closed,
    /// Waiting for an inbound connection.
    Listen,
    /// A connection request was sent and is unanswered so far.
    SynSent,
    /// A connection request arrived and was answered.
    SynReceived,
    /// The connection is open and carrying data.
    Established,
    /// The local end closed and is waiting for the peer to acknowledge.
    FinWait1,
    /// The local close is acknowledged; waiting for the peer to close.
    FinWait2,
    /// The peer closed; the local end has not yet.
    CloseWait,
    /// Both ends closed simultaneously.
    Closing,
    /// Waiting for the final acknowledgement.
    LastAck,
    /// Holding the tuple aside so late packets cannot join a new connection.
    TimeWait,
    /// The kernel is tearing the control block down.
    DeleteTcb,
    /// A value this build does not know; carried through rather than guessed at.
    Unknown(u32),
}

impl TcpState {
    /// Whether the row describes a connection that currently has a live peer.
    ///
    /// The filter for "is this an endpoint worth probing": a listening socket has no
    /// peer, and the closing states describe one that is going away, so probing either
    /// would spend budget on an address the application is no longer talking to.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::SynSent | Self::SynReceived | Self::Established | Self::CloseWait
        )
    }
}

/// One row of a connection table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// Transport protocol.
    pub protocol: Protocol,
    /// The local socket address.
    pub local: SocketAddr,
    /// The peer, when the row has one.
    ///
    /// Always [`None`] for UDP, and for TCP rows with no peer yet (a listening socket).
    pub remote: Option<SocketAddr>,
    /// TCP state; [`None`] for UDP, which has none.
    pub state: Option<TcpState>,
    /// The process that owns the socket.
    pub pid: Pid,
}

impl Connection {
    /// The peer address if this row describes a flow worth measuring.
    ///
    /// Combines the two conditions callers otherwise get wrong separately: the row must
    /// name a peer, and — for TCP — the connection must still be live. A loopback or
    /// link-local peer is left in; deciding that a LAN endpoint is uninteresting belongs
    /// to the layer that knows what the user asked for, not here.
    #[must_use]
    pub fn active_peer(&self) -> Option<SocketAddr> {
        match self.state {
            Some(state) if !state.is_active() => None,
            _ => self.remote,
        }
    }
}

/// Reads the connection tables of the running system.
///
/// Takes `&mut self` so an implementation can keep its query buffer between calls: this
/// is polled about once a second for the life of a session, and a table of several
/// hundred rows would otherwise mean a fresh multi-kilobyte allocation every tick.
pub trait ConnectionTable: Send {
    /// Replaces `out` with the current contents of the TCP and UDP tables.
    ///
    /// `out` is cleared first; passing the same vector back on every poll is the intended
    /// use and keeps the steady state allocation-free.
    ///
    /// # Errors
    ///
    /// Returns an error when the OS refuses to hand over a table. A partial read is never
    /// reported as success — an endpoint list that silently lost half its rows would look
    /// exactly like an application that stopped talking.
    fn snapshot(&mut self, out: &mut Vec<Connection>) -> Result<(), Error>;
}

impl<T: ConnectionTable + ?Sized> ConnectionTable for Box<T> {
    fn snapshot(&mut self, out: &mut Vec<Connection>) -> Result<(), Error> {
        (**self).snapshot(out)
    }
}

/// The host's connection table, if this build has one.
///
/// # Errors
///
/// Returns [`Error::UnsupportedPlatform`] where no backend exists yet.
pub fn system_table() -> Result<Box<dyn ConnectionTable>, Error> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsConnectionTable::new()))
    }
    #[cfg(not(windows))]
    {
        Err(Error::UnsupportedPlatform)
    }
}

/// `MIB_TCP_STATE` values from the Windows IP Helper API (`tcpmib.h`).
///
/// Declared here rather than only in the Windows module so the classification below is
/// testable on any host; a `#[cfg(windows)]` test asserts every one against what
/// `windows-sys` declares, so the transcription cannot drift from the real headers.
pub(crate) mod tcp_state {
    /// No connection.
    pub const CLOSED: u32 = 1;
    /// Listening for inbound connections.
    pub const LISTEN: u32 = 2;
    /// A connection request has been sent.
    pub const SYN_SENT: u32 = 3;
    /// A connection request has been received.
    pub const SYN_RCVD: u32 = 4;
    /// The connection is open.
    pub const ESTAB: u32 = 5;
    /// The local end has closed.
    pub const FIN_WAIT1: u32 = 6;
    /// The local close has been acknowledged.
    pub const FIN_WAIT2: u32 = 7;
    /// The remote end has closed.
    pub const CLOSE_WAIT: u32 = 8;
    /// Both ends closed at once.
    pub const CLOSING: u32 = 9;
    /// Waiting for the final acknowledgement.
    pub const LAST_ACK: u32 = 10;
    /// Holding the tuple aside after close.
    pub const TIME_WAIT: u32 = 11;
    /// The control block is being deleted.
    pub const DELETE_TCB: u32 = 12;
}

/// Maps a raw `MIB_TCP_STATE` onto [`TcpState`].
pub(crate) const fn classify_tcp_state(raw: u32) -> TcpState {
    match raw {
        tcp_state::CLOSED => TcpState::Closed,
        tcp_state::LISTEN => TcpState::Listen,
        tcp_state::SYN_SENT => TcpState::SynSent,
        tcp_state::SYN_RCVD => TcpState::SynReceived,
        tcp_state::ESTAB => TcpState::Established,
        tcp_state::FIN_WAIT1 => TcpState::FinWait1,
        tcp_state::FIN_WAIT2 => TcpState::FinWait2,
        tcp_state::CLOSE_WAIT => TcpState::CloseWait,
        tcp_state::CLOSING => TcpState::Closing,
        tcp_state::LAST_ACK => TcpState::LastAck,
        tcp_state::TIME_WAIT => TcpState::TimeWait,
        tcp_state::DELETE_TCB => TcpState::DeleteTcb,
        other => TcpState::Unknown(other),
    }
}

/// Extracts a port from an IP Helper table field.
///
/// The API documents these fields as a `DWORD` holding the port *in network byte order in
/// its low-order two bytes* — so the value 0x5000 is port 80, not port 20480. Taking the
/// low 16 bits directly is the classic way to get this wrong, and the failure mode is
/// silent: every port is byte-swapped, every probe goes somewhere plausible and nothing
/// ever answers.
///
/// `to_le_bytes` and `from_be_bytes` are operations on the *value*, not on this machine's
/// memory layout, so the swap is correct on any host and needs no cast.
pub(crate) const fn port_from_mib(raw: u32) -> u16 {
    let [low_order, next, _, _] = raw.to_le_bytes();
    u16::from_be_bytes([low_order, next])
}

/// The peer of a table row, or [`None`] when the row does not name one.
///
/// A row with no peer reports the wildcard address and port 0 rather than omitting the
/// fields, so this is where "0.0.0.0:0" stops being mistaken for an endpoint. Port 0 is
/// rejected on its own too: it is not a destination anything can be sent to, so a probe
/// aimed at it would measure nothing.
pub(crate) fn peer(address: IpAddr, port: u16) -> Option<SocketAddr> {
    if port == 0 || address.is_unspecified() {
        return None;
    }
    Some(SocketAddr::new(address, port))
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn ports_are_read_out_of_network_byte_order() {
        // Port 80 sits in the DWORD as 0x00,0x50 — which reads as 0x5000.
        assert_eq!(port_from_mib(0x5000), 80);
        // 27015 (0x6987, the Source-engine port) is stored as 0x69,0x87 and reads as
        // 0x8769.
        assert_eq!(port_from_mib(0x8769), 27_015);
        assert_eq!(port_from_mib(0), 0);
        // 65535 is a palindrome under byte swapping and would hide a mistake on its own.
        assert_eq!(port_from_mib(0xFFFF), 65_535);
    }

    #[test]
    fn ports_ignore_the_upper_half_of_the_field() {
        // Only the low-order two bytes are meaningful; the rest is undefined padding and
        // must not leak into the port.
        assert_eq!(port_from_mib(0xDEAD_5000), 80);
    }

    #[test]
    fn every_documented_tcp_state_is_classified() {
        for (raw, expected) in [
            (tcp_state::CLOSED, TcpState::Closed),
            (tcp_state::LISTEN, TcpState::Listen),
            (tcp_state::SYN_SENT, TcpState::SynSent),
            (tcp_state::SYN_RCVD, TcpState::SynReceived),
            (tcp_state::ESTAB, TcpState::Established),
            (tcp_state::FIN_WAIT1, TcpState::FinWait1),
            (tcp_state::FIN_WAIT2, TcpState::FinWait2),
            (tcp_state::CLOSE_WAIT, TcpState::CloseWait),
            (tcp_state::CLOSING, TcpState::Closing),
            (tcp_state::LAST_ACK, TcpState::LastAck),
            (tcp_state::TIME_WAIT, TcpState::TimeWait),
            (tcp_state::DELETE_TCB, TcpState::DeleteTcb),
        ] {
            assert_eq!(classify_tcp_state(raw), expected, "state {raw}");
        }
    }

    #[test]
    fn an_unknown_state_keeps_its_value() {
        // `MIB_TCP_STATE_RESERVED` (100) and anything a later Windows adds must survive
        // the trip rather than being flattened into a state we would then act on.
        assert_eq!(classify_tcp_state(100), TcpState::Unknown(100));
        assert_eq!(classify_tcp_state(0), TcpState::Unknown(0));
    }

    #[test]
    fn only_live_states_count_as_active() {
        for state in [
            TcpState::SynSent,
            TcpState::SynReceived,
            TcpState::Established,
            TcpState::CloseWait,
        ] {
            assert!(state.is_active(), "{state:?}");
        }
        for state in [
            TcpState::Closed,
            TcpState::Listen,
            TcpState::FinWait1,
            TcpState::FinWait2,
            TcpState::Closing,
            TcpState::LastAck,
            TcpState::TimeWait,
            TcpState::DeleteTcb,
            TcpState::Unknown(100),
        ] {
            assert!(!state.is_active(), "{state:?}");
        }
    }

    #[test]
    fn a_wildcard_row_names_no_peer() {
        assert_eq!(peer(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0), None);
        assert_eq!(peer(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 443), None);
        assert_eq!(peer(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 443), None);
    }

    #[test]
    fn port_zero_is_never_a_peer() {
        assert_eq!(peer(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)), 0), None);
    }

    #[test]
    fn a_real_peer_survives() {
        let address = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5));
        assert_eq!(
            peer(address, 27_015),
            Some(SocketAddr::new(address, 27_015))
        );
    }

    fn tcp_row(state: TcpState, remote: Option<SocketAddr>) -> Connection {
        Connection {
            protocol: Protocol::Tcp,
            local: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), 51_000),
            remote,
            state: Some(state),
            pid: Pid::new(4242),
        }
    }

    #[test]
    fn an_established_row_offers_its_peer() {
        let remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)), 443);
        assert_eq!(
            tcp_row(TcpState::Established, Some(remote)).active_peer(),
            Some(remote)
        );
    }

    #[test]
    fn a_dying_connection_is_not_an_endpoint_to_probe() {
        // The tuple is still in the table, but the application has stopped using it;
        // probing it would spend budget measuring a path nobody is on.
        let remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)), 443);
        assert_eq!(
            tcp_row(TcpState::TimeWait, Some(remote)).active_peer(),
            None
        );
        assert_eq!(tcp_row(TcpState::Listen, None).active_peer(), None);
    }

    #[test]
    fn a_udp_row_never_claims_a_peer() {
        let row = Connection {
            protocol: Protocol::Udp,
            local: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 50_000),
            remote: None,
            state: None,
            pid: Pid::new(4242),
        };
        assert_eq!(row.active_peer(), None);
    }
}
