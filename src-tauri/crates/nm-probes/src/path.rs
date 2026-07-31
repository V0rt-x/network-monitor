//! Walking outward towards a target that will not answer.
//!
//! The fallback of last resort. Many game servers — AWS- and GCP-hosted ones especially —
//! drop ICMP and expose no port, so every prober in this crate comes back with nothing. A
//! TTL-limited walk still learns how far packets get and which router last answered, which
//! turns "we cannot measure this" into "the path stops here". [`nm_core::path::classify`]
//! reads the result; this module only produces it.
//!
//! # Cost
//!
//! A walk is up to [`DEFAULT_MAX_HOPS`] probes, issued one after another. At the product's
//! global cap of 32 probes per second a single full walk spends most of a second's budget
//! for everything the app monitors. It is therefore not a periodic measurement: a caller
//! runs one when a target first goes silent, keeps the answer, and repeats it rarely. The
//! walk stops as early as it honestly can — the moment the target answers, the moment a
//! router says the destination is unreachable, or after [`DEFAULT_SILENT_LIMIT`] consecutive
//! silent hops once something has answered.
//!
//! Silence in the middle is stepped over rather than treated as the end, because routers
//! commonly decline to generate TTL-expired messages; stopping at the first gap would report
//! a failure several hops closer than the real one.

use std::sync::Arc;

use nm_core::path::{Hop, PathTrace};
use nm_core::sample::Rtt;
use nm_platform::icmp::{EchoOutcome, EchoRequest, IcmpProber};

use crate::probe::{ProbeKind, ProbeTarget};
use crate::Error;

/// How far out a walk goes before giving up.
///
/// Thirty is the conventional traceroute limit and comfortably exceeds any real internet
/// path; a walk that runs this far has established that the destination is unreachable, not
/// that the limit was too low.
pub const DEFAULT_MAX_HOPS: u8 = 30;

/// How many consecutive silent hops end a walk, once at least one hop has answered.
///
/// Three is enough to step over the usual run of routers that decline to answer while still
/// stopping promptly once the path is genuinely dead.
pub const DEFAULT_SILENT_LIMIT: u8 = 3;

/// Walks the path towards a target one TTL at a time.
pub struct PathProbe<P> {
    inner: Arc<P>,
    max_hops: u8,
    silent_limit: u8,
}

impl<P> PathProbe<P> {
    /// Wraps a platform ICMP implementation, with the default limits.
    pub fn new(inner: P) -> Self {
        Self {
            inner: Arc::new(inner),
            max_hops: DEFAULT_MAX_HOPS,
            silent_limit: DEFAULT_SILENT_LIMIT,
        }
    }

    /// Uses a different maximum distance.
    ///
    /// A hop limit of zero would issue no probes at all, so it is raised to one: a walk that
    /// silently measured nothing would be indistinguishable from a path that fails at the
    /// very first router.
    #[must_use]
    pub const fn with_max_hops(mut self, max_hops: u8) -> Self {
        self.max_hops = if max_hops == 0 { 1 } else { max_hops };
        self
    }

    /// Uses a different run of silent hops as the stopping point.
    #[must_use]
    pub const fn with_silent_limit(mut self, silent_limit: u8) -> Self {
        self.silent_limit = if silent_limit == 0 { 1 } else { silent_limit };
        self
    }
}

impl<P> PathProbe<P>
where
    P: IcmpProber + 'static,
{
    /// Walks outward until the target answers or the path stops.
    ///
    /// Each hop gets the target's full timeout, so a walk can take up to `max_hops` times
    /// that long. Dropping the returned future stops the walk between hops — the hop already
    /// in flight finishes on the blocking pool and its result is discarded.
    ///
    /// # Errors
    ///
    /// Returns an error if a probe could not be carried out at all. A hop that stays silent
    /// is recorded as a silent hop, not as an error: that is the walk's normal material.
    pub async fn trace(&self, target: &ProbeTarget) -> Result<PathTrace, Error> {
        let mut hops = Vec::new();
        let mut consecutive_silent = 0_u8;

        for ttl in 1..=self.max_hops {
            let mut request = EchoRequest::to(target.address.ip, target.timeout).with_ttl(ttl);
            if let Some(source) = target.source {
                request = request.from_source(source);
            }

            let inner = Arc::clone(&self.inner);
            let echoed = tokio::task::spawn_blocking(move || inner.echo(&request))
                .await
                .map_err(|_join_error| Error::ProbeTaskLost {
                    kind: ProbeKind::IcmpEcho,
                })??;

            match echoed {
                EchoOutcome::Replied { from, rtt } => {
                    hops.push(Hop::answered(ttl, from, Rtt::from_duration(rtt)));
                    return Ok(PathTrace::new(hops, true));
                }
                EchoOutcome::TtlExpired {
                    from: Some(from),
                    rtt,
                } => {
                    consecutive_silent = 0;
                    hops.push(Hop::answered(ttl, from, Rtt::from_duration(rtt)));
                }
                // Windows can report a TTL expiry without naming the router. The hop
                // happened, but it cannot be placed, so it counts as silent rather than as
                // an anonymous answer that would be diagnosed as if its position were known.
                EchoOutcome::TtlExpired { from: None, .. } | EchoOutcome::TimedOut => {
                    hops.push(Hop::silent(ttl));
                    consecutive_silent = consecutive_silent.saturating_add(1);
                    if consecutive_silent >= self.silent_limit && hops.iter().any(answered) {
                        break;
                    }
                }
                // A router has declared the destination unreachable. Nothing further out will
                // answer, and continuing would spend probes to learn nothing.
                EchoOutcome::Unreachable { from } => {
                    hops.push(Hop {
                        ttl,
                        address: from,
                        rtt: None,
                    });
                    break;
                }
            }
        }

        Ok(PathTrace::new(hops, false))
    }
}

fn answered(hop: &Hop) -> bool {
    hop.address.is_some()
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::sync::Mutex;
    use std::time::Duration;

    use nm_core::target::TargetAddress;
    use nm_platform::icmp::IcmpProber;
    use nm_platform::Error as PlatformError;

    use super::*;

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    fn target() -> ProbeTarget {
        ProbeTarget::new(
            TargetAddress::icmp(ip("203.0.113.200")),
            Duration::from_millis(500),
        )
    }

    /// Replays a fixed script of outcomes, one per hop, recording every request it was given.
    ///
    /// A hand-written double rather than a `mockall` mock: what matters here is the
    /// *sequence* of requests, which a script expresses far more directly than a chain of
    /// ordered expectations. Cloning shares the state, so a test can keep a handle on what
    /// the walk asked for after handing the double to the prober. A script that runs out
    /// keeps answering with silence, which is how the walk's own stopping rules get tested.
    #[derive(Clone)]
    struct ScriptedEchoer {
        script: Arc<Mutex<std::vec::IntoIter<Result<EchoOutcome, PlatformError>>>>,
        seen: Arc<Mutex<Vec<EchoRequest>>>,
    }

    impl ScriptedEchoer {
        fn new(script: Vec<Result<EchoOutcome, PlatformError>>) -> Self {
            Self {
                script: Arc::new(Mutex::new(script.into_iter())),
                seen: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn ttls(&self) -> Vec<u8> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter_map(|request| request.ttl)
                .collect()
        }

        fn sources(&self) -> Vec<Option<IpAddr>> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|request| request.source)
                .collect()
        }
    }

    impl IcmpProber for ScriptedEchoer {
        fn echo(&self, request: &EchoRequest) -> Result<EchoOutcome, PlatformError> {
            self.seen.lock().unwrap().push(request.clone());
            self.script
                .lock()
                .unwrap()
                .next()
                .unwrap_or(Ok(EchoOutcome::TimedOut))
        }
    }

    /// A prober whose hops answer as scripted, plus a handle on the recorded requests.
    fn walking(script: Vec<EchoOutcome>) -> (PathProbe<ScriptedEchoer>, ScriptedEchoer) {
        scripted(script.into_iter().map(Ok).collect())
    }

    /// The same, with `failure` raised on the hop after the scripted ones.
    fn failing(
        script: Vec<EchoOutcome>,
        failure: PlatformError,
    ) -> (PathProbe<ScriptedEchoer>, ScriptedEchoer) {
        let mut full: Vec<Result<EchoOutcome, PlatformError>> =
            script.into_iter().map(Ok).collect();
        full.push(Err(failure));
        scripted(full)
    }

    fn scripted(
        script: Vec<Result<EchoOutcome, PlatformError>>,
    ) -> (PathProbe<ScriptedEchoer>, ScriptedEchoer) {
        let echoer = ScriptedEchoer::new(script);
        (PathProbe::new(echoer.clone()), echoer)
    }

    fn expired(address: &str, millis: u64) -> EchoOutcome {
        EchoOutcome::TtlExpired {
            from: Some(ip(address)),
            rtt: Duration::from_millis(millis),
        }
    }

    const fn silent() -> EchoOutcome {
        EchoOutcome::TimedOut
    }

    #[tokio::test]
    async fn walks_outward_one_ttl_at_a_time() {
        let (probe, echoer) = walking(vec![
            expired("192.168.1.1", 1),
            expired("203.0.113.1", 6),
            EchoOutcome::Replied {
                from: ip("203.0.113.200"),
                rtt: Duration::from_millis(20),
            },
        ]);

        let trace = probe.trace(&target()).await.unwrap();
        assert_eq!(echoer.ttls(), vec![1, 2, 3]);
        assert!(trace.reached_target());
        assert_eq!(trace.hops().len(), 3);
        assert_eq!(trace.last_answering().unwrap().ttl, 3);
    }

    #[tokio::test]
    async fn stops_the_moment_the_target_answers() {
        let (probe, echoer) = walking(vec![EchoOutcome::Replied {
            from: ip("203.0.113.200"),
            rtt: Duration::from_millis(9),
        }]);

        let trace = probe.trace(&target()).await.unwrap();
        assert_eq!(
            echoer.ttls(),
            vec![1],
            "a reached target must not be walked past"
        );
        assert!(trace.reached_target());
    }

    #[tokio::test]
    async fn steps_over_silent_hops_in_the_middle() {
        // The behaviour the reality-check spike found necessary: a gap is not the end.
        let (probe, _) = walking(vec![
            expired("192.168.1.1", 1),
            silent(),
            silent(),
            expired("203.0.113.9", 40),
            silent(),
            silent(),
            silent(),
        ]);

        let trace = probe.trace(&target()).await.unwrap();
        assert!(!trace.reached_target());
        assert_eq!(
            trace.last_answering().unwrap().ttl,
            4,
            "the walk must reach past the first gap"
        );
    }

    #[tokio::test]
    async fn gives_up_after_a_run_of_silence() {
        let (probe, echoer) = walking(vec![expired("192.168.1.1", 1)]);
        let trace = probe.trace(&target()).await.unwrap();

        // One answer, then three silent hops end it — well short of the 30-hop limit.
        assert_eq!(echoer.ttls(), vec![1, 2, 3, 4]);
        assert_eq!(trace.hops().len(), 4);
    }

    #[tokio::test]
    async fn a_walk_that_never_hears_anything_still_runs_to_the_limit() {
        // Without an answer there is no basis for deciding the path has ended, so the run of
        // silence must not stop the walk early — locally filtered TTL messages look like
        // this, and so does a path whose first responding router is far out.
        let (probe, echoer) = walking(Vec::new());
        let trace = probe.trace(&target()).await.unwrap();

        assert_eq!(echoer.ttls().len(), usize::from(DEFAULT_MAX_HOPS));
        assert_eq!(trace.last_answering(), None);
    }

    #[tokio::test]
    async fn an_unreachable_report_ends_the_walk() {
        let (probe, echoer) = walking(vec![
            expired("192.168.1.1", 1),
            EchoOutcome::Unreachable {
                from: Some(ip("203.0.113.1")),
            },
        ]);

        let trace = probe.trace(&target()).await.unwrap();
        assert_eq!(
            echoer.ttls(),
            vec![1, 2],
            "nothing further out can answer, so further probes would be wasted"
        );
        assert!(!trace.reached_target());
        assert_eq!(trace.hops().len(), 2);
    }

    #[tokio::test]
    async fn an_anonymous_ttl_expiry_counts_as_silence() {
        // Windows can report the expiry without naming the router. The hop cannot be placed,
        // and diagnosing it as though it could would put the failure at the wrong distance.
        let (probe, _) = walking(vec![
            expired("192.168.1.1", 1),
            EchoOutcome::TtlExpired {
                from: None,
                rtt: Duration::from_millis(5),
            },
        ]);

        let trace = probe.trace(&target()).await.unwrap();
        assert_eq!(trace.last_answering().unwrap().ttl, 1);
        assert!(trace.hops()[1].address.is_none());
    }

    #[tokio::test]
    async fn a_local_failure_aborts_the_walk_instead_of_faking_a_dead_path() {
        let (probe, _) = failing(
            vec![expired("192.168.1.1", 1)],
            PlatformError::Icmp { code: 11_006 },
        );

        assert_eq!(
            probe.trace(&target()).await.unwrap_err(),
            Error::Platform(PlatformError::Icmp { code: 11_006 })
        );
    }

    #[tokio::test]
    async fn every_hop_carries_the_requested_egress_address() {
        let source = ip("192.0.2.9");
        let (probe, echoer) = walking(vec![expired("192.168.1.1", 1), expired("203.0.113.1", 5)]);
        let probed = target().from_source(source);
        probe.trace(&probed).await.unwrap();

        assert!(
            echoer.sources().iter().all(|seen| *seen == Some(source)),
            "a walk that drifted onto another interface would map a different path"
        );
    }

    #[tokio::test]
    async fn the_hop_limit_is_respected() {
        let (probe, echoer) = walking(Vec::new());
        let probe = probe.with_max_hops(5);
        probe.trace(&target()).await.unwrap();
        assert_eq!(echoer.ttls(), vec![1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn a_zero_hop_limit_still_probes_once() {
        let (probe, echoer) = walking(Vec::new());
        let probe = probe.with_max_hops(0);
        probe.trace(&target()).await.unwrap();
        assert_eq!(echoer.ttls(), vec![1]);
    }

    #[tokio::test]
    async fn the_silent_limit_is_respected() {
        let (probe, echoer) = walking(vec![expired("192.168.1.1", 1)]);
        let probe = probe.with_silent_limit(1);
        probe.trace(&target()).await.unwrap();
        assert_eq!(echoer.ttls(), vec![1, 2]);
    }
}
