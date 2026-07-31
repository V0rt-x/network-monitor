//! Tests that send real ICMP packets.
//!
//! Off by default — run them deliberately:
//!
//! ```text
//! cargo test -p nm-platform --features network-tests -- --nocapture
//! ```
//!
//! They exist to check the things a mock cannot: that the IP Helper call is wired up
//! correctly, that an unspecified source address is accepted, and which of the two ways
//! Windows can report a TTL expiry actually occurs. Results are printed as well as
//! asserted, because this file doubles as the instrument for the measurement-model
//! reality check in `PLAN.md`.

#![cfg(all(windows, feature = "network-tests"))]

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::Duration;

use nm_platform::icmp::{windows::WindowsIcmpProber, EchoOutcome, EchoRequest, IcmpProber};

const TIMEOUT: Duration = Duration::from_secs(2);

/// Reserved for documentation (RFC 5737 TEST-NET-1) and routed nowhere, so it is a
/// dependable black hole.
const BLACK_HOLE: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);

/// Cloudflare's public resolver: anycast, ubiquitous, and answers ICMP.
const PUBLIC_ANYCAST: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);

fn prober() -> WindowsIcmpProber {
    WindowsIcmpProber
}

fn echo(target: Ipv4Addr) -> EchoRequest {
    EchoRequest::to(IpAddr::V4(target), TIMEOUT)
}

/// The local address the OS would use to reach the internet.
///
/// A connected UDP socket sends nothing; it only asks the routing table which interface
/// would be chosen, which is exactly the address a source-bound probe should use.
fn preferred_local_address() -> std::io::Result<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("1.1.1.1:53")?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(address) => Ok(address),
        IpAddr::V6(address) => Err(std::io::Error::other(format!(
            "expected an IPv4 route, got {address}"
        ))),
    }
}

#[test]
fn loopback_replies_immediately() {
    let outcome = prober()
        .echo(&echo(Ipv4Addr::LOCALHOST))
        .expect("the loopback echo must be carried out");
    println!("loopback: {outcome:?}");

    match outcome {
        EchoOutcome::Replied { from, rtt } => {
            assert_eq!(from, IpAddr::V4(Ipv4Addr::LOCALHOST));
            assert!(rtt < Duration::from_millis(100), "loopback took {rtt:?}");
        }
        other => panic!("expected a reply from the loopback, got {other:?}"),
    }
}

#[test]
fn an_unspecified_source_lets_the_os_choose_a_route() {
    // The implementation passes 0.0.0.0 when no source is requested; this proves the API
    // accepts that rather than rejecting it as an invalid source.
    let outcome = prober()
        .echo(&echo(PUBLIC_ANYCAST))
        .expect("the echo must be carried out");
    println!("unspecified source -> 1.1.1.1: {outcome:?}");
    assert!(
        matches!(outcome, EchoOutcome::Replied { .. }),
        "1.1.1.1 did not reply: {outcome:?}"
    );
}

#[test]
fn binding_the_source_address_still_reaches_the_target() {
    let source = preferred_local_address().expect("the host must have an IPv4 route");
    println!("preferred local address: {source}");

    let request = echo(PUBLIC_ANYCAST).from_source(IpAddr::V4(source));
    let outcome = prober()
        .echo(&request)
        .expect("the echo must be carried out");
    println!("source-bound -> 1.1.1.1: {outcome:?}");

    assert!(
        matches!(outcome, EchoOutcome::Replied { .. }),
        "source-bound probe failed: {outcome:?}"
    );
}

#[test]
fn a_ttl_of_one_reveals_the_first_hop() {
    // The foundation of path probing. This also records which of the two reporting paths
    // Windows takes, since a TTL expiry can arrive either as a reply structure or through
    // GetLastError, and only the former carries the hop's address.
    let request = echo(PUBLIC_ANYCAST).with_ttl(1);
    let outcome = prober()
        .echo(&request)
        .expect("the echo must be carried out");
    println!("ttl=1 -> 1.1.1.1: {outcome:?}");

    match outcome {
        EchoOutcome::TtlExpired { from, rtt } => {
            println!("  first hop: {from:?} in {rtt:?}");
            assert!(rtt < TIMEOUT);
        }
        EchoOutcome::Replied { from, .. } => {
            panic!("the target answered at ttl=1, which means {from} is one hop away");
        }
        other => panic!("expected a TTL expiry, got {other:?}"),
    }
}

#[test]
fn a_growing_ttl_walks_the_path_outward() {
    // What the path probe does for real: keep increasing the TTL until the destination
    // itself answers, recording every hop that identified itself along the way. Silent
    // hops are expected — plenty of routers decline to send TTL-exceeded — so the walk
    // must step over them rather than stop.
    const MAX_HOPS: u8 = 30;

    let mut hops = Vec::new();
    let mut silent = 0_u32;
    let mut reached = None;

    for ttl in 1..=MAX_HOPS {
        let request = echo(PUBLIC_ANYCAST).with_ttl(ttl);
        let outcome = prober()
            .echo(&request)
            .expect("the echo must be carried out");
        println!("ttl={ttl}: {outcome:?}");
        match outcome {
            EchoOutcome::Replied { from, .. } => {
                reached = Some((ttl, from));
                break;
            }
            EchoOutcome::TtlExpired {
                from: Some(hop), ..
            } => hops.push(hop),
            _ => silent += 1,
        }
    }

    let (ttl, address) = reached.expect("1.1.1.1 was not reached within 30 hops");
    println!(
        "reached {address} at ttl={ttl}; {} named hops, {silent} silent",
        hops.len()
    );

    assert!(
        hops.len() >= 2,
        "a path probe needs identifiable hops to report where a path dies, found {hops:?}"
    );
    assert!(
        hops.iter().all(|hop| *hop != address),
        "an intermediate hop must not be the destination itself"
    );
}

#[test]
fn a_black_hole_times_out_rather_than_erroring() {
    // Silence is a measurement, not a failure: it must come back as a timeout so the
    // statistics count it as packet loss.
    let outcome = prober()
        .echo(&echo(BLACK_HOLE))
        .expect("a silent target is not an error");
    println!("black hole 192.0.2.1: {outcome:?}");
    assert!(
        matches!(
            outcome,
            EchoOutcome::TimedOut | EchoOutcome::Unreachable { .. }
        ),
        "expected silence or an unreachable report, got {outcome:?}"
    );
}

#[test]
fn ipv6_fails_loudly_instead_of_reporting_a_false_outcome() {
    let request = EchoRequest::to(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), TIMEOUT);
    let error = prober()
        .echo(&request)
        .expect_err("IPv6 is not implemented yet");
    println!("ipv6: {error}");
}
