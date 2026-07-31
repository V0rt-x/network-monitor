//! Tests that send a real `ClientHello` to a real server.
//!
//! Off by default — run them deliberately:
//!
//! ```text
//! cargo test -p nm-probes --features network-tests -- --nocapture
//! ```
//!
//! They check the one thing no unit test can: that the hand-assembled hello is well formed
//! enough that a production TLS stack answers it. The whole TLS prober rests on that, and it
//! is a claim about other people's servers, not about our code.
//!
//! Targets are public anycast resolvers used as documented constants, the same way the
//! platform crate uses them. Nothing observed here is recorded in the repository.

#![cfg(feature = "network-tests")]

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use nm_core::sample::ProbeOutcome;
use nm_core::target::TargetAddress;
use nm_probes::probe::{ProbeKind, ProbeTarget, Prober};
use nm_probes::tls::TlsHelloProber;

/// Well-known anycast resolvers that also serve DNS-over-HTTPS on 443.
const TARGETS: &[(&str, IpAddr)] = &[
    ("Cloudflare", IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
    ("Google", IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
    ("Quad9", IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
];

fn probe_target(ip: IpAddr) -> ProbeTarget {
    ProbeTarget::new(TargetAddress::with_port(ip, 443), Duration::from_secs(5))
}

#[tokio::test]
async fn real_servers_answer_the_hand_written_hello() {
    let prober = TlsHelloProber::new();
    assert_eq!(prober.kind(), ProbeKind::TlsHello);

    let mut measured = 0;
    for (name, ip) in TARGETS {
        let outcome = prober.probe(&probe_target(*ip)).await;
        // Deliberately not printing the address: only the role and the timing.
        println!("{name}: {outcome:?}");
        if let Ok(ProbeOutcome::Success(rtt)) = outcome {
            assert!(
                rtt.as_micros() > 0,
                "{name} answered in no time at all, which cannot be a real path"
            );
            measured += 1;
        }
    }

    assert!(
        measured > 0,
        "no server answered the hello; either the network is down or the message is malformed"
    );
}

/// The prober must not report a round trip for a port that speaks no TLS.
#[tokio::test]
async fn a_plain_dns_port_does_not_produce_a_tls_measurement() {
    // Port 53 speaks DNS over TCP. It accepts the connection, so a TCP-connect prober would
    // report a perfectly good number here; the TLS prober must not, because whatever comes
    // back is not a TLS record.
    let target = ProbeTarget::new(
        TargetAddress::with_port(TARGETS[0].1, 53),
        Duration::from_secs(5),
    );
    let outcome = TlsHelloProber::new().probe(&target).await;
    println!("DNS port: {outcome:?}");
    assert!(
        !matches!(outcome, Ok(ProbeOutcome::Success(_))),
        "a non-TLS service must never yield a TLS round-trip time"
    );
}
