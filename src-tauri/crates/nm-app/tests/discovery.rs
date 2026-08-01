//! Turning what the operating system reports into endpoints worth measuring.
//!
//! In `tests/` rather than in the module because `nm-app`'s library sets `test = false` —
//! an in-crate harness cannot start on Windows (see `tests.manifest`).
//!
//! The discovery sources are passed into [`Discovery::start`] rather than looked up, so
//! everything here runs on any operating system against fakes. Addresses used as inputs
//! are well-known constants (`1.1.1.1`, the documented fake-IP range); nothing observed on
//! a real machine appears in this file.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use nm_app::discovery::{
    from_connection, from_flow, is_worth_tracking, Discovery, FlowStatus, Observation,
};
use nm_core::address::AddressPolicy;
use nm_core::endpoint::{EndpointKey, Transport};
use nm_platform::connection::{Connection, ConnectionTable, Protocol, TcpState};
use nm_platform::flow::{FlowDirection, FlowEvent, FlowEventSource, FlowSink};
use nm_platform::process::Pid;

const GAME: Pid = Pid::new(4242);
const OTHER: Pid = Pid::new(77);

/// A public address, standing in for a game server. A well-known constant, never an
/// observation.
fn server(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), port)
}

/// The application's own socket.
fn client(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), port)
}

fn tcp_row(pid: Pid, state: TcpState, remote: Option<SocketAddr>) -> Connection {
    Connection {
        protocol: Protocol::Tcp,
        local: client(51_000),
        remote,
        state: Some(state),
        pid,
    }
}

fn udp_flow(pid: Pid, remote: SocketAddr, bytes: u64) -> FlowEvent {
    FlowEvent {
        pid,
        protocol: Protocol::Udp,
        local: client(50_000),
        remote,
        direction: FlowDirection::Sent,
        bytes,
        observed_at: Duration::from_secs(1),
    }
}

// ---------------------------------------------------------------- translation

#[test]
fn an_established_connection_becomes_an_endpoint() {
    let observation = from_connection(&tcp_row(GAME, TcpState::Established, Some(server(443))))
        .expect("an established row names a peer");

    assert_eq!(observation.pid, GAME);
    assert_eq!(observation.endpoint.transport, Transport::Tcp);
    assert_eq!(observation.endpoint.address, server(443));
    assert_eq!(
        observation.source,
        Some(client(51_000).ip()),
        "a probe must egress the same way the application's flow does"
    );
    assert_eq!(
        observation.bytes(),
        None,
        "a table row says a socket exists, never how busy it is"
    );
}

#[test]
fn a_connection_with_no_live_peer_is_not_an_endpoint() {
    // A listening socket, and one being torn down: probing either would spend budget on a
    // path the application is no longer on.
    assert_eq!(
        from_connection(&tcp_row(GAME, TcpState::Listen, None)),
        None
    );
    assert_eq!(
        from_connection(&tcp_row(GAME, TcpState::TimeWait, Some(server(443)))),
        None
    );
}

#[test]
fn a_udp_table_row_yields_nothing() {
    // The gap the flow source exists to fill: UDP is connectionless, so the kernel has no
    // peer to report and the table alone can never discover what a game plays over.
    let row = Connection {
        protocol: Protocol::Udp,
        local: client(50_000),
        remote: None,
        state: None,
        pid: GAME,
    };
    assert_eq!(from_connection(&row), None);
}

#[test]
fn an_unbound_socket_offers_no_egress_address() {
    let mut row = tcp_row(GAME, TcpState::Established, Some(server(443)));
    row.local = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 51_000);

    let observation = from_connection(&row).expect("the peer is still an endpoint");
    assert_eq!(
        observation.source, None,
        "binding a probe to the wildcard would let the OS choose a route the app may not use"
    );
}

#[test]
fn a_flow_event_carries_its_byte_count() {
    let observation =
        from_flow(&udp_flow(GAME, server(27_015), 1_280)).expect("a flow names its peer");

    assert_eq!(observation.pid, GAME);
    assert_eq!(observation.endpoint, EndpointKey::udp(server(27_015)));
    assert_eq!(observation.source, Some(client(50_000).ip()));
    assert_eq!(
        observation.bytes(),
        Some(1_280),
        "ranking by recent traffic is the whole reason flow events are worth their cost"
    );
}

#[test]
fn a_flow_with_no_real_peer_is_discarded() {
    // A send that has not bound yet reports the wildcard; it is not an endpoint.
    let wildcard = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
    assert_eq!(from_flow(&udp_flow(GAME, wildcard, 64)), None);
    assert_eq!(from_flow(&udp_flow(GAME, server(0), 64)), None);
}

#[test]
fn an_observation_names_a_process_and_leaves_the_application_to_be_resolved() {
    // Two sources on two threads report process identifiers, which is all the operating
    // system knows. Which application one belongs to is a question whose answer changes
    // while an observation is in flight — an anti-cheat re-launch is a new process of the
    // same application — so that mapping is applied once, where the sighting is consumed,
    // rather than guessed at in each source.
    let observation = from_flow(&udp_flow(GAME, server(27_015), 64)).unwrap();
    assert_eq!(observation.pid, GAME);
}

// ---------------------------------------------------------------- filtering

#[test]
fn public_and_tunnelled_endpoints_are_worth_tracking() {
    let policy = AddressPolicy::default();
    assert!(is_worth_tracking(&policy, EndpointKey::udp(server(27_015))));
    // Inside the FakeIP sentinel range: measurable, by a TLS hello, and labelled as
    // end-to-end through the tunnel.
    let sentinel = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 18, 0, 7)), 443);
    assert!(is_worth_tracking(&policy, EndpointKey::tcp(sentinel)));
}

#[test]
fn addresses_no_probe_kind_would_accept_are_left_out() {
    // A game's conversation with its own launcher, its router, or a link-local service is
    // not a network path this product can say anything about — and tracking it would spend
    // one of the sixteen endpoint slots the application is allowed.
    let policy = AddressPolicy::default();
    for address in [
        "127.0.0.1",
        "192.168.1.1",
        "10.0.0.1",
        "169.254.1.1",
        "224.0.0.251",
    ] {
        let ip: IpAddr = address.parse().unwrap();
        assert!(
            !is_worth_tracking(&policy, EndpointKey::udp(SocketAddr::new(ip, 443))),
            "{address}"
        );
    }
}

#[test]
fn port_zero_is_never_an_endpoint() {
    let policy = AddressPolicy::default();
    assert!(!is_worth_tracking(&policy, EndpointKey::tcp(server(0))));
}

// ---------------------------------------------------------------- the sources

/// A connection table that always reports the same rows.
struct FakeTable {
    rows: Vec<Connection>,
}

impl ConnectionTable for FakeTable {
    fn snapshot(&mut self, out: &mut Vec<Connection>) -> Result<(), nm_platform::Error> {
        out.clear();
        out.extend(self.rows.iter().cloned());
        Ok(())
    }
}

/// A flow source that refuses to start, the way an account without the one-time Windows
/// setup does.
struct RefusingFlowSource;

impl FlowEventSource for RefusingFlowSource {
    fn start(&mut self, _sink: FlowSink) -> Result<(), nm_platform::Error> {
        Err(nm_platform::Error::TracingNotPermitted)
    }
    fn watch(&self, _pids: &[Pid]) {}
    fn is_running(&self) -> bool {
        false
    }
    fn stop(&mut self) {}
}

/// A flow source that starts and hands its sink back so a test can push events.
///
/// Its session can be stopped from outside, the way a real one is when another consumer
/// takes the name over.
#[derive(Clone, Default)]
struct FakeFlowSource {
    sink: std::sync::Arc<std::sync::Mutex<Option<FlowSink>>>,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
    starts: std::sync::Arc<std::sync::atomic::AtomicU32>,
    watched: std::sync::Arc<std::sync::Mutex<Vec<Pid>>>,
}

impl FakeFlowSource {
    /// Ends the session without telling the source, as an outside stop does.
    fn kill(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::Release);
        *self.sink.lock().unwrap() = None;
    }

    fn starts(&self) -> u32 {
        self.starts.load(std::sync::atomic::Ordering::Acquire)
    }

    fn emit(&self, event: &FlowEvent) {
        let mut held = self.sink.lock().unwrap();
        if let Some(sink) = held.as_mut() {
            sink(event);
        }
    }
}

impl FlowEventSource for FakeFlowSource {
    fn start(&mut self, sink: FlowSink) -> Result<(), nm_platform::Error> {
        *self.sink.lock().unwrap() = Some(sink);
        self.running
            .store(true, std::sync::atomic::Ordering::Release);
        self.starts
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(())
    }
    fn watch(&self, pids: &[Pid]) {
        let mut watched = self.watched.lock().unwrap();
        watched.clear();
        watched.extend_from_slice(pids);
    }
    fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::Acquire)
    }
    fn stop(&mut self) {
        self.kill();
    }
}

/// Waits for the next observation, failing rather than hanging if none arrives.
async fn next(observations: &mut tokio::sync::mpsc::Receiver<Observation>) -> Option<Observation> {
    tokio::time::timeout(Duration::from_secs(5), observations.recv())
        .await
        .expect("an observation must arrive well inside the poll period")
}

#[tokio::test]
async fn the_table_reports_only_the_processes_being_watched() {
    let (mut discovery, mut observations) = Discovery::start(
        AddressPolicy::default(),
        Ok(Box::new(FakeTable {
            rows: vec![
                tcp_row(GAME, TcpState::Established, Some(server(443))),
                // A different process the user did not choose.
                tcp_row(OTHER, TcpState::Established, Some(server(8080))),
            ],
        })),
        Err(nm_platform::Error::UnsupportedPlatform),
    );

    discovery.watch(&[GAME]);

    let observation = next(&mut observations).await.expect("the channel is open");
    assert_eq!(observation.pid, GAME);
    assert_eq!(observation.endpoint.address, server(443));

    // The next thing to arrive is the same endpoint on the following poll, never the
    // other process's.
    let observation = next(&mut observations).await.expect("the channel is open");
    assert_eq!(observation.pid, GAME);
}

#[tokio::test]
async fn nothing_is_read_until_a_process_is_chosen() {
    let (_discovery, mut observations) = Discovery::start(
        AddressPolicy::default(),
        Ok(Box::new(FakeTable {
            rows: vec![tcp_row(GAME, TcpState::Established, Some(server(443)))],
        })),
        Err(nm_platform::Error::UnsupportedPlatform),
    );

    let quiet = tokio::time::timeout(Duration::from_millis(1_500), observations.recv()).await;
    assert!(
        quiet.is_err(),
        "a table poll enumerates every socket on the machine; there is no reason to \
         until the user has asked about a process"
    );
}

#[tokio::test]
async fn endpoints_nothing_could_measure_never_leave_the_poll_thread() {
    let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6463);
    let (mut discovery, mut observations) = Discovery::start(
        AddressPolicy::default(),
        Ok(Box::new(FakeTable {
            rows: vec![
                tcp_row(GAME, TcpState::Established, Some(loopback)),
                tcp_row(GAME, TcpState::Established, Some(server(443))),
            ],
        })),
        Err(nm_platform::Error::UnsupportedPlatform),
    );

    discovery.watch(&[GAME]);

    let observation = next(&mut observations).await.expect("the channel is open");
    assert_eq!(
        observation.endpoint.address,
        server(443),
        "the loopback row must be filtered before it reaches the tracker"
    );
}

#[tokio::test]
async fn the_poll_thread_stops_with_the_session() {
    let (mut discovery, mut observations) = Discovery::start(
        AddressPolicy::default(),
        Ok(Box::new(FakeTable {
            rows: vec![tcp_row(GAME, TcpState::Established, Some(server(443)))],
        })),
        Err(nm_platform::Error::UnsupportedPlatform),
    );
    discovery.watch(&[GAME]);
    next(&mut observations).await.expect("the channel is open");

    drop(discovery);

    // The thread notices its instruction channel closed and returns, which drops the last
    // sender and closes this end.
    while next(&mut observations).await.is_some() {}
}

#[tokio::test]
async fn a_refused_tracing_session_is_a_state_not_a_fault() {
    let (discovery, _observations) = Discovery::start(
        AddressPolicy::default(),
        Err(nm_platform::Error::UnsupportedPlatform),
        Ok(Box::new(RefusingFlowSource)),
    );

    assert_eq!(
        discovery.flow_status(),
        FlowStatus::NotPermitted,
        "the ordinary state on Windows until the user performs the one-time setup"
    );
}

#[tokio::test]
async fn a_missing_flow_source_is_reported_as_unavailable() {
    let (discovery, _observations) = Discovery::start(
        AddressPolicy::default(),
        Err(nm_platform::Error::UnsupportedPlatform),
        Err(nm_platform::Error::UnsupportedPlatform),
    );

    assert_eq!(discovery.flow_status(), FlowStatus::Unavailable);
    assert_eq!(discovery.dropped_flow_events(), 0);
}

#[tokio::test]
async fn a_flow_event_reaches_the_session() {
    let source = FakeFlowSource::default();
    let (discovery, mut observations) = Discovery::start(
        AddressPolicy::default(),
        Err(nm_platform::Error::UnsupportedPlatform),
        Ok(Box::new(source.clone())),
    );
    assert_eq!(discovery.flow_status(), FlowStatus::Active);

    source.emit(&udp_flow(GAME, server(27_015), 512));
    // A loopback peer is filtered on the tracing thread, before it becomes state.
    source.emit(&udp_flow(
        GAME,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6463),
        64,
    ));

    let observation = next(&mut observations).await.expect("the channel is open");
    assert_eq!(observation.endpoint, EndpointKey::udp(server(27_015)));
    assert_eq!(observation.bytes(), Some(512));
    assert_eq!(discovery.dropped_flow_events(), 0);

    // The session and the test's own copy of the sink are the only holders of the sending
    // end; letting both go closes the stream.
    drop(discovery);
    source.kill();
    assert_eq!(
        next(&mut observations).await,
        None,
        "the loopback event must never have been queued"
    );
}

#[tokio::test]
async fn a_session_stopped_from_outside_is_reported_rather_than_assumed_healthy() {
    // Found by running the app: an ETW session is a named system object, and whoever opens
    // that name next takes it over. The app kept saying "active" while discovering no UDP
    // endpoints at all, which is indistinguishable on screen from a game that has none.
    let source = FakeFlowSource::default();
    let (discovery, _observations) = Discovery::start(
        AddressPolicy::default(),
        Err(nm_platform::Error::UnsupportedPlatform),
        Ok(Box::new(source.clone())),
    );
    assert_eq!(discovery.flow_status(), FlowStatus::Active);

    source.kill();

    assert_eq!(discovery.flow_status(), FlowStatus::Stopped);
}

#[tokio::test]
async fn a_stopped_session_is_started_again_and_resumes_watching() {
    let source = FakeFlowSource::default();
    let (mut discovery, mut observations) = Discovery::start(
        AddressPolicy::default(),
        Err(nm_platform::Error::UnsupportedPlatform),
        Ok(Box::new(source.clone())),
    );
    discovery.watch(&[GAME]);
    assert_eq!(source.starts(), 1);

    source.kill();
    discovery.revive_flow();

    assert_eq!(discovery.flow_status(), FlowStatus::Active);
    assert_eq!(source.starts(), 2);
    assert_eq!(
        *source.watched.lock().unwrap(),
        vec![GAME],
        "a session that came back knows nothing about what it was watching"
    );

    // And it delivers again through the replacement sink.
    source.emit(&udp_flow(GAME, server(27_015), 512));
    let observation = next(&mut observations).await.expect("the channel is open");
    assert_eq!(observation.endpoint, EndpointKey::udp(server(27_015)));
}

#[tokio::test]
async fn a_running_session_is_never_restarted() {
    let source = FakeFlowSource::default();
    let (mut discovery, _observations) = Discovery::start(
        AddressPolicy::default(),
        Err(nm_platform::Error::UnsupportedPlatform),
        Ok(Box::new(source.clone())),
    );

    for _ in 0..5 {
        discovery.revive_flow();
    }

    assert_eq!(source.starts(), 1);
}

#[tokio::test]
async fn a_refused_session_is_not_retried_as_though_it_had_fallen_over() {
    // A refusal is a fact about the account, not a session that stopped. Retrying it every
    // second would spend a system call a second to be told the same thing.
    let (mut discovery, _observations) = Discovery::start(
        AddressPolicy::default(),
        Err(nm_platform::Error::UnsupportedPlatform),
        Ok(Box::new(RefusingFlowSource)),
    );

    discovery.revive_flow();

    assert_eq!(discovery.flow_status(), FlowStatus::NotPermitted);
}
