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

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use ferrisetw::native::EvntraceNativeError;
use ferrisetw::parser::Parser;
use ferrisetw::provider::{EventFilter, Provider};
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::{stop_trace_by_name, TraceError, TraceTrait, UserTrace};
use ferrisetw::EventRecord;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE, ERROR_INVALID_NAME,
};

use super::{decode_sockaddr, FlowDirection, FlowEvent, FlowEventSource, FlowSink};
use crate::connection::Protocol;
use crate::process::Pid;
use crate::Error;

/// `Microsoft-Windows-TCPIP`.
const TCPIP_PROVIDER_GUID: &str = "2F07E2EE-15DB-40F1-90EF-9D7BA282188A";

/// `ut:SendPath | ut:ReceivePath` — the keywords the UDP endpoint events live under.
///
/// Narrower keywords were tried and produce no UDP events at all: `ut:Endpoint`,
/// `ut:Transfer`, `ut:TcpipEndpoint` and `ut:TxIoInfo`.
const KEYWORD_SEND_AND_RECEIVE_PATHS: u64 = 0x0000_0003_0000_0000;

/// `win:Informational`. Anything higher adds per-packet telemetry we would only discard.
const LEVEL_INFORMATIONAL: u8 = 4;

/// `UdpEndpointSendMessages`.
const EVENT_UDP_SENT: u16 = 1169;
/// `UdpEndpointReceiveMessages`.
const EVENT_UDP_RECEIVED: u16 = 1170;

/// Name of our tracing session.
///
/// Fixed rather than unique per run, so that a session orphaned by a crash is found and
/// reclaimed on the next start instead of accumulating one per crash — ETW sessions
/// outlive the process that created them.
const SESSION_NAME: &str = "NetworkMonitorFlows";

/// Delivers per-process flow events by consuming ETW.
pub struct EtwFlowSource {
    /// Processes whose flows are reported. Shared with the tracing callback, which is the
    /// only reader, and replaced wholesale by [`FlowEventSource::watch`].
    watched: Arc<Mutex<Vec<Pid>>>,
    /// The running session, if any. `UserTrace::stop` consumes the value, hence the option.
    trace: Option<UserTrace>,
    /// The thread pumping the session's buffers.
    worker: Option<JoinHandle<()>>,
}

impl Default for EtwFlowSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EtwFlowSource {
    /// Creates a source that is not yet tracing.
    #[must_use]
    pub fn new() -> Self {
        Self {
            watched: Arc::new(Mutex::new(Vec::new())),
            trace: None,
            worker: None,
        }
    }
}

impl std::fmt::Debug for EtwFlowSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The session handle and its pump thread have no useful debug form, and the one
        // thing worth seeing is whether tracing is live.
        f.debug_struct("EtwFlowSource")
            .field("running", &self.trace.is_some())
            .finish_non_exhaustive()
    }
}

impl FlowEventSource for EtwFlowSource {
    fn start(&mut self, sink: FlowSink) -> Result<(), Error> {
        if self.trace.is_some() {
            return Ok(());
        }

        // Reclaim a session our own previous run may have left behind.
        let _ = stop_trace_by_name(SESSION_NAME);

        let watched = Arc::clone(&self.watched);
        let sink = Arc::new(Mutex::new(sink));
        let callback = move |record: &EventRecord, locator: &SchemaLocator| {
            dispatch(record, locator, &watched, &sink);
        };

        let provider = Provider::by_guid(TCPIP_PROVIDER_GUID)
            .any(KEYWORD_SEND_AND_RECEIVE_PATHS)
            .level(LEVEL_INFORMATIONAL)
            .add_filter(EventFilter::ByEventIds(vec![
                EVENT_UDP_SENT,
                EVENT_UDP_RECEIVED,
            ]))
            .add_callback(callback)
            .build();

        let (trace, handle) = UserTrace::new()
            .named(SESSION_NAME.to_owned())
            .enable(provider)
            .start()
            .map_err(map_trace_error)?;

        // `process_from_handle` blocks until the trace stops, so it gets its own thread.
        self.worker = Some(std::thread::spawn(move || {
            let _ = UserTrace::process_from_handle(handle);
        }));
        self.trace = Some(trace);
        Ok(())
    }

    fn watch(&self, pids: &[Pid]) {
        if let Ok(mut watched) = self.watched.lock() {
            watched.clear();
            watched.extend_from_slice(pids);
        }
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
    sink: &Mutex<FlowSink>,
) {
    let direction = match record.event_id() {
        EVENT_UDP_SENT => FlowDirection::Sent,
        EVENT_UDP_RECEIVED => FlowDirection::Received,
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
    };

    if let Ok(mut sink) = sink.lock() {
        sink(&event);
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
        assert!(format!("{source:?}").contains("running: false"));
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
        let mut source = EtwFlowSource::new();
        source.watch(&[Pid::new(std::process::id())]);

        let sink: FlowSink = Box::new(move |event: &FlowEvent| {
            let _ = tx.send(event.clone());
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

        let event = discovered.expect("a UDP peer of this process must be discovered");
        assert_eq!(event.pid, Pid::new(std::process::id()));
        assert_eq!(event.protocol, Protocol::Udp);
        assert_eq!(event.remote, target);
        assert_eq!(event.direction, FlowDirection::Sent);
        assert_eq!(
            event.bytes, 32,
            "the byte count must be the datagram's size"
        );
        assert!(event.local.ip().is_loopback());
    }
}
