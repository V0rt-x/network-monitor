//! ICMP echo probing, lifted from the blocking platform trait onto the runtime.
//!
//! `nm-platform`'s [`IcmpProber`] blocks its thread deliberately (that module explains
//! why). This is where the decision is paid for: every echo runs on tokio's blocking pool,
//! so the runtime's worker threads stay free for the rest of the engine.
//!
//! Cancellation is honest about its limits. Dropping the returned future does not stop the
//! blocking call — `spawn_blocking` has no way to interrupt one — so the task runs to
//! completion, bounded by the request's own timeout, and its result is discarded. At the
//! global cap of 32 probes per second against timeouts of a few seconds, the pool sees a
//! few dozen mostly-idle threads and never approaches its default limit.

use std::sync::Arc;

use async_trait::async_trait;
use nm_core::address::AddressClass;
use nm_core::forgery;
use nm_core::sample::{ProbeOutcome, Rtt};
use nm_platform::icmp::{EchoOutcome, EchoRequest, IcmpProber};

use crate::probe::{ProbeKind, ProbeTarget, Prober};
use crate::Error;

/// Measures a target with ICMP echo requests.
///
/// Generic over the platform implementation so the engine's behaviour is testable with a
/// mock on any operating system, including the parts only Windows can actually run.
pub struct IcmpEchoProber<P> {
    // `Arc` because a blocking task must own what it touches: the closure handed to
    // `spawn_blocking` outlives the borrow of `&self`.
    inner: Arc<P>,
}

impl<P> IcmpEchoProber<P> {
    /// Wraps a platform ICMP implementation.
    pub fn new(inner: P) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl<P> Prober for IcmpEchoProber<P>
where
    P: IcmpProber + 'static,
{
    fn kind(&self) -> ProbeKind {
        ProbeKind::IcmpEcho
    }

    async fn probe(&self, target: &ProbeTarget) -> Result<ProbeOutcome, Error> {
        // ICMP has no ports, so `target.address.port` is ignored rather than rejected: the
        // same endpoint is often registered with the port its flow uses, and refusing it
        // here would make the caller strip a field the protocol simply does not have.
        let mut request = EchoRequest::to(target.address.ip, target.timeout);
        if let Some(source) = target.source {
            request = request.from_source(source);
        }

        let inner = Arc::clone(&self.inner);
        let echoed = tokio::task::spawn_blocking(move || inner.echo(&request))
            .await
            .map_err(|_join_error| Error::ProbeTaskLost {
                kind: ProbeKind::IcmpEcho,
            })?;

        Ok(to_probe_outcome(echoed?, target.class))
    }
}

/// Turns a platform echo result into a measurement.
///
/// Kept separate from the async plumbing so the mapping — the part that decides what the
/// user is told — is a pure function with its own tests.
fn to_probe_outcome(echo: EchoOutcome, class: AddressClass) -> ProbeOutcome {
    match echo {
        // A reply whose hop limit was never decremented crossed no router, so for an
        // address that ought to be far away it was answered on this machine. Reporting its
        // timing as a round trip to the endpoint is the failure this whole path exists to
        // prevent — a tunnel answering for the entire internet in under a millisecond.
        EchoOutcome::Replied {
            hop_limit: Some(hop_limit),
            ..
        } if forgery::reply_was_answered_locally(class, hop_limit) => ProbeOutcome::AnsweredLocally,
        EchoOutcome::Replied { rtt, .. } => ProbeOutcome::Success(Rtt::from_duration(rtt)),
        // This prober never sets a TTL, so an expiry means the system default ran out
        // before the packet arrived: a routing loop, or a path longer than 128 hops.
        // Either way the destination was not reached and nothing was measured about it.
        // A *path* probe reads the same event as its success case, which is why path
        // probing is a separate concern rather than a flag on this type.
        EchoOutcome::TtlExpired { .. } | EchoOutcome::Unreachable { .. } => {
            ProbeOutcome::Unreachable
        }
        EchoOutcome::TimedOut => ProbeOutcome::Timeout,
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::time::Duration;

    use mockall::mock;
    use nm_core::target::TargetAddress;
    use nm_platform::icmp::DEFAULT_PAYLOAD_LEN;
    use nm_platform::Error as PlatformError;

    use super::*;

    mock! {
        Echoer {}
        impl IcmpProber for Echoer {
            fn echo(&self, request: &EchoRequest) -> Result<EchoOutcome, PlatformError>;
        }
    }

    fn ip() -> IpAddr {
        "203.0.113.7".parse().unwrap()
    }

    fn target() -> ProbeTarget {
        ProbeTarget::new(TargetAddress::icmp(ip()), Duration::from_millis(750))
    }

    /// Builds a prober whose single echo returns `echo`.
    fn answering(echo: EchoOutcome) -> IcmpEchoProber<MockEchoer> {
        let mut echoer = MockEchoer::new();
        echoer.expect_echo().times(1).returning(move |_| Ok(echo));
        IcmpEchoProber::new(echoer)
    }

    #[test]
    fn reports_its_own_kind() {
        assert_eq!(
            IcmpEchoProber::new(MockEchoer::new()).kind(),
            ProbeKind::IcmpEcho
        );
    }

    /// A reply that crossed a plausible number of routers on its way back.
    fn replied(rtt: Duration) -> EchoOutcome {
        EchoOutcome::Replied {
            from: ip(),
            rtt,
            hop_limit: Some(57),
        }
    }

    #[tokio::test]
    async fn a_reply_becomes_a_round_trip_time() {
        assert_eq!(
            answering(replied(Duration::from_micros(12_345)))
                .probe(&target())
                .await
                .unwrap(),
            ProbeOutcome::Success(Rtt::from_micros(12_345))
        );
    }

    #[tokio::test]
    async fn a_reply_that_crossed_no_router_is_refused_as_a_measurement() {
        // The finding this exists for: a TUN client answers echo requests for the whole
        // internet itself, in under a millisecond, with the machine's own initial hop
        // limit. Reported as success it would say the user's connection is perfect.
        let echo = EchoOutcome::Replied {
            from: ip(),
            rtt: Duration::from_micros(700),
            hop_limit: Some(128),
        };
        assert_eq!(
            answering(echo).probe(&target()).await.unwrap(),
            ProbeOutcome::AnsweredLocally
        );
    }

    #[tokio::test]
    async fn a_backend_that_reports_no_hop_limit_still_measures() {
        // The proof is an extra, not a precondition. A platform that cannot report the
        // field must keep measuring rather than treat every reply as suspect.
        let echo = EchoOutcome::Replied {
            from: ip(),
            rtt: Duration::from_millis(9),
            hop_limit: None,
        };
        assert_eq!(
            answering(echo).probe(&target()).await.unwrap(),
            ProbeOutcome::Success(Rtt::from_micros(9_000))
        );
    }

    #[tokio::test]
    async fn a_lan_peer_answering_from_zero_hops_away_is_a_real_measurement() {
        // The mirror of the rule: a private address really is on this segment, and calling
        // its reply forged would be false. The class is what separates the two.
        let echo = EchoOutcome::Replied {
            from: "192.168.1.1".parse().unwrap(),
            rtt: Duration::from_micros(400),
            hop_limit: Some(64),
        };
        let private = ProbeTarget::new(
            TargetAddress::icmp("192.168.1.1".parse().unwrap()),
            Duration::from_millis(750),
        )
        .of_class(nm_core::address::AddressClass::Private);

        assert_eq!(
            answering(echo).probe(&private).await.unwrap(),
            ProbeOutcome::Success(Rtt::from_micros(400))
        );
    }

    #[tokio::test]
    async fn silence_is_recorded_as_loss() {
        assert_eq!(
            answering(EchoOutcome::TimedOut)
                .probe(&target())
                .await
                .unwrap(),
            ProbeOutcome::Timeout
        );
    }

    #[tokio::test]
    async fn an_unreachable_report_is_not_loss() {
        // A router answering "unreachable" proves the path carries packets, so counting it
        // as a dropped probe would understate the connection's health.
        let echo = EchoOutcome::Unreachable { from: None };
        assert_eq!(
            answering(echo).probe(&target()).await.unwrap(),
            ProbeOutcome::Unreachable
        );
    }

    #[tokio::test]
    async fn an_unrequested_ttl_expiry_is_not_loss_either() {
        let echo = EchoOutcome::TtlExpired {
            from: None,
            rtt: Duration::from_millis(4),
        };
        assert_eq!(
            answering(echo).probe(&target()).await.unwrap(),
            ProbeOutcome::Unreachable,
            "the hop's round-trip time says nothing about the target and must not be reported as one"
        );
    }

    #[tokio::test]
    async fn a_local_failure_stays_an_error_instead_of_becoming_packet_loss() {
        // The rule the whole error split exists for: our machine failing must never be
        // displayed as someone else's dropped packet.
        let mut echoer = MockEchoer::new();
        echoer
            .expect_echo()
            .times(1)
            .returning(|_| Err(PlatformError::Icmp { code: 11_006 }));

        assert_eq!(
            IcmpEchoProber::new(echoer)
                .probe(&target())
                .await
                .unwrap_err(),
            Error::Platform(PlatformError::Icmp { code: 11_006 })
        );
    }

    #[tokio::test]
    async fn the_request_carries_the_targets_routing_constraints() {
        let source: IpAddr = "192.0.2.9".parse().unwrap();
        let mut echoer = MockEchoer::new();
        echoer
            .expect_echo()
            .withf(move |request| {
                request.target == ip()
                    && request.source == Some(source)
                    && request.ttl.is_none()
                    && request.timeout == Duration::from_millis(750)
                    && request.payload_len == DEFAULT_PAYLOAD_LEN
            })
            .times(1)
            .returning(|_| Ok(EchoOutcome::TimedOut));

        let probed = target().from_source(source);
        IcmpEchoProber::new(echoer).probe(&probed).await.unwrap();
    }

    #[tokio::test]
    async fn a_port_on_the_target_is_ignored_rather_than_refused() {
        // Endpoints are discovered from TCP/UDP flows, so they arrive carrying a port that
        // ICMP has no use for. Rejecting them would push the caller into stripping fields.
        let mut echoer = MockEchoer::new();
        echoer
            .expect_echo()
            .withf(|request| request.target == ip())
            .times(1)
            .returning(|_| Ok(EchoOutcome::TimedOut));

        let probed = ProbeTarget::new(
            TargetAddress::with_port(ip(), 443),
            Duration::from_millis(750),
        );
        assert_eq!(
            IcmpEchoProber::new(echoer).probe(&probed).await.unwrap(),
            ProbeOutcome::Timeout
        );
    }
}
