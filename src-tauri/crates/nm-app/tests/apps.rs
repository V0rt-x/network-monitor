//! Per-application endpoint monitoring: discovery becoming probe targets.
//!
//! In `tests/` rather than in the module because `nm-app`'s library sets `test = false` —
//! an in-crate harness cannot start on Windows (see `tests.manifest`). Everything here
//! therefore goes through the public API, which is no loss: what these assert is the
//! contract the rest of the app depends on.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use nm_app::apps::{AppMonitor, TargetChange};
use nm_core::address::AddressPolicy;
use nm_core::edge::{EdgeReading, PathQuality};
use nm_core::endpoint::{AppId, EndpointKey, LifecyclePolicy};
use nm_core::health::{Health, HealthThresholds};
use nm_core::path::{Hop, PathTrace};
use nm_core::sample::{ProbeOutcome, ProbeSample, Rtt};
use nm_core::target::{TargetAddress, TargetId, TargetRegistry, TargetTag};
use nm_probes::probe::ProbeKind;

const APP: AppId = AppId::new(1);
const OTHER: AppId = AppId::new(2);

fn monitor() -> AppMonitor {
    AppMonitor::new(
        AddressPolicy::default(),
        LifecyclePolicy::default(),
        HealthThresholds::default(),
        Duration::from_secs(60),
    )
    .unwrap()
}

/// A monitor already watching one application, with the registry the session shares
/// between every feature that probes.
fn watching() -> (AppMonitor, TargetRegistry, Instant) {
    let mut monitor = monitor();
    monitor.monitor(APP).unwrap();
    (monitor, TargetRegistry::new(), Instant::now())
}

fn socket(last: u8, port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, last)), port)
}

fn udp(last: u8) -> EndpointKey {
    EndpointKey::udp(socket(last, 27_015))
}

fn local(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
}

fn registrations(changes: &[TargetChange]) -> Vec<TargetId> {
    changes
        .iter()
        .filter_map(|change| match change {
            TargetChange::Register { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

#[test]
fn a_discovered_endpoint_becomes_a_probe_target() {
    let (mut monitor, mut registry, now) = watching();
    monitor
        .observe(APP, udp(1), Some(local(9)), Some(64), now)
        .unwrap();

    let changes = monitor.sweep(&mut registry, now);

    assert_eq!(changes.len(), 1);
    let TargetChange::Register {
        address,
        source,
        interval,
        ..
    } = &changes[0]
    else {
        panic!("expected a registration, got {:?}", changes[0]);
    };
    assert_eq!(address.ip, udp(1).address.ip());
    assert_eq!(address.port, Some(27_015));
    assert_eq!(
        *source,
        Some(local(9)),
        "a probe must egress the same way the application's flow does"
    );
    assert_eq!(*interval, Duration::from_secs(1));
}

#[test]
fn a_known_endpoint_is_not_registered_twice() {
    let (mut monitor, mut registry, now) = watching();
    monitor
        .observe(APP, udp(1), Some(local(9)), None, now)
        .unwrap();
    monitor.sweep(&mut registry, now);

    // Discovery re-observes the same endpoint on every poll; the probe engine must not
    // hear about it again.
    for step in 1..10 {
        let later = now + Duration::from_secs(step);
        monitor
            .observe(APP, udp(1), Some(local(9)), None, later)
            .unwrap();
        assert!(
            monitor.sweep(&mut registry, later).is_empty(),
            "a steady endpoint must produce no commands at all"
        );
    }
}

#[test]
fn an_endpoint_that_goes_away_is_unregistered() {
    let (mut monitor, mut registry, now) = watching();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, now));
    assert_eq!(registered.len(), 1);

    let changes = monitor.sweep(&mut registry, now + Duration::from_secs(121));

    assert_eq!(
        changes,
        vec![TargetChange::Unregister { id: registered[0] }]
    );
    assert_eq!(monitor.endpoint_count(APP), 0);
}

#[test]
fn demotion_stretches_the_interval_instead_of_dropping_the_endpoint() {
    let (mut monitor, mut registry, now) = watching();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.sweep(&mut registry, now);

    // Long enough to go idle, not long enough to be forgotten.
    let later = now + Duration::from_secs(30);
    let changes = monitor.sweep(&mut registry, later);

    assert_eq!(changes.len(), 1);
    assert!(matches!(
        changes[0],
        TargetChange::SetInterval { interval, .. } if interval == Duration::from_secs(10)
    ));
    assert_eq!(
        monitor.endpoint_count(APP),
        1,
        "an idle endpoint is still monitored"
    );
}

#[test]
fn one_address_two_applications_is_probed_once() {
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    monitor.monitor(APP).unwrap();
    monitor.monitor(OTHER).unwrap();
    let now = Instant::now();

    monitor
        .observe(APP, udp(1), Some(local(9)), None, now)
        .unwrap();
    monitor
        .observe(OTHER, udp(1), Some(local(9)), None, now)
        .unwrap();

    let changes = monitor.sweep(&mut registry, now);
    assert_eq!(
        registrations(&changes).len(),
        1,
        "one address must not become two probes"
    );
}

#[test]
fn one_application_letting_go_does_not_stop_the_others_measurement() {
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    monitor.monitor(APP).unwrap();
    monitor.monitor(OTHER).unwrap();
    let now = Instant::now();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.observe(OTHER, udp(1), None, None, now).unwrap();
    monitor.sweep(&mut registry, now);

    let changes = monitor.forget(&mut registry, APP);

    assert!(
        changes.is_empty(),
        "the endpoint is still in use, so nothing may be unregistered"
    );
    assert_eq!(monitor.endpoint_count(OTHER), 1);
}

#[test]
fn the_last_application_letting_go_releases_the_target() {
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    monitor.monitor(APP).unwrap();
    monitor.monitor(OTHER).unwrap();
    let now = Instant::now();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.observe(OTHER, udp(1), None, None, now).unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, now));

    monitor.forget(&mut registry, APP);
    let changes = monitor.forget(&mut registry, OTHER);

    assert_eq!(
        changes,
        vec![TargetChange::Unregister { id: registered[0] }]
    );
}

#[test]
fn a_shared_target_is_probed_at_the_shortest_interval_anyone_wants() {
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    monitor.monitor(APP).unwrap();
    monitor.monitor(OTHER).unwrap();
    let start = Instant::now();

    // One application keeps using the endpoint; the other has gone quiet on it.
    monitor.observe(APP, udp(1), None, None, start).unwrap();
    monitor.observe(OTHER, udp(1), None, None, start).unwrap();
    monitor.sweep(&mut registry, start);

    let later = start + Duration::from_secs(30);
    monitor.observe(APP, udp(1), None, None, later).unwrap();
    let changes = monitor.sweep(&mut registry, later);

    assert!(
        changes.is_empty(),
        "the busy application keeps the target at full rate: {changes:?}"
    );
}

#[test]
fn a_shared_target_slows_down_once_every_user_has_lost_interest() {
    // Regression: the target's interval used to be folded against the last one requested,
    // so it could only ever shrink and a shared endpoint could never be demoted at all.
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    monitor.monitor(APP).unwrap();
    monitor.monitor(OTHER).unwrap();
    let start = Instant::now();

    monitor.observe(APP, udp(1), None, None, start).unwrap();
    monitor.observe(OTHER, udp(1), None, None, start).unwrap();
    monitor.sweep(&mut registry, start);

    // Neither application uses it again; both go idle.
    let later = start + Duration::from_secs(30);
    let changes = monitor.sweep(&mut registry, later);

    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(matches!(
        changes[0],
        TargetChange::SetInterval { interval, .. } if interval == Duration::from_secs(10)
    ));
}

#[test]
fn different_egress_routes_to_one_endpoint_are_disclosed() {
    // A per-process accelerator on one application and not the other. One probe cannot
    // represent both routes, so the disagreement is recorded rather than averaged.
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    monitor.monitor(APP).unwrap();
    monitor.monitor(OTHER).unwrap();
    let now = Instant::now();

    monitor
        .observe(APP, udp(1), Some(local(9)), None, now)
        .unwrap();
    monitor
        .observe(OTHER, udp(1), Some(local(10)), None, now)
        .unwrap();
    monitor.sweep(&mut registry, now);

    let conflicted = monitor.endpoints(OTHER, now);
    assert!(conflicted[0].egress_conflict);
    let owner = monitor.endpoints(APP, now);
    assert!(!owner[0].egress_conflict);
}

#[test]
fn a_re_routed_flow_takes_the_new_egress_address() {
    // The user turns a VPN on mid-session.
    let (mut monitor, mut registry, now) = watching();
    monitor
        .observe(APP, udp(1), Some(local(9)), None, now)
        .unwrap();
    monitor.sweep(&mut registry, now);

    let later = now + Duration::from_secs(1);
    monitor
        .observe(APP, udp(1), Some(local(10)), None, later)
        .unwrap();

    assert_eq!(monitor.endpoints(APP, later)[0].source, Some(local(10)));
}

#[test]
fn a_re_routed_flow_takes_its_probe_with_it() {
    // Regression: the new egress address reached the *report* but never the probe engine,
    // so turning on a VPN relabelled the endpoint while the probe carried on measuring the
    // route the application had stopped using — which is the one comparison this product
    // exists to make.
    let (mut monitor, mut registry, now) = watching();
    monitor
        .observe(APP, udp(1), Some(local(9)), None, now)
        .unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, now));

    let later = now + Duration::from_secs(1);
    monitor
        .observe(APP, udp(1), Some(local(10)), None, later)
        .unwrap();
    let changes = monitor.sweep(&mut registry, later);

    assert_eq!(
        changes,
        vec![TargetChange::SetSource {
            id: registered[0],
            source: Some(local(10))
        }]
    );
}

#[test]
fn a_steady_endpoint_never_restates_its_egress() {
    // The other half of the same regression: a source that has not moved must produce no
    // commands at all, or every endpoint would re-bind once a second.
    let (mut monitor, mut registry, now) = watching();
    monitor
        .observe(APP, udp(1), Some(local(9)), None, now)
        .unwrap();
    monitor.sweep(&mut registry, now);

    for step in 1..6 {
        let later = now + Duration::from_secs(step);
        monitor
            .observe(APP, udp(1), Some(local(9)), None, later)
            .unwrap();
        assert!(monitor.sweep(&mut registry, later).is_empty());
    }
}

#[test]
fn one_application_never_conflicts_with_itself() {
    // Regression: an endpoint first discovered without an egress address — a connection
    // table row for an unbound socket — was compared against the value recorded at
    // registration when its address later became known, and a single monitored application
    // was told its route disagreed with a second application that did not exist.
    let (mut monitor, mut registry, now) = watching();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.sweep(&mut registry, now);

    let later = now + Duration::from_secs(1);
    monitor
        .observe(APP, udp(1), Some(local(9)), None, later)
        .unwrap();
    monitor.sweep(&mut registry, later);

    let report = &monitor.endpoints(APP, later)[0];
    assert!(
        !report.egress_conflict,
        "there is only one application; there is nothing to conflict with"
    );
    assert_eq!(report.source, Some(local(9)));
}

#[test]
fn a_disclosure_is_withdrawn_once_the_other_application_stops() {
    // A warning that outlives its cause is a warning nobody reads.
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    monitor.monitor(APP).unwrap();
    monitor.monitor(OTHER).unwrap();
    let now = Instant::now();
    monitor
        .observe(APP, udp(1), Some(local(9)), None, now)
        .unwrap();
    monitor
        .observe(OTHER, udp(1), Some(local(10)), None, now)
        .unwrap();
    monitor.sweep(&mut registry, now);
    assert!(monitor.endpoints(OTHER, now)[0].egress_conflict);

    monitor.forget(&mut registry, APP);
    let later = now + Duration::from_secs(1);
    monitor
        .observe(OTHER, udp(1), Some(local(10)), None, later)
        .unwrap();
    monitor.sweep(&mut registry, later);

    assert!(!monitor.endpoints(OTHER, later)[0].egress_conflict);
}

#[test]
fn an_unmonitored_application_is_refused() {
    let mut monitor = monitor();
    let error = monitor.observe(APP, udp(1), None, None, Instant::now());
    assert!(error.is_err());
    assert_eq!(monitor.app_count(), 0);
}

#[test]
fn the_application_cap_is_enforced() {
    let mut monitor = monitor();
    for raw in 0..5 {
        monitor.monitor(AppId::new(raw)).unwrap();
    }
    assert!(monitor.monitor(AppId::new(99)).is_err());
    assert_eq!(monitor.app_count(), 5);
}

#[test]
fn a_measurement_reaches_every_application_using_the_target() {
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    monitor.monitor(APP).unwrap();
    monitor.monitor(OTHER).unwrap();
    let now = Instant::now();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.observe(OTHER, udp(1), None, None, now).unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, now));

    monitor.record(
        registered[0],
        ProbeSample::new(now, ProbeOutcome::Success(Rtt::from_micros(12_000))),
    );

    for app in [APP, OTHER] {
        let stats = &monitor.endpoints(app, now)[0].stats;
        assert_eq!(
            stats.rtt.map(|rtt| rtt.mean_ms),
            Some(12.0),
            "one probe answers for every application sharing the endpoint"
        );
    }
}

#[test]
fn probe_state_is_carried_onto_the_endpoint() {
    let (mut monitor, mut registry, now) = watching();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, now));

    monitor.note_probe_state(registered[0], Some(ProbeKind::TlsHello), true, true);

    let report = &monitor.endpoints(APP, now)[0];
    assert_eq!(report.probe_kind, Some(ProbeKind::TlsHello));
    assert!(report.filtering_confirmed);
    assert!(report.measurable);
}

#[test]
fn an_unmeasurable_endpoint_says_so_rather_than_vanishing() {
    let (mut monitor, mut registry, now) = watching();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, now));

    monitor.note_unmeasurable(registered[0]);

    let report = &monitor.endpoints(APP, now)[0];
    assert!(!report.measurable);
    assert_eq!(report.probe_kind, None);
    assert_eq!(
        monitor.endpoint_count(APP),
        1,
        "an endpoint nothing can measure is still an endpoint the user is talking to"
    );
}

#[test]
fn endpoints_of_one_application_hold_independent_states() {
    // The rule Phase 4 exists to honour: an application is a distribution, never one
    // colour. A blocked voice server must not make a working game server look broken.
    let (mut monitor, mut registry, now) = watching();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.observe(APP, udp(2), None, None, now).unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, now));
    assert_eq!(registered.len(), 2);

    // Enough probes each to clear the minimum a verdict needs: one lost packet is not an
    // outage, and the thresholds refuse to judge below that.
    for step in 0..8 {
        let at = now + Duration::from_secs(step);
        monitor.record(
            registered[0],
            ProbeSample::new(at, ProbeOutcome::Success(Rtt::from_micros(9_000))),
        );
        monitor.record(registered[1], ProbeSample::new(at, ProbeOutcome::Timeout));
    }

    let reports = monitor.endpoints(APP, now + Duration::from_secs(8));
    assert_eq!(reports.len(), 2);
    assert!(reports[0].stats.rtt.is_some());
    assert_eq!(reports[1].stats.rtt, None);
    assert_ne!(reports[0].health, reports[1].health);
}

#[test]
fn throughput_stays_unknown_when_nothing_counts_it() {
    let (mut monitor, mut registry, now) = watching();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.observe(APP, udp(2), None, Some(512), now).unwrap();
    monitor.sweep(&mut registry, now);

    let reports = monitor.endpoints(APP, now);
    assert_eq!(reports[0].recent_bytes, None);
    assert_eq!(reports[1].recent_bytes, Some(512));
}

#[test]
fn a_tunnelled_endpoint_is_labelled() {
    let (mut monitor, mut registry, now) = watching();
    // Inside the FakeIP sentinel range a local tunnel remaps.
    let sentinel = EndpointKey::udp(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(198, 18, 0, 7)),
        443,
    ));
    monitor.observe(APP, sentinel, None, None, now).unwrap();
    monitor.sweep(&mut registry, now);

    assert!(monitor.endpoints(APP, now)[0].tunnelled);
}

#[test]
fn an_address_another_feature_already_probes_is_adopted_not_registered_twice() {
    // The registry is shared with the baselines, because one probe engine means one global
    // budget and one identifier space. Re-registering an address the dashboard already
    // measures would reset its fallback chain and its failure history.
    let (mut monitor, mut registry, now) = watching();
    let shared = TargetAddress::with_port(udp(1).address.ip(), udp(1).address.port());
    let existing = registry
        .insert(shared, TargetTag::DomesticBaseline)
        .unwrap();

    monitor
        .observe(APP, udp(1), Some(local(9)), None, now)
        .unwrap();
    let changes = monitor.sweep(&mut registry, now);

    assert!(registrations(&changes).is_empty());
    assert_eq!(
        changes,
        vec![TargetChange::SetInterval {
            id: existing,
            interval: Duration::from_secs(1)
        }]
    );
    assert!(
        monitor.endpoints(APP, now)[0].egress_conflict,
        "the existing probe was bound for another feature, so it cannot be claimed to \
         follow this application's route"
    );
}

#[test]
fn letting_go_of_an_adopted_address_leaves_the_other_feature_probing() {
    let (mut monitor, mut registry, now) = watching();
    let shared = TargetAddress::with_port(udp(1).address.ip(), udp(1).address.port());
    registry
        .insert(shared, TargetTag::DomesticBaseline)
        .unwrap();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.sweep(&mut registry, now);

    let changes = monitor.forget(&mut registry, APP);

    assert!(
        changes.is_empty(),
        "unregistering would stop a baseline the dashboard is still showing: {changes:?}"
    );
    assert!(registry.find(shared).is_some());
}

#[test]
fn forgetting_an_application_with_nothing_discovered_is_quiet() {
    let (mut monitor, mut registry, _) = watching();
    assert!(monitor.forget(&mut registry, APP).is_empty());
    assert!(!monitor.is_monitored(APP));
}

#[test]
fn a_sweep_with_nothing_monitored_asks_for_nothing() {
    let mut monitor = monitor();
    let mut registry = TargetRegistry::new();
    assert!(monitor.sweep(&mut registry, Instant::now()).is_empty());
}

// --- The path edge: measuring the route to an endpoint that answers nothing ---------------

/// Stand-ins for routers on the way to an endpoint, in increasing distance.
///
/// The documentation ranges cannot play a hop: the address policy classifies them as
/// unusable, and a hop not worth probing is passed over. These are well-known public
/// resolver addresses used as routable constants — nothing here sends a packet anywhere, and
/// none of them was observed on any machine.
const HOME_ROUTER: &str = "192.168.1.1";
const NEAR_HOP: &str = "1.1.1.1";
const MID_HOP: &str = "8.8.8.8";
const DEEP_HOP: &str = "9.9.9.9";
const OTHER_HOP: &str = "8.8.4.4";

fn hop(raw: &str) -> IpAddr {
    raw.parse().expect("a literal address")
}

/// A walk that never reached its target, over `(address, milliseconds)` pairs from TTL 1.
fn trace(hops: &[(&str, u32)]) -> PathTrace {
    let hops = hops
        .iter()
        .enumerate()
        .map(|(index, (address, millis))| {
            let ttl = u8::try_from(index + 1).unwrap();
            Hop::answered(ttl, hop(address), Rtt::from_micros(millis * 1_000))
        })
        .collect();
    PathTrace::new(hops, false)
}

/// The ordinary route to a match server: the home router, then three public hops.
fn typical_route() -> PathTrace {
    trace(&[
        (HOME_ROUTER, 1),
        (NEAR_HOP, 6),
        (MID_HOP, 9),
        (DEEP_HOP, 40),
    ])
}

fn walk_requests(changes: &[TargetChange]) -> Vec<TargetId> {
    changes
        .iter()
        .filter_map(|change| match change {
            TargetChange::WalkNow { id } => Some(*id),
            _ => None,
        })
        .collect()
}

fn unregistrations(changes: &[TargetChange]) -> Vec<TargetId> {
    changes
        .iter()
        .filter_map(|change| match change {
            TargetChange::Unregister { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// One monitored endpoint carrying traffic, whose probe kinds are all exhausted.
///
/// The state a game's match server settles into within seconds of a match starting: it
/// answers no echo, no handshake and no hello, because nothing listens on a game port but
/// the game, while every packet of the match crosses it.
fn silent_but_busy() -> (
    AppMonitor,
    TargetRegistry,
    Instant,
    TargetId,
    Vec<TargetChange>,
) {
    let (mut monitor, mut registry, now) = watching();
    monitor
        .observe(APP, udp(1), Some(local(9)), Some(64_000), now)
        .unwrap();
    let endpoint = registrations(&monitor.sweep(&mut registry, now))[0];

    // No probe kind left, and the route still worth measuring — the probe engine's own
    // report of an endpoint that has fallen through to walking its path.
    monitor.note_probe_state(endpoint, None, false, true);
    let changes = monitor.sweep(&mut registry, now);
    (monitor, registry, now, endpoint, changes)
}

#[test]
fn an_endpoint_that_runs_out_of_probe_kinds_asks_for_its_route_to_be_walked() {
    let (_, _, _, endpoint, changes) = silent_but_busy();
    assert_eq!(walk_requests(&changes), vec![endpoint]);
}

#[test]
fn an_endpoint_that_can_still_be_probed_is_left_alone() {
    // A path edge is three probes a second. Nothing gets one while there is something better
    // to measure than a router short of it.
    let (mut monitor, mut registry, now) = watching();
    monitor
        .observe(APP, udp(1), Some(local(9)), Some(64_000), now)
        .unwrap();
    let endpoint = registrations(&monitor.sweep(&mut registry, now))[0];
    monitor.note_probe_state(endpoint, Some(ProbeKind::IcmpEcho), false, true);

    assert!(walk_requests(&monitor.sweep(&mut registry, now)).is_empty());
    assert!(monitor.endpoints(APP, now)[0].path.is_none());
}

#[test]
fn a_walked_route_becomes_probes_along_the_applications_own_egress() {
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();

    let changes = monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now);

    let hops: Vec<(IpAddr, Option<IpAddr>, Duration)> = changes
        .iter()
        .filter_map(|change| match change {
            TargetChange::Register {
                address,
                source,
                interval,
                ..
            } => Some((address.ip, *source, *interval)),
            _ => None,
        })
        .collect();
    assert_eq!(
        hops.iter().map(|(ip, _, _)| *ip).collect::<Vec<_>>(),
        vec![hop(NEAR_HOP), hop(MID_HOP), hop(DEEP_HOP)],
        "the home router is not worth probing, and the rest of the walk is"
    );
    for (_, source, interval) in &hops {
        assert_eq!(
            *source,
            Some(local(9)),
            "a hop measured off the application's route measures the wrong route"
        );
        assert_eq!(*interval, Duration::from_secs(1));
    }
    assert!(
        changes
            .iter()
            .all(|change| !matches!(change, TargetChange::Register { address, .. } if address.port.is_some())),
        "a router is not a service: there is no port for a connecting probe to aim at"
    );
}

#[test]
fn a_hops_measurements_become_the_endpoints_path_figure() {
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();
    let changes = monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now);
    let hops = registrations(&changes);

    for step in 0..6 {
        let at = now + Duration::from_secs(step);
        monitor.record(
            hops[0],
            ProbeSample::new(at, ProbeOutcome::Success(Rtt::from_micros(6_000))),
        );
        monitor.record(
            hops[1],
            ProbeSample::new(at, ProbeOutcome::Success(Rtt::from_micros(9_000))),
        );
        monitor.record(
            hops[2],
            ProbeSample::new(at, ProbeOutcome::Success(Rtt::from_micros(40_000))),
        );
    }

    let report = &monitor.endpoints(APP, now + Duration::from_secs(6))[0];
    let path = report
        .path
        .as_ref()
        .expect("the endpoint holds a path edge");
    assert_eq!(path.quality, PathQuality::Ok);
    assert_eq!(path.rtt_ms(), Some(40.0));
    assert_eq!(
        path.reported_hop().map(|reported| reported.ttl),
        Some(4),
        "the figure belongs to the deepest hop that answers, and says which one that is"
    );
    assert_eq!(
        report.stats.rtt, None,
        "the endpoint itself still answers nothing, and the path must not stand in for it"
    );
}

#[test]
fn a_silent_endpoint_is_on_the_chart_by_its_path_and_not_by_a_round_trip() {
    // The endpoint the whole product exists to watch has no round trip to draw. Leaving it
    // off the chart would hide it; drawing its path figure as a round trip would be the lie
    // this product was built not to tell. So it appears, on its own series.
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();
    let hops =
        registrations(&monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now));
    for step in 0..6 {
        let at = now + Duration::from_secs(step);
        for id in &hops {
            monitor.record(
                *id,
                ProbeSample::new(at, ProbeOutcome::Success(Rtt::from_micros(40_000))),
            );
        }
    }

    let report = &monitor.endpoints(APP, now + Duration::from_secs(6))[0];

    assert!(
        report.chart_rtt_ms.iter().all(Option::is_none),
        "nothing measured a round trip to this endpoint, so its own line stays empty"
    );
    assert!(
        report.chart_path_ms.iter().any(Option::is_some),
        "and the route to it is what puts it on the chart at all"
    );
    assert_eq!(report.chart_path_ms.len(), monitor.chart_ages_secs().len());
}

#[test]
fn a_silent_endpoint_carrying_traffic_is_never_reported_as_unmeasured() {
    // Once the chain has fallen through to the route, no future probe will say anything more
    // about the endpoint — so "not measured yet" has stopped being the honest word, and the
    // traffic crossing it is the answer.
    let (monitor, _, now, _, _) = silent_but_busy();
    assert_eq!(
        monitor.endpoints(APP, now)[0].health,
        Health::CarryingTraffic
    );
}

#[test]
fn only_the_busiest_endpoint_of_an_application_is_given_a_path_edge() {
    // An edge is three probes a second, which `PLAN.md` allots to the one endpoint that
    // matters. A game with four silent servers would otherwise spend the product's whole
    // allowance on a single application.
    let (mut monitor, mut registry, now) = watching();
    monitor
        .observe(APP, udp(1), Some(local(9)), Some(1_000), now)
        .unwrap();
    monitor
        .observe(APP, udp(2), Some(local(9)), Some(64_000), now)
        .unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, now));
    for id in &registered {
        monitor.note_probe_state(*id, None, false, true);
    }

    let asked = walk_requests(&monitor.sweep(&mut registry, now));
    assert_eq!(asked.len(), 1, "one edge per application, no more");

    let reports = monitor.endpoints(APP, now);
    let with_edge: Vec<&EndpointKey> = reports
        .iter()
        .filter(|report| report.path.is_some())
        .map(|report| &report.key)
        .collect();
    assert_eq!(
        with_edge,
        vec![&udp(2)],
        "the busiest endpoint is the match server; the rest keep their single probe"
    );
}

#[test]
fn an_edge_that_moves_to_another_endpoint_gives_up_its_hops_first() {
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();
    let hops =
        registrations(&monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now));
    assert_eq!(hops.len(), 3);

    // A second endpoint takes over as the busiest, and runs out of probe kinds too.
    let later = now + Duration::from_secs(1);
    monitor
        .observe(APP, udp(2), Some(local(9)), Some(500_000), later)
        .unwrap();
    let registered = registrations(&monitor.sweep(&mut registry, later));
    monitor.note_probe_state(registered[0], None, false, true);
    let changes = monitor.sweep(&mut registry, later);

    let released = unregistrations(&changes);
    assert_eq!(
        released.len(),
        3,
        "the old edge's probes must stop before the new one's start: {changes:?}"
    );
    assert!(hops.iter().all(|id| released.contains(id)));

    let reports = monitor.endpoints(APP, later);
    let held: Vec<&EndpointKey> = reports
        .iter()
        .filter(|report| report.path.is_some())
        .map(|report| &report.key)
        .collect();
    assert_eq!(held, vec![&udp(2)], "the edge followed the traffic");
}

#[test]
fn a_rewalk_that_finds_the_same_route_asks_the_probe_engine_for_nothing() {
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();
    monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now);

    let later = now + Duration::from_secs(300);
    let changes = monitor.note_path_trace(&mut registry, endpoint, &typical_route(), later);

    assert!(
        changes.is_empty(),
        "an unchanged route must cost nothing to confirm: {changes:?}"
    );
}

#[test]
fn a_route_that_moved_swaps_only_the_hop_that_changed() {
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();
    let before =
        registrations(&monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now));

    let later = now + Duration::from_secs(300);
    let moved = trace(&[
        (HOME_ROUTER, 1),
        (NEAR_HOP, 6),
        (MID_HOP, 9),
        (OTHER_HOP, 44),
    ]);
    let changes = monitor.note_path_trace(&mut registry, endpoint, &moved, later);

    assert_eq!(unregistrations(&changes), vec![before[2]]);
    assert_eq!(registrations(&changes).len(), 1);
}

#[test]
fn a_hop_that_answers_no_probe_of_ours_is_released() {
    // Answering a time-to-live expiry does not oblige a router to answer an echo addressed
    // to it. One that will not is worth nothing to the edge, and its slot is better empty.
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();
    let hops =
        registrations(&monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now));

    monitor.note_probe_state(hops[2], None, false, true);
    let changes = monitor.sweep(&mut registry, now);

    assert_eq!(unregistrations(&changes), vec![hops[2]]);
    let report = &monitor.endpoints(APP, now)[0];
    assert_eq!(report.path.as_ref().map(|path| path.hops.len()), Some(2));
}

#[test]
fn forgetting_an_application_stops_the_hops_it_was_probing() {
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();
    let hops =
        registrations(&monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now));

    let released = unregistrations(&monitor.forget(&mut registry, APP));

    for id in hops {
        assert!(
            released.contains(&id),
            "a hop outlived its reason for existing"
        );
        assert!(registry.get(id).is_none());
    }
}

#[test]
fn a_walk_for_an_endpoint_with_no_edge_registers_nothing() {
    // A trace can arrive for an endpoint the sweep has since decided is not the one worth
    // three probes a second. Its hops have nowhere to live, and registering them would spend
    // budget nobody decided to spend.
    let (mut monitor, mut registry, now) = watching();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let endpoint = registrations(&monitor.sweep(&mut registry, now))[0];

    assert!(monitor
        .note_path_trace(&mut registry, endpoint, &typical_route(), now)
        .is_empty());
}

#[test]
fn a_hop_that_is_already_a_baseline_is_measured_once_and_answers_both() {
    // The same router can be a baseline target in its own right. The shared registry is what
    // makes that one probe rather than two, and re-registering it would reset a fallback
    // chain and a failure history the dashboard owns.
    let (mut monitor, mut registry, now, endpoint, _) = silent_but_busy();
    let shared = TargetAddress::icmp(hop(DEEP_HOP));
    let existing = registry.insert(shared, TargetTag::ForeignBaseline).unwrap();

    let changes = monitor.note_path_trace(&mut registry, endpoint, &typical_route(), now);

    assert!(
        !registrations(&changes).contains(&existing),
        "the address was already being probed: {changes:?}"
    );
    for step in 0..6 {
        let at = now + Duration::from_secs(step);
        monitor.record(
            existing,
            ProbeSample::new(at, ProbeOutcome::Success(Rtt::from_micros(40_000))),
        );
    }
    let report = &monitor.endpoints(APP, now + Duration::from_secs(6))[0];
    assert_eq!(
        report.path.as_ref().and_then(EdgeReading::rtt_ms),
        Some(40.0),
        "the baseline's own measurement is the edge's measurement"
    );

    // And letting go of the application must not stop the dashboard's probe.
    let released = unregistrations(&monitor.forget(&mut registry, APP));
    assert!(!released.contains(&existing));
    assert!(registry.find(shared).is_some());
}
