//! What the app-monitor page is told about a monitored application.
//!
//! In `tests/` rather than in the module because `nm-app`'s library sets `test = false` —
//! an in-crate harness cannot start on Windows (see `tests.manifest`).
//!
//! These assert the two rules the page exists to honour: an application is a
//! *distribution*, never one colour, and the worst endpoints are the ones the user sees
//! first.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use nm_app::apps::AppMonitor;
use nm_app::{AppProcessView, AppView, HealthCountsView, HealthView, ProbeKindView, TransportView};
use nm_core::address::AddressPolicy;
use nm_core::endpoint::{AppId, EndpointKey, LifecyclePolicy};
use nm_core::health::HealthThresholds;
use nm_core::sample::{ProbeOutcome, ProbeSample, Rtt};
use nm_core::target::{TargetId, TargetRegistry};
use nm_probes::probe::ProbeKind;

const APP: AppId = AppId::new(1);
const PID: u32 = 4242;
const APP_ID: u32 = 1;

/// Enough samples to clear the minimum a verdict needs: one lost packet is not an outage,
/// and the thresholds refuse to judge below that.
const ENOUGH: u32 = 8;

fn monitor() -> (AppMonitor, TargetRegistry, Instant) {
    let mut monitor = AppMonitor::new(
        AddressPolicy::default(),
        LifecyclePolicy::default(),
        HealthThresholds::default(),
        Duration::from_secs(60),
    )
    .unwrap();
    monitor.monitor(APP).unwrap();
    (monitor, TargetRegistry::new(), Instant::now())
}

fn udp(last: u8) -> EndpointKey {
    EndpointKey::udp(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, last)),
        27_015,
    ))
}

fn registered(
    monitor: &mut AppMonitor,
    registry: &mut TargetRegistry,
    now: Instant,
) -> Vec<TargetId> {
    monitor
        .sweep(registry, now)
        .iter()
        .filter_map(|change| match change {
            nm_app::apps::TargetChange::Register { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

/// Fills an endpoint's history with one outcome repeated.
fn fill(monitor: &mut AppMonitor, id: TargetId, now: Instant, outcome: ProbeOutcome) {
    for step in 0..ENOUGH {
        monitor.record(
            id,
            ProbeSample::new(now + Duration::from_secs(step.into()), outcome),
        );
    }
}

fn view(monitor: &AppMonitor, now: Instant) -> AppView {
    AppView::of(
        APP_ID,
        "game.exe".to_owned(),
        vec![AppProcessView {
            pid: PID,
            name: "game.exe".to_owned(),
        }],
        &monitor.endpoints(APP, now),
    )
}

#[test]
fn the_worst_endpoints_come_first() {
    // Sorted by severity so the broken few are visible without hunting through a long list.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.observe(APP, udp(2), None, None, now).unwrap();
    monitor.observe(APP, udp(3), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    assert_eq!(ids.len(), 3);

    fill(
        &mut monitor,
        ids[0],
        now,
        ProbeOutcome::Success(Rtt::from_micros(9_000)),
    );
    fill(&mut monitor, ids[1], now, ProbeOutcome::Timeout);
    // The third is never probed, so nothing is known about it.

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    let order: Vec<HealthView> = view
        .endpoints
        .iter()
        .map(|endpoint| endpoint.health)
        .collect();
    assert_eq!(
        order,
        vec![HealthView::Unreachable, HealthView::Unknown, HealthView::Ok],
        "an endpoint nothing gets through to outranks one that was merely never measured"
    );
}

#[test]
fn an_application_is_a_distribution_and_not_one_colour() {
    // An application rolled up to its worst endpoint reads as "the game is broken" when the
    // game is fine; rolled up to its best it hides the failure the user came to find. So
    // there is no per-application verdict at all — only counts.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.observe(APP, udp(2), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);

    fill(
        &mut monitor,
        ids[0],
        now,
        ProbeOutcome::Success(Rtt::from_micros(9_000)),
    );
    fill(&mut monitor, ids[1], now, ProbeOutcome::Timeout);

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    assert_eq!(view.counts.ok, 1);
    assert_eq!(view.counts.unreachable, 1);
    assert_eq!(view.counts.degraded, 0);
    assert_eq!(view.id, APP_ID);
    assert_eq!(view.name, "game.exe");
    assert_eq!(
        view.processes
            .iter()
            .map(|process| process.pid)
            .collect::<Vec<_>>(),
        vec![PID],
        "the processes an application consists of are shown, not summarised"
    );
}

#[test]
fn ties_keep_a_stable_order() {
    // Two endpoints agreeing about their health must not reshuffle under the cursor from
    // one emission to the next.
    let (mut monitor, mut registry, now) = monitor();
    for last in [3, 1, 2] {
        monitor.observe(APP, udp(last), None, None, now).unwrap();
    }
    registered(&mut monitor, &mut registry, now);

    let keys: Vec<String> = view(&monitor, now)
        .endpoints
        .into_iter()
        .map(|endpoint| endpoint.key)
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}

#[test]
fn transport_is_part_of_an_endpoints_identity() {
    // A server reached over TCP for a lobby and UDP for play is two endpoints with two
    // independent fates, so their keys must differ.
    let (mut monitor, mut registry, now) = monitor();
    let address = udp(1).address;
    monitor
        .observe(APP, EndpointKey::udp(address), None, None, now)
        .unwrap();
    monitor
        .observe(APP, EndpointKey::tcp(address), None, None, now)
        .unwrap();
    registered(&mut monitor, &mut registry, now);

    let view = view(&monitor, now);
    assert_eq!(view.endpoints.len(), 2);
    let keys: Vec<&str> = view
        .endpoints
        .iter()
        .map(|endpoint| endpoint.key.as_str())
        .collect();
    assert_ne!(keys[0], keys[1]);
    assert!(view
        .endpoints
        .iter()
        .any(|endpoint| endpoint.transport == TransportView::Tcp));
    assert!(view
        .endpoints
        .iter()
        .any(|endpoint| endpoint.transport == TransportView::Udp));
}

#[test]
fn every_caveat_travels_with_the_number() {
    // A round-trip time means different things depending on how it was obtained. Each of
    // these is carried separately so the page can say so rather than showing a bare figure.
    let (mut monitor, mut registry, now) = monitor();
    let sentinel = EndpointKey::udp(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(198, 18, 0, 7)),
        443,
    ));
    let egress = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
    monitor
        .observe(APP, sentinel, Some(egress), Some(4_096), now)
        .unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    monitor.note_probe_state(ids[0], Some(ProbeKind::TlsHello), true, true);

    let view = view(&monitor, now);
    let endpoint = &view.endpoints[0];

    assert!(
        endpoint.tunnelled,
        "a figure through a tunnel is not a round trip to the server"
    );
    assert_eq!(endpoint.probe_kind, Some(ProbeKindView::TlsHello));
    assert!(endpoint.filtering_confirmed);
    assert!(endpoint.measurable);
    assert_eq!(endpoint.egress, Some(egress.to_string()));
    assert!(!endpoint.egress_conflict);
    assert_eq!(endpoint.recent_bytes, Some(4_096.0));
}

#[test]
fn a_game_server_carrying_traffic_is_never_called_unreachable() {
    // Found by running the app against a live game. Its match server answers no probe of
    // any kind — nothing listens on a game port but the game — while hundreds of kilobytes
    // cross it every half minute. Reported as "unreachable", that reads as "your game
    // server is down" about a server the user is playing on.
    let (mut monitor, mut registry, now) = monitor();
    monitor
        .observe(APP, udp(1), None, Some(630_000), now)
        .unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    let endpoint = &view.endpoints[0];

    assert_eq!(endpoint.health, HealthView::CarryingTraffic);
    assert_eq!(view.counts.carrying_traffic, 1);
    assert_eq!(view.counts.unreachable, 0);
    // And it claims nothing it did not measure: liveness is not latency.
    assert_eq!(endpoint.rtt_ms, None);
    assert_eq!(endpoint.recent_bytes, Some(630_000.0));
}

#[test]
fn a_silent_endpoint_with_no_traffic_is_still_unreachable() {
    // The complement: without passive evidence there is nothing to soften the verdict with,
    // and inventing life for an endpoint nothing has been seen crossing would be the same
    // failure in the opposite direction.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, Some(0), now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    assert_eq!(view.endpoints[0].health, HealthView::Unreachable);
}

#[test]
fn a_measured_endpoint_keeps_the_verdict_its_probes_earned() {
    // Traffic must not paper over a degraded path — that is the finding the user came for.
    let (mut monitor, mut registry, now) = monitor();
    monitor
        .observe(APP, udp(1), None, Some(630_000), now)
        .unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(
        &mut monitor,
        ids[0],
        now,
        ProbeOutcome::Success(Rtt::from_micros(400_000)),
    );

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    assert_eq!(view.endpoints[0].health, HealthView::Degraded);
}

#[test]
fn a_live_but_unmeasured_endpoint_sorts_below_the_broken_ones() {
    // It needs no action from the user, so it must not sit above an endpoint that does.
    let (mut monitor, mut registry, now) = monitor();
    monitor
        .observe(APP, udp(1), None, Some(630_000), now)
        .unwrap();
    monitor.observe(APP, udp(2), None, Some(0), now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);
    fill(&mut monitor, ids[1], now, ProbeOutcome::Timeout);

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    let order: Vec<HealthView> = view
        .endpoints
        .iter()
        .map(|endpoint| endpoint.health)
        .collect();
    assert_eq!(
        order,
        vec![HealthView::Unreachable, HealthView::CarryingTraffic]
    );
}

#[test]
fn unknown_throughput_is_not_reported_as_zero() {
    // Without a flow source there are no byte counters at all; showing zero for a busy game
    // would be a lie the user cannot see through.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    registered(&mut monitor, &mut registry, now);

    assert_eq!(view(&monitor, now).endpoints[0].recent_bytes, None);
}

#[test]
fn an_unmeasured_endpoint_reports_no_figures_at_all() {
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    registered(&mut monitor, &mut registry, now);

    let view = view(&monitor, now);
    let endpoint = &view.endpoints[0];
    assert_eq!(endpoint.rtt_ms, None);
    assert_eq!(endpoint.jitter_ms, None);
    assert_eq!(endpoint.loss_pct, None);
    assert_eq!(endpoint.health, HealthView::Unknown);
    assert!(endpoint.series_age_secs.is_empty());
}

#[test]
fn an_application_with_nothing_discovered_renders_empty_rather_than_missing() {
    let (monitor, _registry, now) = monitor();
    let view = view(&monitor, now);
    assert!(view.endpoints.is_empty());
    assert_eq!(view.counts, HealthCountsView::default());
}
