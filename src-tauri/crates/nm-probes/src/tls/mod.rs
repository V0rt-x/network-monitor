//! TLS probing: the only kind that measures an endpoint a local tunnel remaps.
//!
//! The spike (`docs/measurement-reality-check.md`) established why this exists. When a
//! router runs sing-box with fake-IP, an ICMP echo never enters the tunnel and a TCP connect
//! is answered by the tunnel itself in a fraction of a millisecond. The first byte a tunnel
//! cannot forge is the server's reply to a `ClientHello`: it speaks no TLS, so it must open
//! its own upstream connection and carry the exchange to the real destination.
//!
//! **This prober sends a `ClientHello` and times the first byte of the answer. It does not
//! complete a handshake.** That is a deliberate choice and it decides the crate's dependency
//! footprint: no TLS implementation, no cipher suite, no certificate parsing, no bundled
//! root store to age and no OpenSSL to find at build time on Linux. The quantity we need is
//! one round trip on the real path, and the first flight already contains it. Completing the
//! handshake would add a key exchange at both ends of every probe — the cost `PLAN.md` flags
//! as the reason tunnelled endpoints need long intervals — to learn something the product
//! never asks: whether the certificate validates.
//!
//! The round trip is measured from writing the hello to the first byte back, deliberately
//! excluding the TCP connect. For a direct endpoint the two legs are the same quantity; for
//! a tunnelled one the connect is local and means nothing, so leaving it out is what makes
//! the two comparable. What remains for a tunnelled endpoint is still an end-to-end figure
//! that includes the tunnel's own upstream setup, and must be labelled as such rather than
//! presented as a round trip to the server.

mod client_hello;

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use nm_core::sample::{ProbeOutcome, Rtt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::probe::{ProbeKind, ProbeTarget, Prober};
use crate::socket::{connect, local, Connected};
use crate::Error;
use client_hello::{classify_response, client_hello, HelloNonce, Response};

/// Measures the round trip from a `ClientHello` to the server's first answering byte.
#[derive(Debug)]
pub struct TlsHelloProber {
    nonce: NonceSource,
}

impl TlsHelloProber {
    /// Creates a prober.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nonce: NonceSource::new(),
        }
    }

    /// Connects, sends the hello, and waits for the first byte.
    ///
    /// Split out from [`Prober::probe`] so the deadline wraps the whole exchange rather than
    /// each leg: a target that stalls after accepting the connection must not get a fresh
    /// budget for the part that matters.
    async fn exchange(&self, target: &ProbeTarget) -> Result<ProbeOutcome, Error> {
        let mut stream = match connect(target, ProbeKind::TlsHello).await? {
            Connected::Settled(outcome) => return Ok(outcome),
            Connected::Stream { stream, .. } => stream,
        };

        // No SNI: an endpoint discovered from a connection table is an address, and the name
        // that produced it is only recoverable from the OS resolver cache — a later item in
        // `PLAN.md`. Servers answer a hello without SNI anyway, with a default certificate
        // or an alert, and either arrives one round trip later.
        let hello = client_hello(&self.nonce.next(), None);

        let started = Instant::now();
        if let Err(error) = stream.write_all(&hello).await {
            return classify_exchange_error(error.kind());
        }
        if let Err(error) = stream.flush().await {
            return classify_exchange_error(error.kind());
        }

        let mut first = [0_u8; 1];
        match stream.read(&mut first).await {
            // The peer accepted the connection and closed it without answering. The path
            // carries packets; the exchange did not happen.
            Ok(0) => Ok(ProbeOutcome::Unreachable),
            Ok(_) => {
                let elapsed = started.elapsed();
                Ok(match classify_response(first[0]) {
                    Response::Tls => ProbeOutcome::Success(Rtt::from_duration(elapsed)),
                    Response::NotTls => ProbeOutcome::Blocked,
                })
            }
            Err(error) => classify_exchange_error(error.kind()),
        }
    }
}

impl Default for TlsHelloProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Prober for TlsHelloProber {
    fn kind(&self) -> ProbeKind {
        ProbeKind::TlsHello
    }

    async fn probe(&self, target: &ProbeTarget) -> Result<ProbeOutcome, Error> {
        match tokio::time::timeout(target.timeout, self.exchange(target)).await {
            Ok(result) => result,
            Err(_elapsed) => Ok(ProbeOutcome::Timeout),
        }
    }
}

/// Decides what a failure *after* the connection was established says about the path.
///
/// Read differently from the same failures during `connect`, because the position in the
/// exchange changes their meaning. A connection accepted and then torn down the moment a
/// `ClientHello` appears is the signature of something on the path objecting to this
/// exchange — the classic shape of TLS-level filtering. That is [`ProbeOutcome::Blocked`]:
/// no packet was lost, so it must stay out of the loss ratio, and no round trip was
/// measured, so there is nothing to report but the fact that this probe kind cannot work
/// here.
fn classify_exchange_error(reason: io::ErrorKind) -> Result<ProbeOutcome, Error> {
    match reason {
        io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => Ok(ProbeOutcome::Blocked),
        io::ErrorKind::TimedOut => Ok(ProbeOutcome::Timeout),
        io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => {
            Ok(ProbeOutcome::Unreachable)
        }
        other => Err(local(&io::Error::from(other), ProbeKind::TlsHello)),
    }
}

/// Supplies the random-looking fields of each hello.
///
/// Not a cryptographic generator and it does not need to be: the handshake never reaches the
/// point where these bytes would derive anything. They vary only so that a client sending a
/// byte-identical hello every few seconds does not stand out on the wire — which matters for
/// an audience whose traffic is inspected. Seeded from the wall clock, which is legitimate
/// here precisely because this is not a measurement.
#[derive(Debug)]
struct NonceSource {
    state: AtomicU64,
}

impl NonceSource {
    fn new() -> Self {
        // Assembled from the two parts rather than from `as_nanos`, whose `u128` would need
        // a truncating cast; a seed only has to differ between runs.
        let seed =
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0x2545_F491_4F6C_DD1D, |since| {
                    since
                        .as_secs()
                        .wrapping_mul(1_000_000_000)
                        .wrapping_add(u64::from(since.subsec_nanos()))
                });
        Self {
            state: AtomicU64::new(seed),
        }
    }

    /// Produces the next hello's nonce fields.
    fn next(&self) -> HelloNonce {
        let mut state = self.state.fetch_add(GOLDEN_GAMMA, Ordering::Relaxed);
        HelloNonce {
            random: fill(&mut state),
            session_id: fill(&mut state),
            key_share: fill(&mut state),
        }
    }
}

/// The odd increment `splitmix64` advances its state by.
const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// Fills 32 bytes from four `splitmix64` outputs.
fn fill(state: &mut u64) -> [u8; 32] {
    let mut out = [0_u8; 32];
    for chunk in out.chunks_exact_mut(8) {
        chunk.copy_from_slice(&splitmix64(state).to_be_bytes());
    }
    out
}

/// One step of `splitmix64`.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(GOLDEN_GAMMA);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    use nm_core::target::TargetAddress;
    use tokio::net::TcpListener;

    use super::*;

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn target(port: u16) -> ProbeTarget {
        ProbeTarget::new(
            TargetAddress::with_port(LOOPBACK, port),
            Duration::from_secs(5),
        )
    }

    /// Accepts one connection, reads the whole hello, and answers with `reply`.
    ///
    /// A stand-in for a TLS server: the prober only ever looks at the first byte, so
    /// returning the right first byte exercises exactly the code under test without a cipher
    /// implementation on either side.
    ///
    /// Draining the hello completely is not tidiness — closing a socket with unread data in
    /// its receive buffer makes TCP send a reset instead of a graceful close, and the prober
    /// would rightly read that as interference. A real server reads the hello before deciding
    /// anything, so the stub must too.
    async fn serving(reply: &'static [u8]) -> u16 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            let mut header = [0_u8; 5];
            stream.read_exact(&mut header).await.unwrap();
            let body = usize::from(u16::from_be_bytes([header[3], header[4]]));
            stream.read_exact(&mut vec![0_u8; body]).await.unwrap();

            if !reply.is_empty() {
                stream.write_all(reply).await.unwrap();
                stream.flush().await.unwrap();
                // Hold the connection open until the prober has read its answer.
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
        port
    }

    #[test]
    fn reports_its_own_kind() {
        assert_eq!(TlsHelloProber::new().kind(), ProbeKind::TlsHello);
    }

    #[tokio::test]
    async fn a_server_hello_is_a_round_trip_time() {
        // 0x16: a handshake record, which is what a ServerHello arrives in.
        let port = serving(&[0x16, 0x03, 0x03, 0x00, 0x40]).await;
        let outcome = TlsHelloProber::new().probe(&target(port)).await.unwrap();
        assert!(
            matches!(outcome, ProbeOutcome::Success(_)),
            "expected a measurement, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn an_alert_measures_the_path_just_as_well() {
        // A server that dislikes the hello still had to receive it, so the round trip is
        // real. Refusing to record it would discard a perfectly good measurement.
        let port = serving(&[0x15, 0x03, 0x03, 0x00, 0x02]).await;
        let outcome = TlsHelloProber::new().probe(&target(port)).await.unwrap();
        assert!(matches!(outcome, ProbeOutcome::Success(_)));
    }

    #[tokio::test]
    async fn an_answer_that_is_not_tls_is_not_reported_as_a_round_trip() {
        // An interception box answering with an HTTP error page. Something replied, but the
        // timing belongs to that something and not to the destination.
        let port = serving(b"HTTP/1.1 403 Forbidden\r\n").await;
        assert_eq!(
            TlsHelloProber::new().probe(&target(port)).await.unwrap(),
            ProbeOutcome::Blocked
        );
    }

    #[tokio::test]
    async fn a_peer_that_closes_without_answering_is_not_loss() {
        let port = serving(b"").await;
        assert_eq!(
            TlsHelloProber::new().probe(&target(port)).await.unwrap(),
            ProbeOutcome::Unreachable
        );
    }

    #[tokio::test]
    async fn a_silent_peer_runs_out_the_deadline() {
        // Accepts the connection and never answers: the deadline must cover the exchange,
        // not just the connect, or this would hang forever.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(stream);
        });

        let probed = ProbeTarget::new(
            TargetAddress::with_port(LOOPBACK, port),
            Duration::from_millis(200),
        );
        assert_eq!(
            TlsHelloProber::new().probe(&probed).await.unwrap(),
            ProbeOutcome::Timeout
        );
    }

    #[tokio::test]
    async fn a_target_without_a_port_is_refused_rather_than_guessed() {
        let probed = ProbeTarget::new(TargetAddress::icmp(LOOPBACK), Duration::from_secs(1));
        assert_eq!(
            TlsHelloProber::new().probe(&probed).await.unwrap_err(),
            Error::PortRequired {
                kind: ProbeKind::TlsHello
            }
        );
    }

    #[test]
    fn a_reset_after_the_hello_reads_as_filtering_rather_than_loss() {
        // The classic shape of TLS-level blocking: the connection is accepted, then torn
        // down the instant a ClientHello appears. Counting it as a lost packet would report
        // the wrong problem; counting it as a round trip would report a fictional one.
        for reason in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::BrokenPipe,
        ] {
            assert_eq!(
                classify_exchange_error(reason).unwrap(),
                ProbeOutcome::Blocked,
                "{reason:?}"
            );
        }
    }

    #[test]
    fn routing_failures_mid_exchange_stay_measurements() {
        assert_eq!(
            classify_exchange_error(io::ErrorKind::TimedOut).unwrap(),
            ProbeOutcome::Timeout
        );
        for reason in [
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkUnreachable,
        ] {
            assert_eq!(
                classify_exchange_error(reason).unwrap(),
                ProbeOutcome::Unreachable,
                "{reason:?}"
            );
        }
    }

    #[test]
    fn unclassified_failures_mid_exchange_stay_errors() {
        assert_eq!(
            classify_exchange_error(io::ErrorKind::OutOfMemory).unwrap_err(),
            Error::LocalFailure {
                kind: ProbeKind::TlsHello,
                reason: io::ErrorKind::OutOfMemory
            }
        );
    }

    #[test]
    fn consecutive_nonces_differ_in_every_field() {
        let source = NonceSource::new();
        let first = source.next();
        let second = source.next();
        assert_ne!(first.random, second.random);
        assert_ne!(first.session_id, second.session_id);
        assert_ne!(first.key_share, second.key_share);
    }

    #[test]
    fn the_three_fields_of_one_nonce_differ_from_each_other() {
        let nonce = NonceSource::new().next();
        assert_ne!(nonce.random, nonce.session_id);
        assert_ne!(nonce.session_id, nonce.key_share);
        assert_ne!(nonce.random, nonce.key_share);
    }

    #[test]
    fn nonces_do_not_repeat_over_a_long_run() {
        // A short cycle would put the same hello on the wire again and again, which is the
        // anomaly the varying bytes exist to avoid.
        let source = NonceSource::new();
        let seen: HashSet<[u8; 32]> = (0..2_000).map(|_| source.next().random).collect();
        assert_eq!(seen.len(), 2_000);
    }

    #[test]
    fn a_nonce_is_not_mostly_one_byte() {
        // Catches a generator that fills correctly but produces a constant, which would look
        // fine in every other test here.
        let nonce = NonceSource::new().next();
        let distinct: HashSet<u8> = nonce.random.into_iter().collect();
        assert!(distinct.len() > 8, "only {} distinct bytes", distinct.len());
    }
}
