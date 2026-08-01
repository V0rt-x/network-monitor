//! Flow events on Windows, from the `Microsoft-Windows-TCPIP` provider via ETW.
//!
//! Every constant below was established by measurement rather than by reading, and the
//! reasoning is written up in `docs/etw-privileges-spike.md`. Three of them are load
//! bearing:
//!
//! * **The provider.** `Microsoft-Windows-Kernel-Network`, the obvious candidate, is a
//!   kernel provider and is refused to an unelevated session even when that session may
//!   be created at all. `Microsoft-Windows-TCPIP` is not, and it carries the UDP events
//!   with the process, both socket addresses and a byte count.
//! * **The level.** `Informational` excludes the per-packet telemetry that the same
//!   keyword emits at higher levels — sixteen times the volume for the same information.
//! * **The event-ID filter.** The kernel applies it before events reach this process. In
//!   a measured comparison it took a twenty-second stream from 32 859 events to 94, all
//!   of them wanted. Without it this module would be the app's largest CPU cost; with it
//!   it is not measurable against the budget.
//!
//! Filtering by process ID is deliberately *not* attempted at the kernel: `ferrisetw`
//! documents that filter as ineffective on a user-mode session, and it could not work for
//! this provider regardless, because these events are written by the kernel rather than by
//! the process they describe. Process selection therefore happens in the callback, before
//! any address is decoded.
//!
//! **A session usually cannot be opened at all.** A standard Windows account may not
//! create one; the machine needs a one-time administrative setup first. That is reported
//! as [`Error::TracingNotPermitted`] so the layer above can explain it, and it is the
//! expected answer rather than a failure.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use ferrisetw::native::EvntraceNativeError;
use ferrisetw::parser::Parser;
use ferrisetw::provider::{EventFilter, Provider};
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::{stop_trace_by_name, TraceError, TraceTrait, UserTrace};
use ferrisetw::EventRecord;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE, ERROR_INVALID_NAME,
};

use super::{
    decode_sockaddr, FlowDirection, FlowEvent, FlowEventSource, FlowReport, FlowSink, TcpPortWatch,
    TcpRttEvent,
};
use crate::connection::Protocol;
use crate::process::Pid;
use crate::Error;

/// `Microsoft-Windows-TCPIP`.
const TCPIP_PROVIDER_GUID: &str = "2F07E2EE-15DB-40F1-90EF-9D7BA282188A";

/// `ut:SendPath | ut:ReceivePath`, plus the keyword the TCP connection summary lives under.
///
/// Narrower keywords were tried and produce no UDP events at all: `ut:Endpoint`,
/// `ut:Transfer`, `ut:TcpipEndpoint` and `ut:TxIoInfo`. The third bit comes from the
/// provider's own manifest, where event 1477 is declared under `0x8000200000000000`.
const KEYWORDS: u64 = 0x0000_2003_0000_0000;

/// `TcpSummary`, a level of the provider's own above `win:Informational`.
///
/// The UDP events sit at `Informational` (4) and would be satisfied by it; the connection
/// summary is declared at 16, and ETW delivers an event only when its level is at or below
/// the session's. So reaching the summary means raising the session, which also lets
/// through the `Verbose` per-path telemetry that level 4 was chosen to exclude.
///
/// **The event-ID filter is what makes that safe, and it is measured rather than assumed**:
/// with the filter in place a live game session delivered 44 events a second in total,
/// summaries included (`docs/flow-metrics-spike.md`). The kernel applies the filter before
/// anything reaches this process. Level 17 — `TcpIpPerPacket`, the expensive one — stays
/// above the session and is never generated for us at all.
///
/// The alternative considered was a second session for the summary alone, keeping this one
/// at level 4. It was rejected for the cost of a second named system object and a second
/// pump thread, against a delivered rate that is not measurable in the budget.
const LEVEL_TCP_SUMMARY: u8 = 16;

/// `UdpEndpointSendMessages`.
const EVENT_UDP_SENT: u16 = 1169;
/// `UdpEndpointReceiveMessages`.
const EVENT_UDP_RECEIVED: u16 = 1170;
/// `TcpConnectionSummary` — the stack's own round-trip estimate for a connection.
const EVENT_TCP_SUMMARY: u16 = 1477;

/// Name of our tracing session.
///
/// Fixed rather than unique per run, so that a session orphaned by a crash is found and
/// reclaimed on the next start instead of accumulating one per crash — ETW sessions
/// outlive the process that created them.
///
/// The cost of that choice is that the name is the only thing standing between two
/// consumers: whoever starts second reclaims the session from whoever started first, and
/// the first is left running with nothing arriving. That is why
/// [`FlowEventSource::is_running`] exists, and why anything that opens a session for its
/// own purposes — the tests below — must use [`EtwFlowSource::with_session_name`] rather
/// than quietly stopping the session of a copy of the app the developer has running.
pub const SESSION_NAME: &str = "NetworkMonitorFlows";

/// Delivers per-process flow events by consuming ETW.
pub struct EtwFlowSource {
    /// Processes whose flows are reported. Shared with the tracing callback, which is the
    /// only reader, and replaced wholesale by [`FlowEventSource::watch`].
    watched: Arc<Mutex<Vec<Pid>>>,
    /// Local ports whose connection summaries are wanted. See [`TcpPortWatch`]: the summary
    /// event names no process, so this is the only filter it can have.
    ports: TcpPortWatch,
    /// Session name to create and to reclaim.
    session: String,
    /// The running session, if any. `UserTrace::stop` consumes the value, hence the option.
    trace: Option<UserTrace>,
    /// The thread pumping the session's buffers.
    worker: Option<JoinHandle<()>>,
    /// Cleared by the pump thread when the session ends, however it ended.
    ///
    /// The only honest way to know: `ProcessTrace` blocks until the session stops, so its
    /// return *is* the news that tracing is over — whether we stopped it, another consumer
    /// reclaimed the name, or an administrator killed it.
    running: Arc<AtomicBool>,
}

impl Default for EtwFlowSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EtwFlowSource {
    /// Creates a source that is not yet tracing, using the product's session name.
    #[must_use]
    pub fn new() -> Self {
        Self::with_session_name(SESSION_NAME)
    }

    /// Creates a source that will open a session of its own name.
    ///
    /// For tests and for anything else that must not reclaim the running application's
    /// session out from under it.
    #[must_use]
    pub fn with_session_name(session: &str) -> Self {
        Self {
            watched: Arc::new(Mutex::new(Vec::new())),
            ports: TcpPortWatch::new(),
            session: session.to_owned(),
            trace: None,
            worker: None,
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl std::fmt::Debug for EtwFlowSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The session handle and its pump thread have no useful debug form, and the one
        // thing worth seeing is whether tracing is live.
        f.debug_struct("EtwFlowSource")
            .field("session", &self.session)
            .field("running", &self.is_running())
            .finish_non_exhaustive()
    }
}

impl FlowEventSource for EtwFlowSource {
    fn start(&mut self, sink: FlowSink) -> Result<(), Error> {
        if self.is_running() {
            return Ok(());
        }
        // A trace object left over from a session that has since stopped. Letting go of it
        // first is what makes `start` a usable retry rather than a no-op that reports
        // success while nothing is being delivered.
        self.stop();

        // Reclaim a session our own previous run may have left behind.
        let _ = stop_trace_by_name(&self.session);

        let watched = Arc::clone(&self.watched);
        let ports = self.ports.clone();
        let sink = Arc::new(Mutex::new(sink));
        let callback = move |record: &EventRecord, locator: &SchemaLocator| {
            dispatch(record, locator, &watched, &ports, &sink);
        };

        let provider = Provider::by_guid(TCPIP_PROVIDER_GUID)
            .any(KEYWORDS)
            .level(LEVEL_TCP_SUMMARY)
            .add_filter(EventFilter::ByEventIds(vec![
                EVENT_UDP_SENT,
                EVENT_UDP_RECEIVED,
                EVENT_TCP_SUMMARY,
            ]))
            .add_callback(callback)
            .build();

        let (trace, handle) = UserTrace::new()
            .named(self.session.clone())
            .enable(provider)
            .start()
            .map_err(map_trace_error)?;

        self.running.store(true, Ordering::Release);
        let running = Arc::clone(&self.running);
        // `process_from_handle` blocks until the trace stops, so it gets its own thread —
        // and its return is the one reliable signal that the session has ended, whoever
        // ended it.
        self.worker = Some(std::thread::spawn(move || {
            let _ = UserTrace::process_from_handle(handle);
            running.store(false, Ordering::Release);
        }));
        self.trace = Some(trace);
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    fn watch(&self, pids: &[Pid]) {
        if let Ok(mut watched) = self.watched.lock() {
            watched.clear();
            watched.extend_from_slice(pids);
        }
    }

    fn watch_tcp_ports(&mut self, ports: TcpPortWatch) {
        self.ports = ports;
    }

    fn stop(&mut self) {
        if let Some(trace) = self.trace.take() {
            // A failure here means the session is already gone, which is the state we
            // wanted; there is nothing to report and nothing to retry.
            let _ = trace.stop();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for EtwFlowSource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Turns one ETW event into a [`FlowEvent`], or discards it.
///
/// Every step is a reason to drop the event rather than to guess: an event number we did
/// not ask for, a schema that will not resolve, a field that will not parse, an address
/// family we cannot name. Discovery that invents an endpoint would send probes to it.
fn dispatch(
    record: &EventRecord,
    locator: &SchemaLocator,
    watched: &Mutex<Vec<Pid>>,
    ports: &TcpPortWatch,
    sink: &Mutex<FlowSink>,
) {
    let direction = match record.event_id() {
        EVENT_UDP_SENT => FlowDirection::Sent,
        EVENT_UDP_RECEIVED => FlowDirection::Received,
        EVENT_TCP_SUMMARY => return dispatch_summary(record, locator, ports, sink),
        _ => return,
    };

    let Ok(schema) = locator.event_schema(record) else {
        return;
    };
    let parser = Parser::create(record, &schema);

    let Ok(raw_pid) = parser.try_parse::<u32>("Pid") else {
        return;
    };
    let pid = Pid::new(raw_pid);

    // Checked before any address is decoded: flows belonging to applications the user did
    // not select must not enter this program's memory at all.
    let watching = watched.lock().is_ok_and(|list| list.contains(&pid));
    if !watching {
        return;
    }

    // `NumMessages` sits beside these and is deliberately not read. It reports 1 on a send
    // and **0 on a receive** — measured over four minutes of a live match, on every one of
    // 4 792 arrivals — so it cannot say how many datagrams an arrival stands for. What
    // takes its place is a measurement rather than a field: consecutive arrivals of one
    // server update land under a millisecond apart, and `nm_core::flow` coalesces on that.
    let (Ok(local), Ok(remote), Ok(bytes)) = (
        parser.try_parse::<Vec<u8>>("LocalSockAddr"),
        parser.try_parse::<Vec<u8>>("RemoteSockAddr"),
        parser.try_parse::<u32>("NumBytes"),
    ) else {
        return;
    };

    let (Some(local), Some(remote)) = (decode_sockaddr(&local), decode_sockaddr(&remote)) else {
        return;
    };

    let event = FlowEvent {
        pid,
        protocol: Protocol::Udp,
        local,
        remote,
        direction,
        bytes: u64::from(bytes),
        // The kernel's own stamp, not the moment this callback ran: events arrive in
        // batches up to a buffer flush late, so the arrival-timing metrics would otherwise
        // measure the flush interval.
        observed_at: super::event_time(record.raw_timestamp()),
    };

    if let Ok(mut sink) = sink.lock() {
        sink(FlowReport::Flow(&event));
    }
}

/// Turns one TCP connection summary into a [`TcpRttEvent`], or discards it.
///
/// **The local port is read first and nothing else is touched until it matches.** This
/// event names no process — verified against the live provider — so without that test the
/// alternative would be decoding both addresses of every connection closing anywhere on the
/// machine in order to throw almost all of them away. On a machine whose owner is under
/// surveillance that is not an acceptable way to obtain a latency figure, and the port
/// costs one integer comparison.
fn dispatch_summary(
    record: &EventRecord,
    locator: &SchemaLocator,
    ports: &TcpPortWatch,
    sink: &Mutex<FlowSink>,
) {
    let Ok(schema) = locator.event_schema(record) else {
        return;
    };
    let parser = Parser::create(record, &schema);

    let Ok(local_port) = parser.try_parse::<u32>("LocalPort") else {
        return;
    };
    let Some(local_port) = summary_port(local_port) else {
        return;
    };
    if !ports.contains(local_port) {
        return;
    }

    let (Ok(local), Ok(remote)) = (
        parser.try_parse::<Vec<u8>>("LocalAddress"),
        parser.try_parse::<Vec<u8>>("RemoteAddress"),
    ) else {
        return;
    };
    let (Some(local), Some(remote)) = (decode_sockaddr(&local), decode_sockaddr(&remote)) else {
        return;
    };

    let (Ok(rtt), Ok(min_rtt), Ok(max_rtt)) = (
        parser.try_parse::<u32>("RttUs"),
        parser.try_parse::<u32>("MinRttUs"),
        parser.try_parse::<u32>("MaxRttUs"),
    ) else {
        return;
    };

    let event = TcpRttEvent {
        // The blob's own port is what the connection used; `LocalPort` is a later addition
        // to the event and repeats it. Where the blob is unported — a shape this provider
        // has not been seen to emit, but one the decoder allows — the separate field stands
        // in, so the connection is still identifiable.
        local: with_port(local, local_port),
        remote,
        rtt: Duration::from_micros(u64::from(rtt)),
        min_rtt: Duration::from_micros(u64::from(min_rtt)),
        max_rtt: Duration::from_micros(u64::from(max_rtt)),
        observed_at: super::event_time(record.raw_timestamp()),
    };

    if let Ok(mut sink) = sink.lock() {
        sink(FlowReport::Rtt(&event));
    }
}

/// Reads the connection summary's `LocalPort` field, which is in **network byte order**.
///
/// The recurring hazard of this whole area, and it fails silently: the field is a `DWORD`
/// holding a port the wrong way round, so a filter that compared it directly would match
/// nothing and the feature would simply appear not to work. Found exactly that way — the
/// live test below produced no measurement for twenty-five seconds while the addresses beside
/// the field decoded perfectly. The connection tables carry the same trap and are pinned by
/// their own test; this is that test's twin.
///
/// Returns [`None`] for a value that does not fit a port, which cannot happen for a real
/// event and would mean the field had changed meaning.
fn summary_port(raw: u32) -> Option<u16> {
    u16::try_from(raw).ok().map(u16::swap_bytes)
}

/// Fills in a socket address's port where the decoded blob carried none.
fn with_port(address: std::net::SocketAddr, port: u16) -> std::net::SocketAddr {
    if address.port() == 0 {
        std::net::SocketAddr::new(address.ip(), port)
    } else {
        address
    }
}

/// Maps the tracing library's failure onto ours.
///
/// The distinction that matters is "this account may not trace" against everything else:
/// the first is the normal state of a machine that has not been set up, which the UI has
/// to explain, and the rest are faults.
fn map_trace_error(error: TraceError) -> Error {
    match error {
        TraceError::InvalidTraceName => Error::Os {
            api: "StartTrace",
            code: ERROR_INVALID_NAME,
        },
        TraceError::EtwNativeError(EvntraceNativeError::AlreadyExist) => Error::Os {
            api: "StartTrace",
            code: ERROR_ALREADY_EXISTS,
        },
        TraceError::EtwNativeError(EvntraceNativeError::InvalidHandle) => Error::Os {
            api: "StartTrace",
            code: ERROR_INVALID_HANDLE,
        },
        TraceError::EtwNativeError(EvntraceNativeError::IoError(io)) => {
            let code = io
                .raw_os_error()
                .and_then(|raw| u32::try_from(raw).ok())
                .unwrap_or(0);
            if code == ERROR_ACCESS_DENIED {
                Error::TracingNotPermitted
            } else {
                Error::Os {
                    api: "StartTrace",
                    code,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, UdpSocket};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_fresh_source_is_not_running() {
        let source = EtwFlowSource::new();
        assert!(source.trace.is_none());
        assert!(!source.is_running());
        assert!(format!("{source:?}").contains("running: false"));
    }

    #[test]
    fn the_product_session_name_is_not_what_the_tests_take_over() {
        // The guard on the footgun above: a test that opened the product's session would
        // stop a running app's tracing, and the app would keep reporting it as healthy.
        let source = EtwFlowSource::with_session_name("NetworkMonitorFlowsTest");
        assert_ne!(source.session, SESSION_NAME);
        assert_eq!(EtwFlowSource::new().session, SESSION_NAME);
    }

    #[test]
    fn watching_replaces_rather_than_accumulates() {
        // The set is what the user has selected right now; a stop that left its pid
        // behind would keep reporting a game the user stopped monitoring.
        let source = EtwFlowSource::new();
        source.watch(&[Pid::new(10), Pid::new(20)]);
        source.watch(&[Pid::new(30)]);

        let watched = source.watched.lock().unwrap();
        assert_eq!(*watched, vec![Pid::new(30)]);
    }

    #[test]
    fn stopping_a_source_that_never_started_is_harmless() {
        let mut source = EtwFlowSource::new();
        source.stop();
        source.stop();
    }

    #[test]
    fn access_denied_is_reported_as_a_permission_state_not_a_fault() {
        let denied = TraceError::EtwNativeError(EvntraceNativeError::IoError(
            std::io::Error::from_raw_os_error(
                i32::try_from(ERROR_ACCESS_DENIED).expect("the constant fits"),
            ),
        ));
        assert_eq!(map_trace_error(denied), Error::TracingNotPermitted);
    }

    #[test]
    fn other_failures_keep_their_code() {
        let other = TraceError::EtwNativeError(EvntraceNativeError::IoError(
            std::io::Error::from_raw_os_error(1450),
        ));
        assert_eq!(
            map_trace_error(other),
            Error::Os {
                api: "StartTrace",
                code: 1450
            }
        );
        assert_eq!(
            map_trace_error(TraceError::EtwNativeError(
                EvntraceNativeError::AlreadyExist
            )),
            Error::Os {
                api: "StartTrace",
                code: ERROR_ALREADY_EXISTS
            }
        );
    }

    /// End to end against the running system.
    ///
    /// **Both outcomes are a pass, and that is deliberate.** On a machine whose account
    /// may not trace — the default, and what CI runners are — the only correct behaviour
    /// is [`Error::TracingNotPermitted`], and asserting that is the whole test. Where a
    /// session can be opened, the test additionally proves the thing that matters: that a
    /// UDP peer of *this* process is discovered, which the connection tables cannot do.
    ///
    /// The traffic is generated here and stays on loopback; nothing is sent to a network,
    /// and only this process's own flows are watched.
    #[test]
    fn discovers_a_udp_peer_of_this_process_or_says_it_may_not_trace() {
        let (tx, rx) = mpsc::channel();
        // A session name of its own. Sharing the product's would mean that running the test
        // suite silently stops the tracing of an app the developer has open — which is
        // exactly what happened, and cost an evening working out why a live game's UDP
        // endpoints had stopped appearing.
        let mut source = EtwFlowSource::with_session_name("NetworkMonitorFlowsTest");
        source.watch(&[Pid::new(std::process::id())]);

        let sink: FlowSink = Box::new(move |report: FlowReport<'_>| {
            if let FlowReport::Flow(event) = report {
                let _ = tx.send(event.clone());
            }
        });

        match source.start(sink) {
            Err(Error::TracingNotPermitted) => {
                // Printed rather than silent: a test that can pass two ways must say
                // which way it went, or a machine that lost the ability to trace would
                // look exactly like one where discovery still works.
                eprintln!("skipped: this account may not open a tracing session");
                return;
            }
            Err(other) => panic!("starting the trace failed unexpectedly: {other}"),
            Ok(()) => {}
        }

        let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind a loopback socket");
        let target = server.local_addr().expect("read the bound address");
        let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind a loopback socket");

        // ETW delivers in batches, so the traffic is repeated until an event arrives or
        // the deadline passes; a single datagram could sit in a buffer past the check.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut discovered = None;
        while Instant::now() < deadline && discovered.is_none() {
            client
                .send_to(&[7u8; 32], target)
                .expect("send on loopback");
            while let Ok(event) = rx.try_recv() {
                if event.remote == target {
                    discovered = Some(event);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        source.stop();

        // Observed on the dev machine: a session left running by a crashed app — the product
        // reclaims its own name on start-up, but only when it gets to run — consumes this
        // provider with nobody draining it, and a second consumer of the same provider then
        // receives nothing at all. `logman query -ets` lists them; `logman stop <name> -ets`
        // clears one.
        let event = discovered.expect(
            "a UDP peer of this process must be discovered — check for an orphaned \
             NetworkMonitorFlows tracing session if this fails",
        );
        assert_eq!(event.pid, Pid::new(std::process::id()));
        assert_eq!(event.protocol, Protocol::Udp);
        assert_eq!(event.remote, target);
        assert_eq!(event.direction, FlowDirection::Sent);
        assert_eq!(
            event.bytes, 32,
            "the byte count must be the datagram's size"
        );
        assert!(event.local.ip().is_loopback());
        assert!(
            !event.observed_at.is_zero(),
            "the event must carry the kernel's own stamp — the arrival metrics measure \
             nothing without it"
        );
    }

    /// The stack's own round-trip estimate, end to end against the running system.
    ///
    /// Passes two ways for the same reason as the test above. Where a session opens, it
    /// proves the three things section C rests on: that the summary event reaches an
    /// unelevated session at all, that the local-port filter is enough to identify a
    /// connection without any process identifier, and that both address blobs decode.
    ///
    /// The connection is made and closed here, on loopback. A summary arrives when a
    /// connection ends as well as periodically during a long one, so a short local
    /// connection is the quickest way to produce one — and its round trip will be near
    /// zero, which is the correct answer for loopback and is asserted as such rather than
    /// dressed up as a network measurement.
    #[test]
    fn reads_the_stacks_own_round_trip_or_says_it_may_not_trace() {
        use std::io::{Read as _, Write as _};
        use std::net::{TcpListener, TcpStream};

        let (tx, rx) = mpsc::channel();
        let ports = TcpPortWatch::new();
        let mut source = EtwFlowSource::with_session_name("NetworkMonitorRttTest");
        source.watch_tcp_ports(ports.clone());

        let sink: FlowSink = Box::new(move |report: FlowReport<'_>| {
            if let FlowReport::Rtt(event) = report {
                let _ = tx.send(*event);
            }
        });

        match source.start(sink) {
            Err(Error::TracingNotPermitted) => {
                eprintln!("skipped: this account may not open a tracing session");
                return;
            }
            Err(other) => panic!("starting the trace failed unexpectedly: {other}"),
            Ok(()) => {}
        }

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind a loopback socket");
        let server = listener.local_addr().expect("read the bound address");

        // Connections are made in a loop rather than once: a summary is written when the
        // connection ends, and the buffered stream that carries it is flushed on a timer,
        // so a single attempt could easily finish before the deadline without its summary
        // having been delivered yet.
        let deadline = Instant::now() + Duration::from_secs(25);
        let mut measured: Option<TcpRttEvent> = None;
        // Every port used so far stays watched: a summary is written when the connection
        // ends and delivered on the buffer's own timer, so one for an earlier attempt can
        // land while a later attempt is being made.
        let mut used: Vec<u16> = Vec::new();
        while Instant::now() < deadline && measured.is_none() {
            if let Ok(mut client) = TcpStream::connect(server) {
                let local = client.local_addr().expect("read the bound address");
                used.push(local.port());
                // Named *before* the connection closes, since the filter is applied when
                // the event is decoded and the summary can arrive at any point after.
                ports.replace(used.iter().copied());
                if let Ok((mut accepted, _)) = listener.accept() {
                    // A connection that carried data gives the stack something to time.
                    let _ = client.write_all(&[7u8; 64]);
                    let _ = accepted.write_all(&[7u8; 64]);
                    let mut buffer = [0u8; 64];
                    let _ = accepted.read(&mut buffer);
                    let _ = client.read(&mut buffer);
                }
                drop(client);
            }

            while let Ok(event) = rx.try_recv() {
                if used.contains(&event.local.port()) {
                    measured = Some(event);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        source.stop();

        let event = measured.expect(
            "the stack's own round trip must be readable — check for an orphaned \
             NetworkMonitorFlows tracing session if this fails",
        );
        assert_eq!(
            event.remote, server,
            "the far end must decode, or a measurement could never be matched to an endpoint"
        );
        assert!(event.local.ip().is_loopback());
        assert!(
            event.min_rtt <= event.max_rtt,
            "the stack's own bounds must bracket each other: {event:?}"
        );
        assert!(
            event.rtt < Duration::from_millis(50),
            "a loopback connection cannot honestly be tens of milliseconds: {:?}",
            event.rtt
        );
        assert!(!event.observed_at.is_zero());
    }

    #[test]
    fn the_summarys_local_port_is_read_in_network_order() {
        // 57120 is 0xDF20, so the field holds 0x20DF — 8415. Reading it as it comes gives a
        // port that exists but is not this connection's, the filter matches nothing, and the
        // whole feature quietly does nothing at all. Observed values from a live session.
        assert_eq!(summary_port(8_415), Some(57_120));
        assert_eq!(summary_port(9_183), Some(57_123));
        assert_eq!(summary_port(0), Some(0));
        assert_eq!(
            summary_port(u32::from(u16::MAX) + 1),
            None,
            "a value too wide for a port means the field has changed meaning"
        );
    }

    #[test]
    fn a_port_nobody_asked_about_is_not_watched() {
        // The gate that keeps every other application's connections out of this process:
        // the summary event carries no process identifier, so an empty or unrelated port
        // set must match nothing rather than everything.
        let ports = TcpPortWatch::new();
        assert!(!ports.contains(443));

        ports.replace([51_000, 51_001]);
        assert!(ports.contains(51_000));
        assert!(!ports.contains(443));

        // Replacement, not accumulation: a connection that closed stops being watched, or
        // the set would eventually match ports reissued to other applications.
        ports.replace([51_002]);
        assert!(!ports.contains(51_000));
        assert!(ports.contains(51_002));
    }
}
