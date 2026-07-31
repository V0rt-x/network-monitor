//! Tests for the state behind the general-health dashboard.
//!
//! The monitor reads no clock and opens no socket, so a whole session of degradation is
//! replayed here in microseconds with no network involved.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use nm_app::baselines::{BaselineGroup, BaselineTarget};
use nm_app::monitor::{health_window, BaselineMonitor, HISTORY_CAPACITY};
use nm_app::{GroupView, HealthView, ProbeKindView, TargetView, SERIES_POINTS};
use nm_core::health::HealthThresholds;
use nm_core::sample::{ProbeOutcome, ProbeSample, Rtt};
use nm_core::target::{TargetAddress, TargetId, TargetRegistry, TargetTag};

const WINDOW: Duration = Duration::from_secs(60);

/// Distinct handles, minted from a real registry because that is the only way to get one.
fn ids(count: u8) -> Vec<TargetId> {
    let mut registry = TargetRegistry::new();
    (0..count)
        .map(|index| {
            let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, index + 1));
            registry
                .insert(
                    TargetAddress::with_port(ip, 443),
                    TargetTag::ForeignBaseline,
                )
                .unwrap()
        })
        .collect()
}

fn target(group: BaselineGroup, key: &str, address: Option<&str>) -> BaselineTarget {
    BaselineTarget {
        group,
        key: key.to_owned(),
        label: key.to_owned(),
        written_address: address.unwrap_or("unresolvable.example").to_owned(),
        address: address.map(|raw| TargetAddress::with_port(raw.parse().unwrap(), 443)),
    }
}

fn monitor() -> BaselineMonitor {
    BaselineMonitor::new(HealthThresholds::default(), WINDOW)
}

fn ok(millis: u32) -> ProbeOutcome {
    ProbeOutcome::Success(Rtt::from_micros(millis * 1_000))
}

/// Records `outcomes` for `id`, one second apart, ending at `now`.
fn feed(monitor: &mut BaselineMonitor, id: TargetId, now: Instant, outcomes: &[ProbeOutcome]) {
    let count = u64::try_from(outcomes.len()).unwrap();
    for (index, outcome) in outcomes.iter().enumerate() {
        let age = Duration::from_secs(count - u64::try_from(index).unwrap());
        monitor.record(
            id,
            ProbeSample::new(
                now.checked_sub(age).expect("the test clock has room"),
                *outcome,
            ),
        );
    }
}

fn group(snapshot: &nm_app::NetworkHealth, which: BaselineGroup) -> &GroupView {
    snapshot
        .groups
        .iter()
        .find(|view| view.group == which)
        .expect("both groups are always present")
}

/// The only member of a group, for the many single-target cases below.
fn only_target(monitor: &BaselineMonitor, now: Instant, which: BaselineGroup) -> TargetView {
    let snapshot = monitor.snapshot(now, 1);
    group(&snapshot, which)
        .targets
        .first()
        .cloned()
        .expect("the group has one member")
}

#[test]
fn an_empty_monitor_still_reports_both_groups() {
    // The dashboard must render its two columns from the first frame, saying "unknown"
    // rather than showing nothing at all.
    let snapshot = monitor().snapshot(Instant::now(), 0);
    assert_eq!(snapshot.groups.len(), 2);
    assert_eq!(snapshot.groups[0].group, BaselineGroup::Domestic);
    assert_eq!(snapshot.groups[1].group, BaselineGroup::Foreign);
    for view in &snapshot.groups {
        assert_eq!(view.verdict, HealthView::Unknown);
        assert!(view.targets.is_empty());
        assert_eq!(view.rtt_ms, None);
        assert_eq!(
            view.loss_pct, None,
            "an unmeasured group must not read as 0 % loss"
        );
    }
}

#[test]
fn the_window_and_uptime_travel_with_the_snapshot() {
    let snapshot = monitor().snapshot(Instant::now(), 42);
    assert_eq!(snapshot.uptime_secs, 42);
    assert_eq!(snapshot.window_secs, 60);
}

#[test]
fn a_healthy_group_reads_as_ok() {
    let now = Instant::now();
    let ids = ids(2);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "a", Some("1.1.1.1")),
            Some(ids[0]),
            false,
        )
        .unwrap();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "b", Some("8.8.8.8")),
            Some(ids[1]),
            false,
        )
        .unwrap();

    feed(&mut monitor, ids[0], now, &[ok(10), ok(11), ok(10)]);
    feed(&mut monitor, ids[1], now, &[ok(20), ok(21), ok(20)]);

    let snapshot = monitor.snapshot(now, 1);
    let foreign = group(&snapshot, BaselineGroup::Foreign);
    assert_eq!(foreign.verdict, HealthView::Ok);
    assert_eq!(foreign.counts.ok, 2);
    assert_eq!(foreign.targets.len(), 2);
}

#[test]
fn one_dead_member_degrades_the_group_and_stays_visible() {
    // The requirement CLAUDE.md spells out: never collapse a group to one colour. The
    // failing member must be findable, and the healthy ones must not be reported as broken.
    let now = Instant::now();
    let ids = ids(3);
    let mut monitor = monitor();
    for (index, id) in ids.iter().enumerate() {
        monitor
            .add(
                &target(
                    BaselineGroup::Domestic,
                    &format!("t{index}"),
                    Some("1.1.1.1"),
                ),
                Some(*id),
                false,
            )
            .unwrap();
    }

    feed(&mut monitor, ids[0], now, &[ok(10), ok(10), ok(10)]);
    feed(&mut monitor, ids[1], now, &[ok(12), ok(12), ok(12)]);
    feed(&mut monitor, ids[2], now, &[ProbeOutcome::Timeout; 4]);

    let snapshot = monitor.snapshot(now, 1);
    let domestic = group(&snapshot, BaselineGroup::Domestic);
    assert_eq!(domestic.verdict, HealthView::Degraded);
    assert_eq!(domestic.counts.ok, 2);
    assert_eq!(domestic.counts.unreachable, 1);

    let healths: Vec<_> = domestic.targets.iter().map(|t| t.health).collect();
    assert_eq!(
        healths,
        vec![HealthView::Ok, HealthView::Ok, HealthView::Unreachable]
    );
}

#[test]
fn groups_do_not_leak_into_each_other() {
    let now = Instant::now();
    let ids = ids(2);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Domestic, "home", Some("1.1.1.1")),
            Some(ids[0]),
            false,
        )
        .unwrap();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "away", Some("8.8.8.8")),
            Some(ids[1]),
            false,
        )
        .unwrap();

    feed(&mut monitor, ids[0], now, &[ok(10), ok(10), ok(10)]);
    feed(&mut monitor, ids[1], now, &[ProbeOutcome::Timeout; 4]);

    let snapshot = monitor.snapshot(now, 1);
    // The headline the whole product turns on: domestic fine, foreign dead.
    assert_eq!(
        group(&snapshot, BaselineGroup::Domestic).verdict,
        HealthView::Ok
    );
    assert_eq!(
        group(&snapshot, BaselineGroup::Foreign).verdict,
        HealthView::Unreachable
    );
}

#[test]
fn filtered_probes_never_become_packet_loss() {
    let now = Instant::now();
    let ids = ids(1);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "a", Some("1.1.1.1")),
            Some(ids[0]),
            false,
        )
        .unwrap();
    feed(&mut monitor, ids[0], now, &[ProbeOutcome::Blocked; 4]);

    let view = only_target(&monitor, now, BaselineGroup::Foreign);
    assert_eq!(view.health, HealthView::Blocked);
    assert_eq!(view.loss_pct, None, "a filtered probe measures nothing");
}

#[test]
fn an_unresolved_target_stays_listed_and_unmeasurable() {
    // A foreign baseline that quietly shrank to its working members would read as good
    // news, which is exactly the lie this product exists to avoid.
    let now = Instant::now();
    let mut monitor = monitor();
    monitor
        .add(&target(BaselineGroup::Foreign, "gone", None), None, false)
        .unwrap();

    let snapshot = monitor.snapshot(now, 1);
    let foreign = group(&snapshot, BaselineGroup::Foreign);
    assert_eq!(foreign.targets.len(), 1);
    let view = &foreign.targets[0];
    assert_eq!(view.resolved_address, None);
    assert!(!view.measurable);
    assert_eq!(view.health, HealthView::Unknown);
    assert_eq!(foreign.verdict, HealthView::Unknown);
}

#[test]
fn the_probe_kind_and_proven_filtering_reach_the_view() {
    let now = Instant::now();
    let ids = ids(1);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "a", Some("1.1.1.1")),
            Some(ids[0]),
            false,
        )
        .unwrap();

    monitor.note_probe_state(
        ids[0],
        Some(nm_probes::probe::ProbeKind::TlsHello),
        true,
        true,
    );
    feed(&mut monitor, ids[0], now, &[ok(90), ok(92), ok(91)]);

    let view = only_target(&monitor, now, BaselineGroup::Foreign);
    assert_eq!(view.probe_kind, Some(ProbeKindView::TlsHello));
    assert!(view.filtering_confirmed);
    assert!(view.measurable);
}

#[test]
fn a_target_that_runs_out_of_probe_kinds_says_so() {
    let now = Instant::now();
    let ids = ids(1);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "a", Some("1.1.1.1")),
            Some(ids[0]),
            true,
        )
        .unwrap();
    monitor.note_unmeasurable(ids[0]);

    let view = only_target(&monitor, now, BaselineGroup::Foreign);
    assert!(!view.measurable);
    assert_eq!(view.probe_kind, None);
    assert!(
        view.tunnelled,
        "the tunnel is why its figure means something else"
    );
}

#[test]
fn the_series_carries_real_time_and_keeps_its_gaps() {
    let now = Instant::now();
    let ids = ids(1);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "a", Some("1.1.1.1")),
            Some(ids[0]),
            false,
        )
        .unwrap();
    feed(
        &mut monitor,
        ids[0],
        now,
        &[ok(10), ProbeOutcome::Timeout, ok(12)],
    );

    let view = only_target(&monitor, now, BaselineGroup::Foreign);
    assert_eq!(view.series_rtt_ms, vec![Some(10.0), None, Some(12.0)]);
    // Ages are negative seconds before now, ascending towards the right-hand edge.
    assert_eq!(view.series_age_secs.len(), 3);
    assert!(view.series_age_secs[0] < view.series_age_secs[2]);
    assert!(view.series_age_secs.iter().all(|age| *age <= 0.0));
}

#[test]
fn the_series_is_capped_however_long_the_app_runs() {
    // Bounded IPC payloads are part of the resource budget: this array is sent every
    // second, for every target, for as long as the window is open.
    let now = Instant::now();
    let ids = ids(1);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "a", Some("1.1.1.1")),
            Some(ids[0]),
            false,
        )
        .unwrap();

    for index in 0..(HISTORY_CAPACITY * 2) {
        let age = Duration::from_millis(u64::try_from(index).unwrap());
        monitor.record(
            ids[0],
            ProbeSample::new(
                now.checked_sub(age).expect("the test clock has room"),
                ok(10),
            ),
        );
    }

    let view = only_target(&monitor, now, BaselineGroup::Foreign);
    assert_eq!(view.series_rtt_ms.len(), SERIES_POINTS);
    assert_eq!(view.series_age_secs.len(), SERIES_POINTS);
}

#[test]
fn samples_older_than_the_window_stop_counting() {
    let now = Instant::now();
    let ids = ids(1);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "a", Some("1.1.1.1")),
            Some(ids[0]),
            false,
        )
        .unwrap();

    // An outage that ended five minutes ago must not still be the verdict.
    for index in 1..=10_u64 {
        // Strictly older than the window, which is inclusive at its cutoff.
        let age = WINDOW + Duration::from_secs(index);
        monitor.record(
            ids[0],
            ProbeSample::new(
                now.checked_sub(age).expect("the test clock has room"),
                ProbeOutcome::Timeout,
            ),
        );
    }
    feed(&mut monitor, ids[0], now, &[ok(10), ok(10), ok(10)]);

    let view = only_target(&monitor, now, BaselineGroup::Foreign);
    assert_eq!(view.health, HealthView::Ok);
    assert_eq!(view.loss_pct, Some(0.0));
}

#[test]
fn a_sample_for_an_unknown_handle_is_ignored_rather_than_misfiled() {
    let now = Instant::now();
    let ids = ids(2);
    let mut monitor = monitor();
    monitor
        .add(
            &target(BaselineGroup::Foreign, "a", Some("1.1.1.1")),
            Some(ids[0]),
            false,
        )
        .unwrap();

    monitor.record(ids[1], ProbeSample::new(now, ok(10)));
    monitor.note_probe_state(
        ids[1],
        Some(nm_probes::probe::ProbeKind::IcmpEcho),
        true,
        true,
    );

    let view = only_target(&monitor, now, BaselineGroup::Foreign);
    assert_eq!(view.health, HealthView::Unknown);
    assert!(!view.filtering_confirmed);
}

#[test]
fn the_health_window_scales_with_the_probe_interval() {
    // A fixed window would hold one sample at a sixty-second interval, leaving every
    // verdict permanently unknown.
    assert_eq!(
        health_window(Duration::from_secs(1)),
        Duration::from_secs(60)
    );
    assert_eq!(
        health_window(Duration::from_secs(5)),
        Duration::from_secs(60)
    );
    assert_eq!(
        health_window(Duration::from_secs(20)),
        Duration::from_secs(240)
    );
    assert_eq!(
        health_window(Duration::from_secs(60)),
        Duration::from_secs(600)
    );
    assert_eq!(health_window(Duration::MAX), Duration::from_secs(600));
}

#[test]
fn counting_and_emptiness_track_what_was_added() {
    let mut monitor = monitor();
    assert!(monitor.is_empty());
    monitor
        .add(&target(BaselineGroup::Foreign, "a", None), None, false)
        .unwrap();
    assert_eq!(monitor.len(), 1);
    assert!(!monitor.is_empty());
}
