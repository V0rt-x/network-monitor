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
use nm_core::endpoint::{AppId, EndpointKey, LifecyclePolicy};
use nm_core::health::HealthThresholds;
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
