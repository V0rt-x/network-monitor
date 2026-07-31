//! Connection tables on Windows, via the IP Helper API.
//!
//! `GetExtendedTcpTable` and `GetExtendedUdpTable` in their `_OWNER_PID` forms are the
//! documented, unprivileged way to learn which process owns which socket — the same
//! information `netstat -b` prints, obtained the same way, with no driver and no
//! elevation.
//!
//! Four tables are read per poll: TCP and UDP, each over IPv4 and IPv6. A build that
//! skipped IPv6 would quietly lose every endpoint of a game on a v6-capable connection,
//! which is a growing share of them.
//!
//! The shape of the reply is the awkward part. Each table is a count followed by a
//! variable-length row array declared in the headers as a one-element array, so the rows
//! have to be walked by hand. Every read below goes through `read_unaligned` and is
//! bounded by the byte count the API itself reported, never by the row count it claims:
//! trusting `dwNumEntries` over the buffer length would turn a short reply into a read
//! past the end of our allocation.

use std::mem::offset_of;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::{ptr, slice};

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, FALSE, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
    MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDP6TABLE_OWNER_PID,
    MIB_UDPROW_OWNER_PID, MIB_UDPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

use super::{
    classify_tcp_state, peer, port_from_mib, Connection, ConnectionTable, Protocol, TcpState,
};
use crate::icmp::from_in_addr;
use crate::process::Pid;
use crate::Error;

/// Initial query buffer, in 32-bit words.
///
/// 64 KiB, which holds roughly a thousand TCP rows — more than a desktop normally has
/// open, so the steady state is one call per table and no reallocation for the life of
/// the session.
const INITIAL_CAPACITY_WORDS: usize = 16 * 1024;

/// How many times a query may be retried after being told the buffer is too small.
///
/// One retry is the normal case (the buffer really was too small). More than that means
/// the table is growing between the size query and the fetch, and a few extra attempts
/// with slack added each time settle it. A bound is needed because a machine opening
/// sockets fast enough could otherwise keep this loop running forever.
const MAX_ATTEMPTS: u32 = 6;

/// Extra room added on each retry, as a fraction of the size the API asked for.
///
/// The size it reports describes the table as it was a moment ago; padding it means the
/// next attempt still fits if a few more sockets opened in between.
const GROWTH_SLACK_PERCENT: u32 = 25;

/// Which table to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Table {
    Tcp,
    Udp,
}

impl Table {
    /// Name of the API this table comes from, for error reporting.
    const fn api(self) -> &'static str {
        match self {
            Self::Tcp => "GetExtendedTcpTable",
            Self::Udp => "GetExtendedUdpTable",
        }
    }
}

/// Reads the Windows connection tables.
///
/// Owns its query buffer so that polling — once a second, for hours — costs no
/// allocation once the buffer has grown to fit the machine.
#[derive(Debug)]
pub struct WindowsConnectionTable {
    /// Query buffer, held as words so its allocation is aligned for the row structures
    /// the API writes into it. Every row type in these tables is a group of `u32`s and
    /// byte arrays, so four-byte alignment is what they require.
    buffer: Vec<u32>,
}

impl Default for WindowsConnectionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsConnectionTable {
    /// Creates a table reader with a buffer sized for a typical machine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: vec![0; INITIAL_CAPACITY_WORDS],
        }
    }

    /// Fetches one table into the buffer and returns how many bytes it holds.
    fn query(&mut self, table: Table, family: u32) -> Result<usize, Error> {
        for _ in 0..MAX_ATTEMPTS {
            let capacity = self.buffer.len().saturating_mul(size_of::<u32>());
            let mut size = u32::try_from(capacity).unwrap_or(u32::MAX);
            let pointer = self.buffer.as_mut_ptr().cast();

            // SAFETY: `pointer` addresses `size` bytes of live, correctly aligned
            // storage owned by `self.buffer`, and `size` is the capacity the API is told
            // it has. The API writes at most that many bytes and reports the number it
            // used — or, when the buffer is too small, the number it needs — back
            // through the same variable. `sort` is FALSE, so no ordering work is done.
            let status = unsafe {
                match table {
                    Table::Tcp => GetExtendedTcpTable(
                        pointer,
                        &raw mut size,
                        FALSE,
                        family,
                        TCP_TABLE_OWNER_PID_ALL,
                        0,
                    ),
                    Table::Udp => GetExtendedUdpTable(
                        pointer,
                        &raw mut size,
                        FALSE,
                        family,
                        UDP_TABLE_OWNER_PID,
                        0,
                    ),
                }
            };

            if status == NO_ERROR {
                let used = usize::try_from(size).unwrap_or(0);
                return Ok(used.min(capacity));
            }
            if status != ERROR_INSUFFICIENT_BUFFER {
                return Err(Error::Os {
                    api: table.api(),
                    code: status,
                });
            }

            let wanted = size.saturating_add(size / 100 * GROWTH_SLACK_PERCENT);
            let words = usize::try_from(wanted).unwrap_or(usize::MAX) / size_of::<u32>() + 1;
            // Never shrink: a smaller request after a larger one would mean re-growing on
            // the next poll, and the buffer is the thing that keeps polling free of
            // allocation.
            if words > self.buffer.len() {
                self.buffer.resize(words, 0);
            } else {
                self.buffer.resize(self.buffer.len().saturating_mul(2), 0);
            }
        }

        Err(Error::Os {
            api: table.api(),
            code: ERROR_INSUFFICIENT_BUFFER,
        })
    }

    /// The bytes of the last query, as the API filled them in.
    fn filled(&self, len: usize) -> &[u8] {
        let capacity = self.buffer.len().saturating_mul(size_of::<u32>());
        let len = len.min(capacity);
        // SAFETY: `self.buffer` owns at least `capacity` initialised bytes, `len` does
        // not exceed it, and `u8` has weaker alignment than `u32`, so the whole range is
        // a valid `u8` slice for as long as the borrow of `self` lasts.
        unsafe { slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), len) }
    }
}

/// Reads the row count a table header declares.
fn row_count(bytes: &[u8]) -> usize {
    if bytes.len() < size_of::<u32>() {
        return 0;
    }
    // SAFETY: the length check above guarantees four readable bytes at the start of the
    // slice; `read_unaligned` assumes no alignment beyond that.
    let count = unsafe { ptr::read_unaligned(bytes.as_ptr().cast::<u32>()) };
    usize::try_from(count).unwrap_or(0)
}

/// Walks `count` rows of type `R` beginning at `offset`.
///
/// The count is clamped to what the buffer can actually hold, so a reply that is shorter
/// than its own header claims yields fewer rows instead of reading past the allocation.
fn rows<R: Copy>(bytes: &[u8], offset: usize, count: usize) -> impl Iterator<Item = R> + '_ {
    let available = bytes.len().saturating_sub(offset) / size_of::<R>();
    (0..count.min(available)).map(move |index| {
        // SAFETY: `index < available`, so `offset + (index + 1) * size_of::<R>()` is
        // within `bytes`, which is initialised storage owned by the caller.
        // `read_unaligned` makes no alignment assumption, and `R` is `Copy` and
        // inhabited by any bit pattern the API can write into these tables.
        unsafe { ptr::read_unaligned(bytes.as_ptr().add(offset + index * size_of::<R>()).cast()) }
    })
}

/// Builds an IPv6 address and its scope from a table row's fields.
fn ipv6_socket(address: [u8; 16], scope_id: u32, port: u16) -> SocketAddr {
    SocketAddr::V6(SocketAddrV6::new(
        Ipv6Addr::from(address),
        port,
        0,
        scope_id,
    ))
}

impl ConnectionTable for WindowsConnectionTable {
    fn snapshot(&mut self, out: &mut Vec<Connection>) -> Result<(), Error> {
        out.clear();

        let len = self.query(Table::Tcp, u32::from(AF_INET))?;
        let bytes = self.filled(len);
        let count = row_count(bytes);
        let offset = offset_of!(MIB_TCPTABLE_OWNER_PID, table);
        out.extend(
            rows::<MIB_TCPROW_OWNER_PID>(bytes, offset, count).map(|row| {
                let state = classify_tcp_state(row.dwState);
                Connection {
                    protocol: Protocol::Tcp,
                    local: SocketAddr::new(
                        IpAddr::V4(from_in_addr(row.dwLocalAddr)),
                        port_from_mib(row.dwLocalPort),
                    ),
                    remote: tcp_peer(
                        state,
                        IpAddr::V4(from_in_addr(row.dwRemoteAddr)),
                        port_from_mib(row.dwRemotePort),
                    ),
                    state: Some(state),
                    pid: Pid::new(row.dwOwningPid),
                }
            }),
        );

        let len = self.query(Table::Tcp, u32::from(AF_INET6))?;
        let bytes = self.filled(len);
        let count = row_count(bytes);
        let offset = offset_of!(MIB_TCP6TABLE_OWNER_PID, table);
        out.extend(
            rows::<MIB_TCP6ROW_OWNER_PID>(bytes, offset, count).map(|row| {
                let state = classify_tcp_state(row.dwState);
                Connection {
                    protocol: Protocol::Tcp,
                    local: ipv6_socket(
                        row.ucLocalAddr,
                        row.dwLocalScopeId,
                        port_from_mib(row.dwLocalPort),
                    ),
                    remote: tcp_peer(
                        state,
                        IpAddr::V6(Ipv6Addr::from(row.ucRemoteAddr)),
                        port_from_mib(row.dwRemotePort),
                    ),
                    state: Some(state),
                    pid: Pid::new(row.dwOwningPid),
                }
            }),
        );

        let len = self.query(Table::Udp, u32::from(AF_INET))?;
        let bytes = self.filled(len);
        let count = row_count(bytes);
        let offset = offset_of!(MIB_UDPTABLE_OWNER_PID, table);
        out.extend(
            rows::<MIB_UDPROW_OWNER_PID>(bytes, offset, count).map(|row| Connection {
                protocol: Protocol::Udp,
                local: SocketAddr::new(
                    IpAddr::V4(from_in_addr(row.dwLocalAddr)),
                    port_from_mib(row.dwLocalPort),
                ),
                // The kernel knows of no peer for a datagram socket, so there is nothing
                // honest to put here. ETW flow events fill this gap.
                remote: None,
                state: None,
                pid: Pid::new(row.dwOwningPid),
            }),
        );

        let len = self.query(Table::Udp, u32::from(AF_INET6))?;
        let bytes = self.filled(len);
        let count = row_count(bytes);
        let offset = offset_of!(MIB_UDP6TABLE_OWNER_PID, table);
        out.extend(
            rows::<MIB_UDP6ROW_OWNER_PID>(bytes, offset, count).map(|row| Connection {
                protocol: Protocol::Udp,
                local: ipv6_socket(
                    row.ucLocalAddr,
                    row.dwLocalScopeId,
                    port_from_mib(row.dwLocalPort),
                ),
                remote: None,
                state: None,
                pid: Pid::new(row.dwOwningPid),
            }),
        );

        Ok(())
    }
}

/// The peer of a TCP row, or [`None`] when the state means the fields are meaningless.
///
/// A listening socket's remote fields are not merely zero — on some Windows versions they
/// hold leftovers. Gating on the state as well as on the address is what stops a stale
/// tuple from being handed to the probe engine as a live endpoint.
fn tcp_peer(state: TcpState, address: IpAddr, port: u16) -> Option<SocketAddr> {
    if matches!(state, TcpState::Listen | TcpState::Closed) {
        return None;
    }
    peer(address, port)
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket};

    use super::*;

    /// Every socket these tests open stays on the loopback interface: the suite must
    /// measure this machine's own tables and never put a packet on a network.
    const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

    fn snapshot() -> Vec<Connection> {
        let mut table = WindowsConnectionTable::new();
        let mut rows = Vec::new();
        table
            .snapshot(&mut rows)
            .expect("the connection tables are readable without elevation");
        rows
    }

    fn me() -> Pid {
        Pid::new(std::process::id())
    }

    #[test]
    fn a_listening_socket_appears_with_our_pid_and_no_peer() {
        let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let row = snapshot()
            .into_iter()
            .find(|row| {
                row.pid == me() && row.protocol == Protocol::Tcp && row.local.port() == port
            })
            .expect("a socket this process just bound must be in the table");

        assert_eq!(row.state, Some(TcpState::Listen));
        assert_eq!(row.remote, None);
        assert_eq!(row.active_peer(), None);
        assert_eq!(row.local.ip(), IpAddr::V4(LOOPBACK));
    }

    #[test]
    fn an_established_connection_reports_both_ends() {
        let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
        let server_port = listener.local_addr().unwrap().port();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let _accepted = listener.accept().unwrap();
        let client_port = client.local_addr().unwrap().port();

        let row = snapshot()
            .into_iter()
            .find(|row| {
                row.pid == me() && row.protocol == Protocol::Tcp && row.local.port() == client_port
            })
            .expect("the client half of a live connection must be in the table");

        assert_eq!(row.state, Some(TcpState::Established));
        // The port is the real assertion: it proves the network-byte-order handling,
        // which is the one thing in this file that fails silently when it is wrong.
        assert_eq!(
            row.active_peer(),
            Some(SocketAddr::new(IpAddr::V4(LOOPBACK), server_port))
        );
    }

    #[test]
    fn a_udp_socket_appears_without_a_peer_even_once_connected() {
        // Connecting a datagram socket sets a default destination in *our* stack; the
        // table still has nowhere to report it. This is the gap ETW exists to close, and
        // asserting it here stops anyone from later "fixing" the None into a guess.
        let socket = UdpSocket::bind((LOOPBACK, 0)).unwrap();
        let port = socket.local_addr().unwrap().port();
        let peer = UdpSocket::bind((LOOPBACK, 0)).unwrap();
        socket.connect(peer.local_addr().unwrap()).unwrap();

        let row = snapshot()
            .into_iter()
            .find(|row| {
                row.pid == me() && row.protocol == Protocol::Udp && row.local.port() == port
            })
            .expect("a bound UDP socket must be in the table");

        assert_eq!(row.remote, None);
        assert_eq!(row.state, None);
    }

    #[test]
    fn the_snapshot_covers_both_address_families() {
        let rows = snapshot();

        assert!(rows.len() > 10, "{} rows", rows.len());
        assert!(
            rows.iter().any(|row| row.local.is_ipv4()),
            "no IPv4 rows in the table"
        );
        // Windows binds v6 sockets for its own services on any modern install; a build
        // that silently returned only v4 would lose a game's endpoints on a v6 network.
        assert!(
            rows.iter().any(|row| row.local.is_ipv6()),
            "no IPv6 rows in the table"
        );
    }

    #[test]
    fn rows_are_self_consistent() {
        for row in snapshot() {
            match row.protocol {
                Protocol::Tcp => assert!(row.state.is_some(), "a TCP row must carry a state"),
                Protocol::Udp => {
                    assert!(row.state.is_none(), "UDP has no state");
                    assert!(row.remote.is_none(), "UDP rows cannot name a peer");
                }
            }
            if let Some(remote) = row.remote {
                assert_ne!(remote.port(), 0, "port 0 is not a peer");
            }
        }
    }

    #[test]
    fn polling_twice_reuses_the_buffer_and_replaces_the_rows() {
        let mut table = WindowsConnectionTable::new();
        let mut rows = Vec::new();

        table.snapshot(&mut rows).unwrap();
        let first = rows.len();
        table.snapshot(&mut rows).unwrap();

        // The second poll must have cleared the first, not appended to it — otherwise a
        // long session would grow an endpoint list without bound.
        assert!(rows.len() < first * 2, "{} rows after {first}", rows.len());
    }

    #[test]
    fn a_short_reply_yields_fewer_rows_rather_than_reading_past_it() {
        // The defence against a header that claims more rows than the API delivered.
        let bytes = [0u8; 8];
        assert_eq!(rows::<MIB_TCPROW_OWNER_PID>(&bytes, 4, 1_000).count(), 0);
        assert_eq!(rows::<MIB_UDPROW_OWNER_PID>(&bytes, 4, 1_000).count(), 0);
    }

    #[test]
    fn an_empty_reply_declares_no_rows() {
        assert_eq!(row_count(&[]), 0);
        assert_eq!(row_count(&[0, 0]), 0);
        assert_eq!(row_count(&[7, 0, 0, 0]), 7);
    }

    #[test]
    fn a_listening_row_never_yields_a_stale_peer() {
        let address = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9));
        assert_eq!(tcp_peer(TcpState::Listen, address, 443), None);
        assert_eq!(tcp_peer(TcpState::Closed, address, 443), None);
        assert_eq!(
            tcp_peer(TcpState::Established, address, 443),
            Some(SocketAddr::new(address, 443))
        );
    }

    /// The transcribed `MIB_TCP_STATE` values, checked against the real headers so a typo
    /// cannot turn a live connection into an unknown state.
    #[test]
    fn tcp_state_constants_match_the_windows_headers() {
        use super::super::tcp_state;
        use windows_sys::Win32::NetworkManagement::IpHelper as sys;

        for (ours, theirs) in [
            (tcp_state::CLOSED, sys::MIB_TCP_STATE_CLOSED),
            (tcp_state::LISTEN, sys::MIB_TCP_STATE_LISTEN),
            (tcp_state::SYN_SENT, sys::MIB_TCP_STATE_SYN_SENT),
            (tcp_state::SYN_RCVD, sys::MIB_TCP_STATE_SYN_RCVD),
            (tcp_state::ESTAB, sys::MIB_TCP_STATE_ESTAB),
            (tcp_state::FIN_WAIT1, sys::MIB_TCP_STATE_FIN_WAIT1),
            (tcp_state::FIN_WAIT2, sys::MIB_TCP_STATE_FIN_WAIT2),
            (tcp_state::CLOSE_WAIT, sys::MIB_TCP_STATE_CLOSE_WAIT),
            (tcp_state::CLOSING, sys::MIB_TCP_STATE_CLOSING),
            (tcp_state::LAST_ACK, sys::MIB_TCP_STATE_LAST_ACK),
            (tcp_state::TIME_WAIT, sys::MIB_TCP_STATE_TIME_WAIT),
            (tcp_state::DELETE_TCB, sys::MIB_TCP_STATE_DELETE_TCB),
        ] {
            assert_eq!(u32::try_from(theirs), Ok(ours));
        }
    }

    /// `MIB_TCP_STATE_RESERVED` has no meaning for us, and must stay unclassified rather
    /// than colliding with a real state.
    #[test]
    fn the_reserved_state_stays_unknown() {
        use windows_sys::Win32::NetworkManagement::IpHelper as sys;

        let reserved = u32::try_from(sys::MIB_TCP_STATE_RESERVED).unwrap();
        assert_eq!(classify_tcp_state(reserved), TcpState::Unknown(reserved));
    }
}
