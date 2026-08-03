//! Tests for the state behind the service status page.
//!
//! In `tests/` rather than in a `#[cfg(test)]` module because `nm-app`'s library target
//! sets `test = false`; see `tests.manifest` for the Windows loader constraint behind it.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use nm_app::services::{ResolvedService, ServiceEndpoint, ServiceGroup};
use nm_app::status::{ServiceMonitor, CHECK_INTERVAL, TIMELINE_POINTS};
use nm_app::{CheckMarkView, HealthView, ServiceView};
use nm_core::sample::{ProbeOutcome, ProbeSample, Rtt};
use nm_core::status::StatusThresholds;
use nm_core::target::{TargetAddress, TargetId, TargetRegistry, TargetTag};

/// A documentation-range address, so nothing here can name a real machine.
fn ip(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
}

fn endpoint(key: &str, address: Option<TargetAddress>) -> ServiceEndpoint {
    ServiceEndpoint {
        key: key.to_owned(),
        written_address: format!("{key}.example.net"),
        address,
    }
}

fn service(id: &str, endpoints: Vec<ServiceEndpoint>) -> ResolvedService {
    ResolvedService {
        id: id.to_owned(),
        label: id.to_owned(),
        group: ServiceGroup::GamingPlatform,
        probe_kind: None,
        endpoints,
    }
}

fn ok(ms: u32) -> ProbeOutcome {
    ProbeOutcome::Success(Rtt::from_micros(ms * 1_000))
}

/// A monitor holding one service of `count` endpoints, with a handle for each.
///
/// The registry comes back with it: handles are only meaningful within the registry that
/// issued them, which the running app guarantees by having exactly one.
fn one_service(count: u8) -> (ServiceMonitor, Vec<TargetId>, TargetRegistry) {
    let mut registry = TargetRegistry::new();
    let mut endpoints = Vec::new();
    let mut handles = Vec::new();
    for index in 0..count {
        let address = TargetAddress::with_port(ip(index + 1), 443);
        endpoints.push(endpoint(&format!("svc/e{index}"), Some(address)));
        handles.push(Some(
            registry.insert(address, TargetTag::StatusService).unwrap(),
        ));
    }

    let mut monitor = ServiceMonitor::new(StatusThresholds::default());
    monitor.add(&service("svc", endpoints), &handles).unwrap();
    (monitor, handles.into_iter().flatten().collect(), registry)
}

/// The moment `ago` before `now`, saturating at the clock's origin.
fn before(now: Instant, ago: Duration) -> Instant {
    now.checked_sub(ago).unwrap_or(now)
}

/// Feeds one check per outcome, oldest first, ending `now`.
fn feed(monitor: &mut ServiceMonitor, id: TargetId, now: Instant, outcomes: &[ProbeOutcome]) {
    let total = u32::try_from(outcomes.len()).unwrap();
    for (index, outcome) in outcomes.iter().enumerate() {
        let back = total - u32::try_from(index).unwrap() - 1;
        monitor.record(
            id,
            ProbeSample::new(before(now, CHECK_INTERVAL * back), *outcome),
        );
    }
}

fn only(monitor: &ServiceMonitor, now: Instant) -> ServiceView {
    monitor
        .snapshot(now)
        .services
        .into_iter()
        .next()
        .expect("the monitor holds one service")
}

#[test]
fn a_service_nothing_has_checked_yet_is_unknown_rather_than_reachable() {
    let (monitor, _, _registry) = one_service(1);
    let view = only(&monitor, Instant::now());

    assert_eq!(view.verdict, HealthView::Unknown);
    assert_eq!(view.last_checked_secs, None);
    assert!(view.endpoints[0].checks.is_empty());
    assert!(view.endpoints[0].rtt_ms.is_none());
}

#[test]
fn an_endpoint_whose_name_never_resolved_stays_listed_and_says_it_is_unmeasured() {
    // A status page that quietly shrank to its working members would read as good news, and
    // under censorship a lookup that fails is itself the finding.
    let mut monitor = ServiceMonitor::new(StatusThresholds::default());
    monitor
        .add(&service("svc", vec![endpoint("svc/e0", None)]), &[None])
        .unwrap();

    let view = only(&monitor, Instant::now());
    assert_eq!(view.endpoints.len(), 1);
    assert!(!view.endpoints[0].measurable);
    assert_eq!(view.endpoints[0].resolved_address, None);
    assert_eq!(view.endpoints[0].written_address, "svc/e0.example.net");
    assert_eq!(view.verdict, HealthView::Unknown);
}

#[test]
fn a_clean_run_of_checks_reads_as_reachable() {
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    feed(&mut monitor, ids[0], now, &[ok(40), ok(42), ok(38)]);

    let view = only(&monitor, now);
    assert_eq!(view.verdict, HealthView::Ok);
    assert_eq!(view.counts.ok, 1);
    // The freshest answer, not the window's mean: a card says how the service responds now.
    assert_eq!(view.endpoints[0].rtt_ms, Some(38.0));
    assert!(view.endpoints[0].mean_rtt_ms.is_some());
    assert_eq!(view.last_checked_secs, Some(0.0));
}

#[test]
fn one_failed_check_moves_the_card_off_reachable_without_claiming_an_outage() {
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    feed(
        &mut monitor,
        ids[0],
        now,
        &[ok(40), ok(40), ProbeOutcome::Timeout],
    );

    let view = only(&monitor, now);
    assert_eq!(
        view.verdict,
        HealthView::Degraded,
        "a single lost check must not be reported as a service being down"
    );
    assert_eq!(
        view.endpoints[0].checks.last().unwrap().mark,
        CheckMarkView::Lost,
        "the strip must show the failed check even while the headline does not claim one"
    );
}

#[test]
fn the_second_consecutive_failure_makes_the_card_unreachable() {
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    feed(
        &mut monitor,
        ids[0],
        now,
        &[ok(40), ProbeOutcome::Timeout, ProbeOutcome::Timeout],
    );

    assert_eq!(only(&monitor, now).verdict, HealthView::Unreachable);
}

#[test]
fn recovery_shows_on_the_next_check_rather_than_waiting_out_a_window() {
    // The whole reason the status page has a rule of its own. A window rule would keep the
    // card red for minutes after the service came back.
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    feed(
        &mut monitor,
        ids[0],
        now,
        &[
            ProbeOutcome::Timeout,
            ProbeOutcome::Timeout,
            ProbeOutcome::Timeout,
            ok(45),
        ],
    );

    let view = only(&monitor, now);
    assert_ne!(view.verdict, HealthView::Unreachable);
    assert_eq!(view.verdict, HealthView::Degraded);
}

#[test]
fn a_service_with_two_endpoints_shows_the_distribution_rather_than_one_colour() {
    // A storefront that answers while the gateway does not is the finding, and a single
    // amber dot would hide which half is broken.
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(2);
    feed(&mut monitor, ids[0], now, &[ok(40), ok(40)]);
    feed(
        &mut monitor,
        ids[1],
        now,
        &[ProbeOutcome::Timeout, ProbeOutcome::Timeout],
    );

    let view = only(&monitor, now);
    assert_eq!(view.verdict, HealthView::Degraded);
    assert_eq!(view.counts.ok, 1);
    assert_eq!(view.counts.unreachable, 1);
    assert_eq!(view.endpoints[0].health, HealthView::Ok);
    assert_eq!(view.endpoints[1].health, HealthView::Unreachable);
}

#[test]
fn last_checked_reports_the_freshest_endpoint_not_the_stalest() {
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(2);
    monitor.record(
        ids[0],
        ProbeSample::new(before(now, CHECK_INTERVAL * 4), ok(40)),
    );
    monitor.record(
        ids[1],
        ProbeSample::new(before(now, Duration::from_secs(3)), ok(40)),
    );

    let view = only(&monitor, now);
    let age = view.last_checked_secs.unwrap();
    assert!((age - 3.0).abs() < 0.5, "{age}");
}

#[test]
fn a_run_of_filtered_checks_is_blocked_rather_than_unreachable() {
    // Filtering measured nothing about the service; calling it down would claim knowledge
    // the checks never produced.
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    feed(
        &mut monitor,
        ids[0],
        now,
        &[ProbeOutcome::Blocked, ProbeOutcome::Blocked],
    );

    let view = only(&monitor, now);
    assert_eq!(view.verdict, HealthView::Blocked);
    assert_eq!(
        view.endpoints[0].checks.last().unwrap().mark,
        CheckMarkView::Filtered
    );
}

#[test]
fn the_timeline_is_capped_and_stamped_with_real_elapsed_time() {
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    let outcomes: Vec<ProbeOutcome> = (0..TIMELINE_POINTS + 10).map(|_| ok(40)).collect();
    feed(&mut monitor, ids[0], now, &outcomes);

    let checks = only(&monitor, now).endpoints[0].checks.clone();
    assert_eq!(checks.len(), TIMELINE_POINTS);
    // Negative and ascending to zero at the right-hand edge, so a stretched interval shows
    // as a gap rather than as an evenly spaced lie.
    assert!(checks[0].age_secs < checks[checks.len() - 1].age_secs);
    assert!(checks[checks.len() - 1].age_secs <= 0.0);
    assert!(checks.iter().all(|check| check.age_secs <= 0.0));
}

#[test]
fn a_slow_answer_is_marked_apart_from_a_fast_one() {
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    feed(&mut monitor, ids[0], now, &[ok(40), ok(900)]);

    let view = only(&monitor, now);
    let marks: Vec<CheckMarkView> = view.endpoints[0]
        .checks
        .iter()
        .map(|check| check.mark)
        .collect();
    assert_eq!(marks, vec![CheckMarkView::Answered, CheckMarkView::Slow]);
    assert_eq!(view.verdict, HealthView::Degraded);
}

#[test]
fn a_tunnelled_endpoint_is_labelled_rather_than_presented_as_a_round_trip() {
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    monitor.note_tunnelled(ids[0]);
    feed(&mut monitor, ids[0], now, &[ok(180), ok(175)]);

    assert!(only(&monitor, now).endpoints[0].tunnelled);
}

#[test]
fn a_tunnel_found_after_registration_reaches_the_card() {
    // An ordinary public address reached through a TUN client looks innocent when it is
    // registered: nothing about the address says a tunnel will take it. The engine learns
    // it from the route or from a reply that crossed no router, and the finding arrives
    // with the next report rather than waiting for a restart.
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    assert!(!only(&monitor, now).endpoints[0].tunnelled);

    monitor.note_probe_state(
        ids[0],
        Some(nm_probes::probe::ProbeKind::TlsHello),
        true,
        false,
        true,
    );
    feed(&mut monitor, ids[0], now, &[ok(180), ok(175)]);

    assert!(only(&monitor, now).endpoints[0].tunnelled);
}

#[test]
fn a_tunnel_switched_off_stops_being_claimed() {
    // The other half, and the reason the flag travels with every report instead of being
    // settled once: a user turning their VPN off must not keep a badge saying their
    // figures are measured through something that is no longer there.
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    monitor.note_probe_state(
        ids[0],
        Some(nm_probes::probe::ProbeKind::TlsHello),
        true,
        false,
        true,
    );
    assert!(only(&monitor, now).endpoints[0].tunnelled);

    monitor.note_probe_state(
        ids[0],
        Some(nm_probes::probe::ProbeKind::IcmpEcho),
        false,
        false,
        true,
    );

    assert!(!only(&monitor, now).endpoints[0].tunnelled);
}

#[test]
fn an_endpoint_that_ran_out_of_probe_kinds_says_so() {
    let now = Instant::now();
    let (mut monitor, ids, _registry) = one_service(1);
    monitor.note_unmeasurable(ids[0]);

    let view = only(&monitor, now);
    assert!(!view.endpoints[0].measurable);
    assert_eq!(view.endpoints[0].probe_kind, None);
}

#[test]
fn a_handle_belonging_to_nothing_here_is_ignored() {
    // Baselines, application endpoints and services share one probe engine, so every
    // monitor is told about every result and must drop the ones that are not its business.
    let now = Instant::now();
    let (mut monitor, _, mut registry) = one_service(1);
    let stranger = registry
        .insert(TargetAddress::icmp(ip(200)), TargetTag::AppEndpoint)
        .unwrap();

    monitor.record(stranger, ProbeSample::new(now, ok(40)));
    monitor.note_unmeasurable(stranger);

    let view = only(&monitor, now);
    assert!(view.endpoints[0].checks.is_empty());
    assert!(view.endpoints[0].measurable);
}

#[test]
fn the_snapshot_states_the_cadence_its_figures_were_taken_at() {
    let (monitor, _, _registry) = one_service(1);
    let snapshot = monitor.snapshot(Instant::now());

    assert_eq!(
        snapshot.check_interval_secs,
        u32::try_from(CHECK_INTERVAL.as_secs()).unwrap()
    );
    assert!(snapshot.window_secs >= snapshot.check_interval_secs);
}
