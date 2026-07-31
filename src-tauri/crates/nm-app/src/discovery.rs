//! Feeding [`crate::apps::AppMonitor`] from the operating system.
//!
//! Two sources of very different shape, joined into one stream of [`Observation`]:
//!
//! * the **connection tables**, polled once a second. They see that a socket exists and
//!   who its peer is — for TCP. A UDP row names no peer at all, so polling alone can never
//!   discover the endpoints a game actually plays over.
//! * **flow events**, pushed by the platform's tracing facility. They fill exactly that
//!   gap, and they carry byte counts, which is what lets the endpoint tracker rank by
//!   recent traffic. On Windows they need a one-time setup the user has to perform, so
//!   running without them is the **default** state rather than an edge case — and the
//!   difference is reported ([`Discovery::flow_status`]) rather than left to look like an
//!   application that has gone quiet.
//!
//! # What is filtered out, and where
//!
//! An endpoint is dropped here only when nothing could honestly be said about it:
//! [`is_worth_tracking`] keeps the addresses the probe engine can measure — public ones,
//! and the synthetic ones a local tunnel remaps — and discards loopback, LAN and reserved
//! space. Those are not measurements the product withholds; they are addresses no probe
//! kind will accept, and tracking them would spend an application's endpoint budget on a
//! game's conversation with its own launcher.
//!
//! Everything past that filter reaches the tracker, which demotes rather than drops.
//!
//! # Threads
//!
//! The table poll is a plain OS thread: the call is a synchronous syscall over a table of
//! a few hundred rows, and giving it a thread of its own keeps it off the async runtime
//! entirely rather than blocking a worker for the length of a syscall once a second. The
//! flow sink is called on the tracing thread and must never block, so it hands over with
//! [`tokio::sync::mpsc::Sender::try_send`] and counts what it could not place
//! ([`Discovery::dropped_flow_events`]) instead of waiting or pretending.

use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nm_core::address::AddressPolicy;
use nm_core::endpoint::{AppId, EndpointKey};
use nm_platform::connection::{Connection, ConnectionTable, Protocol};
use nm_platform::flow::{FlowEvent, FlowEventSource};
use nm_platform::process::Pid;
use tokio::sync::mpsc;

/// How often the connection tables are re-read.
///
/// One second is the floor `CLAUDE.md` sets for polling loops. It is also comfortably
/// below the ten seconds of silence that make an endpoint idle, so a single slow poll
/// never demotes a busy endpoint.
pub const POLL_PERIOD: Duration = Duration::from_secs(1);

/// How many observations may queue before a source waits or drops.
///
/// Generous by design: a burst of flow events must not be lost merely because the session
/// task was busy folding in a probe report. At five applications of a few dozen endpoints
/// this is several seconds of headroom, and an [`Observation`] is four machine words with
/// nothing on the heap.
const OBSERVATION_QUEUE: usize = 512;

/// One sighting of a monitored application using a remote endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    /// Which application.
    pub app: AppId,
    /// The endpoint it was seen using.
    pub endpoint: EndpointKey,
    /// The local address its flow egresses from, so a probe can follow the same route.
    ///
    /// [`None`] when the socket names no address of its own, which tells us nothing about
    /// the route and must not be turned into a guess.
    pub source: Option<IpAddr>,
    /// Bytes this sighting accounts for, or [`None`] where the source cannot count them.
    ///
    /// A connection table always answers [`None`]: it reports that a socket exists, never
    /// how busy it is. Reporting `Some(0)` would make an unmeasured endpoint
    /// indistinguishable from an idle one.
    pub bytes: Option<u64>,
}

/// Whether per-process flow events are being delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlowStatus {
    /// A session is open and events are arriving.
    Active,
    /// This account may not open a tracing session.
    ///
    /// The ordinary state on Windows until the user performs the one-time setup. UDP
    /// endpoints and byte counts are missing; TCP endpoints are not. The UI must say so
    /// plainly — an absent endpoint is not an absent flow.
    NotPermitted,
    /// No flow source exists on this platform, or it failed for another reason.
    Unavailable,
}

/// The identity the endpoint tracker keys an application by.
///
/// A process identifier, deliberately: within one session it names one running program,
/// and a game that is restarted comes back with a different one — which is the right
/// answer, because its endpoints are new and its old measurements say nothing about them.
#[must_use]
pub const fn app_of(pid: Pid) -> AppId {
    AppId::new(pid.get())
}

/// Whether an endpoint is one the probe engine could say anything about.
///
/// Two refusals, both of which would otherwise become an endpoint that is listed forever
/// and never measured: an address whose class no probe kind will accept (loopback, LAN,
/// link-local, reserved and documentation space), and port zero, which is not a
/// destination anything can be sent to.
#[must_use]
pub fn is_worth_tracking(policy: &AddressPolicy, endpoint: EndpointKey) -> bool {
    endpoint.address.port() != 0 && policy.classify(endpoint.address.ip()).worth_probing()
}

/// Turns a connection-table row into an observation, or nothing.
///
/// [`None`] for every row that does not describe a live conversation with a peer: a
/// listening socket, a connection being torn down, and every UDP row — which names no peer
/// the kernel could report.
#[must_use]
pub fn from_connection(row: &Connection) -> Option<Observation> {
    let peer = row.active_peer()?;
    Some(Observation {
        app: app_of(row.pid),
        endpoint: endpoint_key(row.protocol, peer),
        source: egress(row.local.ip()),
        bytes: None,
    })
}

/// Turns a flow event into an observation, or nothing.
///
/// [`None`] for an event whose peer is the wildcard address or port zero: a send that has
/// not bound yet reports one, and it is not an endpoint.
#[must_use]
pub fn from_flow(event: &FlowEvent) -> Option<Observation> {
    if event.remote.ip().is_unspecified() || event.remote.port() == 0 {
        return None;
    }
    Some(Observation {
        app: app_of(event.pid),
        endpoint: endpoint_key(event.protocol, event.remote),
        source: egress(event.local.ip()),
        bytes: Some(event.bytes),
    })
}

/// The transport is part of an endpoint's identity: a server reached over TCP for a lobby
/// and UDP for play is two endpoints with two independent fates.
fn endpoint_key(protocol: Protocol, peer: std::net::SocketAddr) -> EndpointKey {
    match protocol {
        Protocol::Tcp => EndpointKey::tcp(peer),
        Protocol::Udp => EndpointKey::udp(peer),
    }
}

/// The local address a probe should bind to, when the socket names one.
///
/// A socket bound to the wildcard has not committed to an interface, so there is nothing
/// to follow; binding a probe to `0.0.0.0` would let the OS choose a route that may not be
/// the application's.
fn egress(local: IpAddr) -> Option<IpAddr> {
    if local.is_unspecified() {
        None
    } else {
        Some(local)
    }
}

/// The discovery sources feeding one monitoring session.
///
/// Dropping this ends both: the poll thread notices its instruction channel has closed and
/// returns, and the flow source stops its session.
pub struct Discovery {
    watched: std::sync::mpsc::Sender<Vec<Pid>>,
    flow: Option<Box<dyn FlowEventSource>>,
    flow_status: FlowStatus,
    dropped: Arc<AtomicU64>,
}

impl std::fmt::Debug for Discovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The sources are trait objects with nothing useful to print; what a reader needs
        // is whether flow events are running and whether anything is being lost.
        f.debug_struct("Discovery")
            .field("flow_status", &self.flow_status)
            .field("dropped_flow_events", &self.dropped_flow_events())
            .finish_non_exhaustive()
    }
}

impl Discovery {
    /// Starts discovery, returning the stream of observations it will produce.
    ///
    /// The sources are passed in rather than looked up so that a test can drive the whole
    /// pipeline with fakes on any operating system; `crate::runtime` passes
    /// [`nm_platform::connection::system_table`] and
    /// [`nm_platform::flow::system_flow_source`].
    ///
    /// Nothing is reported until [`Discovery::watch`] names the processes to follow — an
    /// empty set means no table is even read, so the cost of a session with no monitored
    /// application is a thread asleep.
    #[must_use]
    pub fn start(
        policy: AddressPolicy,
        table: Result<Box<dyn ConnectionTable>, nm_platform::Error>,
        flow: Result<Box<dyn FlowEventSource>, nm_platform::Error>,
    ) -> (Self, mpsc::Receiver<Observation>) {
        let (sender, receiver) = mpsc::channel(OBSERVATION_QUEUE);
        let (watched, changes) = std::sync::mpsc::channel();
        let dropped = Arc::new(AtomicU64::new(0));

        match table {
            Ok(table) => spawn_poll_thread(table, policy.clone(), changes, sender.clone()),
            Err(error) => {
                eprintln!("network-monitor: no connection table on this platform: {error}");
            }
        }

        let (flow, flow_status) = start_flow(flow, policy, sender, Arc::clone(&dropped));

        (
            Self {
                watched,
                flow,
                flow_status,
                dropped,
            },
            receiver,
        )
    }

    /// Replaces the set of processes whose endpoints are reported.
    ///
    /// Both sources are told. Anything not named here is discarded before it becomes an
    /// [`Observation`] — in the tracing callback for flow events, which is data
    /// minimisation as much as economy: on a machine whose owner is under surveillance,
    /// this program should hold as little of the network's shape as the job allows.
    pub fn watch(&self, pids: &[Pid]) {
        // A closed channel means the poll thread has already stopped; the flow source is
        // told either way. The thread wakes on this rather than at the end of its period,
        // so a process the user just chose is discovered at once.
        let _ = self.watched.send(pids.to_vec());
        if let Some(flow) = &self.flow {
            flow.watch(pids);
        }
    }

    /// Whether per-process flow events are available, and if not, why.
    #[must_use]
    pub const fn flow_status(&self) -> FlowStatus {
        self.flow_status
    }

    /// How many flow events could not be queued because the session was behind.
    ///
    /// Counted rather than ignored: each one is a refreshed endpoint the tracker did not
    /// hear about. It is not a lost measurement — probes are what measure — but a
    /// persistently rising number would mean discovery is lagging, and that must be
    /// visible rather than inferred.
    #[must_use]
    pub fn dropped_flow_events(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Opens the flow session, classifying the refusal the common case produces.
fn start_flow(
    flow: Result<Box<dyn FlowEventSource>, nm_platform::Error>,
    policy: AddressPolicy,
    sender: mpsc::Sender<Observation>,
    dropped: Arc<AtomicU64>,
) -> (Option<Box<dyn FlowEventSource>>, FlowStatus) {
    let Ok(mut flow) = flow else {
        return (None, FlowStatus::Unavailable);
    };

    let sink = Box::new(move |event: &FlowEvent| {
        let Some(observation) = from_flow(event) else {
            return;
        };
        if !is_worth_tracking(&policy, observation.endpoint) {
            return;
        }
        // The tracing thread must never block. A full queue is counted, not waited on.
        if let Err(mpsc::error::TrySendError::Full(_)) = sender.try_send(observation) {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    });

    match flow.start(sink) {
        Ok(()) => (Some(flow), FlowStatus::Active),
        Err(nm_platform::Error::TracingNotPermitted) => (None, FlowStatus::NotPermitted),
        Err(error) => {
            eprintln!("network-monitor: flow events are unavailable: {error}");
            (None, FlowStatus::Unavailable)
        }
    }
}

/// Runs the connection-table poll on a thread of its own.
fn spawn_poll_thread(
    table: Box<dyn ConnectionTable>,
    policy: AddressPolicy,
    watched: std::sync::mpsc::Receiver<Vec<Pid>>,
    sender: mpsc::Sender<Observation>,
) {
    let started = std::thread::Builder::new()
        .name("nm-connection-poll".to_owned())
        .spawn(move || poll_loop(table, &policy, &watched, &sender));

    if let Err(error) = started {
        // The OS refused a thread, which means the machine is in no state to monitor
        // anything. TCP discovery is lost; flow events, if permitted, still work.
        eprintln!("network-monitor: connection polling could not start: {error}");
    }
}

/// Polls the connection tables until the session ends.
///
/// With no application monitored the table is never read at all: the thread simply waits
/// for a set to be named. That is not only economy — a table poll enumerates every socket
/// on the machine, and there is no reason to look at any of them until the user has asked
/// about a process.
fn poll_loop(
    mut table: Box<dyn ConnectionTable>,
    policy: &AddressPolicy,
    watched: &std::sync::mpsc::Receiver<Vec<Pid>>,
    sender: &mpsc::Sender<Observation>,
) {
    let mut rows: Vec<Connection> = Vec::new();
    let mut pids: Vec<Pid> = Vec::new();
    let mut complained = false;

    loop {
        let started = Instant::now();

        if !pids.is_empty() {
            match table.snapshot(&mut rows) {
                Ok(()) => {
                    for row in &rows {
                        if !pids.contains(&row.pid) {
                            continue;
                        }
                        let Some(observation) = from_connection(row) else {
                            continue;
                        };
                        if !is_worth_tracking(policy, observation.endpoint) {
                            continue;
                        }
                        // The receiver is gone: the session has ended.
                        if sender.blocking_send(observation).is_err() {
                            return;
                        }
                    }
                }
                Err(error) if !complained => {
                    // Said once. A table that fails usually keeps failing, and a message a
                    // second would bury everything else.
                    complained = true;
                    eprintln!("network-monitor: the connection table could not be read: {error}");
                }
                Err(_) => {}
            }
        }

        // Wait out the rest of the period, measured from the start of the poll so that the
        // work does not add to the interval — and wake early if the set of processes
        // changed, because a process the user just chose should not wait a second to be
        // looked at.
        let waited = match POLL_PERIOD.checked_sub(started.elapsed()) {
            Some(left) => watched.recv_timeout(left),
            // The poll outran its own period. Do not wait further, but still notice a
            // change or a shutdown, so a slow table cannot turn this into a busy loop.
            None => watched.try_recv().map_err(|error| match error {
                std::sync::mpsc::TryRecvError::Empty => std::sync::mpsc::RecvTimeoutError::Timeout,
                std::sync::mpsc::TryRecvError::Disconnected => {
                    std::sync::mpsc::RecvTimeoutError::Disconnected
                }
            }),
        };

        match waited {
            Ok(next) => {
                // Take the latest word rather than polling once per queued update.
                pids = next;
                while let Ok(newer) = watched.try_recv() {
                    pids = newer;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            // The session dropped its handle: there is nothing left to discover for.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}
