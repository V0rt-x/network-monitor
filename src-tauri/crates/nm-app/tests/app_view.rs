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

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use nm_app::apps::AppMonitor;
use nm_app::asn::NetworkNames;
use nm_app::discovery::PassiveRtt;
use nm_app::{
    AppView, EndpointAgeKindView, EndpointView, HealthCountsView, HealthView, ProbeKindView,
    TransportView,
};
use nm_core::address::AddressPolicy;
use nm_core::asn::AsnTable;
use nm_core::diagnosis::BaselineEvidence;
use nm_core::endpoint::{AppId, EndpointKey, LifecyclePolicy};
use nm_core::flow::{FlowInstant, FlowObservation};
use nm_core::health::{Health, HealthCounts, HealthThresholds};
use nm_core::sample::{ProbeOutcome, ProbeSample, Rtt};
use nm_core::target::{TargetId, TargetRegistry};
use nm_platform::interface::{InterfaceNames, NetworkInterface};
use nm_probes::probe::ProbeKind;

const APP: AppId = AppId::new(1);
const PID: u32 = 4242;
const APP_ID: u32 = 1;

/// Enough samples to clear the minimum a verdict needs: one lost packet is not an outage,
/// and the thresholds refuse to judge below that.
const ENOUGH: u32 = 8;

/// One sighting that carried `bytes` of traffic.
///
/// The moment is fixed because these tests are about what the page does with a byte count;
/// the arrival-timing figures, which need distinct stamps, build their own streams.
fn traffic(bytes: u32) -> FlowObservation {
    FlowObservation::received(FlowInstant::from_origin(Duration::from_secs(1)), bytes)
}

fn monitor() -> (AppMonitor, TargetRegistry, Instant) {
    let mut monitor = AppMonitor::new(
        AddressPolicy::default(),
        LifecyclePolicy::default(),
        HealthThresholds::default(),
        Duration::from_secs(60),
    )
    .unwrap();
    // The moment monitoring began, which is where the chart's axis is anchored.
    let now = Instant::now();
    monitor.monitor(APP, now).unwrap();
    (monitor, TargetRegistry::new(), now)
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

/// The targets a sweep registered, keyed by the address they were registered for.
///
/// A test that mixes transports cannot use registration order: the tracker holds endpoints
/// in key order, and which of a TCP and a UDP endpoint comes first is not something a test
/// about the *page* should depend on.
fn registered_by_ip(
    monitor: &mut AppMonitor,
    registry: &mut TargetRegistry,
    now: Instant,
) -> BTreeMap<IpAddr, TargetId> {
    monitor
        .sweep(registry, now)
        .iter()
        .filter_map(|change| match change {
            nm_app::apps::TargetChange::Register { id, address, .. } => Some((address.ip, *id)),
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

/// The health window these tests give the monitor, which is also the warm-up.
const WINDOW: Duration = Duration::from_secs(60);

/// Fills an endpoint's history across a whole window, one sample a second.
///
/// What it buys is a *past* the warm-up rule is satisfied by: the derived figures are
/// withheld until the window behind them is real, so a test about what loss means has to
/// reach that point rather than assert a dash that warm-up would have produced anyway.
fn soak(monitor: &mut AppMonitor, id: TargetId, now: Instant, outcome: ProbeOutcome) {
    for step in 0..=WINDOW.as_secs() {
        monitor.record(
            id,
            ProbeSample::new(now + Duration::from_secs(step), outcome),
        );
    }
}

/// Names the one adapter these tests pretend the machine has.
///
/// Invented, like every address here: `192.0.2.0/24` is the documentation range and
/// nothing on a real machine appears in this file.
fn adapters() -> InterfaceNames {
    InterfaceNames::of(&[NetworkInterface {
        name: "Test Adapter".to_owned(),
        addresses: vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))],
    }])
}

/// A baseline group whose four members all answer, so nothing the tests assert about an
/// application is ever shadowed by a verdict about the network underneath it.
fn clean_baseline() -> HealthCounts {
    let mut counts = HealthCounts::default();
    for _ in 0..4 {
        counts.record(Health::Ok);
    }
    counts
}

/// Every endpoint of an application, in the order the page shows them.
///
/// The page groups by transport — the match traffic first, the supporting connections below
/// — and orders by severity inside each group. Most of these tests are about the ordering
/// and the figures rather than the grouping, so they flatten it; the tests that are about
/// the grouping read `groups` directly.
fn flat(view: &AppView) -> Vec<EndpointView> {
    view.groups
        .iter()
        .flat_map(|group| group.endpoints.clone())
        .collect()
}

/// The page as it looks once the warm-up is over.
///
/// Most of these tests are about what a *measurement* renders as, and during warm-up the
/// derived figures are deliberately withheld — so they ask for the view at a moment past it.
/// The tests that are about the warm-up itself use [`warming`].
fn view(monitor: &mut AppMonitor, now: Instant) -> AppView {
    view_at(monitor, now, None)
}

/// The page as it looks while the application is still warming up.
fn warming(monitor: &mut AppMonitor, now: Instant) -> AppView {
    let warmup = monitor.warmup_remaining(APP, now);
    view_at(monitor, now, warmup)
}

/// A directory naming one of the two invented endpoints these tests probe, and not the other.
///
/// Hand-built rather than the bundled snapshot, for two reasons: loading 12 MB to answer one
/// question would be the slowest thing in this file, and a test that asserted on the real
/// internet's registrations would start failing the day a block changed hands. Covering
/// `1.1.1.0`–`1.1.1.1` and stopping there is what lets one fixture assert both that a known
/// address is named and that an unknown one is left alone.
fn names() -> NetworkNames {
    NetworkNames::of(
        AsnTable::parse(
            "1.1.1.0\t1.1.1.1\t64500",
            "64500\tUS\tEXAMPLE-NET Example Transit",
        )
        .expect("the fixture directory must parse"),
    )
}

fn view_at(monitor: &mut AppMonitor, now: Instant, warmup: Option<Duration>) -> AppView {
    view_named(monitor, now, warmup, &names())
}

fn view_named(
    monitor: &mut AppMonitor,
    now: Instant,
    warmup: Option<Duration>,
    names: &NetworkNames,
) -> AppView {
    let axis = monitor.chart_elapsed_secs(APP, now);
    let reports = monitor.endpoints(APP, now);
    AppView::of(
        APP_ID,
        "game.exe".to_owned(),
        vec![PID],
        axis,
        0.0,
        warmup,
        &adapters(),
        names,
        &reports,
        None,
        (
            BaselineEvidence::of(clean_baseline()),
            BaselineEvidence::of(clean_baseline()),
        ),
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

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    let order: Vec<HealthView> = flat(&view).iter().map(|endpoint| endpoint.health).collect();
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

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    assert_eq!(view.counts.ok, 1);
    assert_eq!(view.counts.unreachable, 1);
    assert_eq!(view.counts.degraded, 0);
    assert_eq!(view.id, APP_ID);
    assert_eq!(view.name, "game.exe");
    assert_eq!(
        view.pids,
        vec![PID],
        "the identifiers travel so the picker can tell a taken offer; the page shows a count"
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

    let keys: Vec<String> = flat(&view(&mut monitor, now))
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

    let view = view(&mut monitor, now);
    let endpoints = flat(&view);
    assert_eq!(endpoints.len(), 2);
    let keys: Vec<&str> = endpoints
        .iter()
        .map(|endpoint| endpoint.key.as_str())
        .collect();
    assert_ne!(keys[0], keys[1]);
    assert!(endpoints
        .iter()
        .any(|endpoint| endpoint.transport == TransportView::Tcp));
    assert!(endpoints
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
        .observe(APP, sentinel, Some(egress), Some(traffic(4_096)), now)
        .unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    // Tunnelled, because this endpoint is a fake-IP sentinel and the engine classifies it
    // as one. The flag arrives with every report rather than being fixed at discovery, so
    // a run that said otherwise here would be a run that had stopped believing its own
    // classification.
    monitor.note_probe_state(ids[0], Some(ProbeKind::TlsHello), true, true, true);

    let view = view(&mut monitor, now);
    let endpoints = flat(&view);
    let endpoint = &endpoints[0];

    assert!(
        endpoint.tunnelled,
        "a figure through a tunnel is not a round trip to the server"
    );
    assert_eq!(endpoint.probe_kind, Some(ProbeKindView::TlsHello));
    assert!(endpoint.filtering_confirmed);
    assert!(endpoint.measurable);
    assert_eq!(endpoint.egress, Some(egress.to_string()));
    assert_eq!(
        endpoint.egress_interface.as_deref(),
        Some("Test Adapter"),
        "an address is not something a user can check a before-and-after against; a name is"
    );
    assert_eq!(
        endpoint.probe_egress, None,
        "the probe follows the application, so there is no second route to disclose"
    );
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
        .observe(APP, udp(1), None, Some(traffic(630_000)), now)
        .unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    // Across a whole window, so the dash below is the rule about what probes describe and
    // not the warm-up withholding a figure it would have shown a minute later.
    soak(&mut monitor, ids[0], now, ProbeOutcome::Timeout);

    let view = view(&mut monitor, now + WINDOW);
    let endpoints = flat(&view);
    let endpoint = &endpoints[0];

    assert_eq!(
        endpoint.warmup_secs_remaining, None,
        "the window has filled"
    );
    assert_eq!(endpoint.health, HealthView::CarryingTraffic);
    assert_eq!(view.counts.carrying_traffic, 1);
    assert_eq!(view.counts.unreachable, 0);
    // And it claims nothing it did not measure: liveness is not latency.
    assert_eq!(endpoint.rtt_ms, None);
    assert_eq!(
        endpoint.loss_pct, None,
        "every probe timed out, but those probes were ours and the port is not one the \
         game plays over — quoting 100 % loss here would report a working server as dead"
    );
    assert_eq!(endpoint.recent_bytes, Some(630_000.0));
}

#[test]
fn a_silent_endpoint_with_no_traffic_is_still_unreachable() {
    // The complement: without passive evidence there is nothing to soften the verdict with,
    // and inventing life for an endpoint nothing has been seen crossing would be the same
    // failure in the opposite direction.
    let (mut monitor, mut registry, now) = monitor();
    monitor
        .observe(APP, udp(1), None, Some(traffic(0)), now)
        .unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    assert_eq!(flat(&view)[0].health, HealthView::Unreachable);
}

#[test]
fn a_measured_endpoint_keeps_the_verdict_its_probes_earned() {
    // Traffic must not paper over a degraded path — that is the finding the user came for.
    let (mut monitor, mut registry, now) = monitor();
    monitor
        .observe(APP, udp(1), None, Some(traffic(630_000)), now)
        .unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(
        &mut monitor,
        ids[0],
        now,
        ProbeOutcome::Success(Rtt::from_micros(400_000)),
    );

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    assert_eq!(flat(&view)[0].health, HealthView::Degraded);
}

#[test]
fn a_live_but_unmeasured_endpoint_sorts_below_the_broken_ones() {
    // It needs no action from the user, so it must not sit above an endpoint that does.
    let (mut monitor, mut registry, now) = monitor();
    monitor
        .observe(APP, udp(1), None, Some(traffic(630_000)), now)
        .unwrap();
    monitor
        .observe(APP, udp(2), None, Some(traffic(0)), now)
        .unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);
    fill(&mut monitor, ids[1], now, ProbeOutcome::Timeout);

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    let order: Vec<HealthView> = flat(&view).iter().map(|endpoint| endpoint.health).collect();
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

    assert_eq!(flat(&view(&mut monitor, now))[0].recent_bytes, None);
}

#[test]
fn an_unmeasured_endpoint_reports_no_figures_at_all() {
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    registered(&mut monitor, &mut registry, now);

    let view = view(&mut monitor, now);
    let endpoints = flat(&view);
    let endpoint = &endpoints[0];
    assert_eq!(endpoint.rtt_ms, None);
    assert_eq!(endpoint.jitter_ms, None);
    assert_eq!(endpoint.loss_pct, None);
    assert_eq!(endpoint.health, HealthView::Unknown);
    assert!(
        endpoint.chart_rtt_ms.iter().all(Option::is_none),
        "an unmeasured endpoint occupies the chart's axis and draws nothing on it"
    );
}

#[test]
fn an_application_with_nothing_discovered_renders_empty_rather_than_missing() {
    let (mut monitor, _registry, now) = monitor();
    let view = view(&mut monitor, now);
    assert!(flat(&view).is_empty());
    assert_eq!(view.counts, HealthCountsView::default());
    assert_eq!(
        view.groups.len(),
        2,
        "both groups are always sent: an absent match-traffic group would read as a game \
         that plays over nothing, rather than as one that has not connected yet"
    );
}

// ------------------------------------------------------- one chart, one axis

#[test]
fn every_endpoint_is_drawn_against_the_same_axis() {
    // The whole reason the chart exists: a list of sparklines answers "how is this
    // endpoint", and the question the user has is "which of these is the odd one out".
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

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    let slots = view.chart_elapsed_secs.len();
    assert!(slots > 1);
    assert_eq!(
        view.chart_elapsed_secs.first().copied(),
        Some(0.0),
        "the axis begins where monitoring did, so a fresh application draws from the left \
         edge rather than being pinned to the right with empty space behind it"
    );
    assert!(
        view.chart_elapsed_secs
            .windows(2)
            .all(|pair| pair[1] > pair[0]),
        "and it ascends"
    );
    for endpoint in &flat(&view) {
        assert_eq!(endpoint.chart_rtt_ms.len(), slots);
        assert_eq!(endpoint.chart_path_ms.len(), slots);
    }
}

#[test]
fn a_stretch_where_nothing_answered_is_a_break_in_the_line() {
    // The only thing a break may mean. A slot covers several probes, so one lost packet
    // beside a successful one is loss — reported as a percentage in the row — and drawing
    // it as a break would say the endpoint went away.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);

    // One slot in which something answered, then a run of slots in which nothing did.
    monitor.record(
        ids[0],
        ProbeSample::new(now, ProbeOutcome::Success(Rtt::from_micros(9_000))),
    );
    monitor.record(
        ids[0],
        ProbeSample::new(now + Duration::from_secs(1), ProbeOutcome::Timeout),
    );
    for step in 2..12 {
        monitor.record(
            ids[0],
            ProbeSample::new(now + Duration::from_secs(step), ProbeOutcome::Timeout),
        );
    }

    let view = view(&mut monitor, now + Duration::from_secs(11));
    let endpoints = flat(&view);
    let drawn = &endpoints[0].chart_rtt_ms;

    assert_eq!(
        drawn.last().copied(),
        Some(None),
        "nothing answered anywhere near now"
    );
    assert!(
        drawn.contains(&Some(9.0)),
        "and the slot that did answer keeps its figure, timeout beside it or not"
    );
}

#[test]
fn a_chart_slot_shows_the_slowest_round_trip_in_it() {
    // The chart exists to find the spike; a slot that showed the typical figure would hide
    // exactly what the user came for. The row beside it still reports the mean.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    for (step, micros) in [(0, 9_000), (1, 90_000), (2, 9_000)] {
        monitor.record(
            ids[0],
            ProbeSample::new(
                now + Duration::from_secs(step),
                ProbeOutcome::Success(Rtt::from_micros(micros)),
            ),
        );
    }

    let view = view(&mut monitor, now + Duration::from_secs(2));

    assert!(
        flat(&view)[0].chart_rtt_ms.contains(&Some(90.0)),
        "the spike survives being put on a coarser grid"
    );
}

/// A match server's traffic: twenty updates a second each way, on the operating system's
/// own event clock, starting `from_ms` after that clock's origin.
fn play(monitor: &mut AppMonitor, key: EndpointKey, now: Instant, from_ms: u64, updates: u64) {
    for index in 0..updates {
        let moment = from_ms + index * 50;
        monitor
            .observe(
                APP,
                key,
                None,
                Some(FlowObservation::sent(
                    FlowInstant::from_origin(Duration::from_millis(moment)),
                    64,
                )),
                now,
            )
            .unwrap();
        monitor
            .observe(
                APP,
                key,
                None,
                Some(FlowObservation::received(
                    FlowInstant::from_origin(Duration::from_millis(moment + 10)),
                    1_024,
                )),
                now,
            )
            .unwrap();
    }
}

#[test]
fn a_silent_match_server_gets_a_flow_column_while_its_own_figures_stay_dashes() {
    // The rule the whole phase exists to protect. Nothing we can send reaches a game's
    // match server, so its round trip, jitter and loss have to stay empty — and the traffic
    // crossing it is measured all the same, in a column of its own. Filling those three
    // dashes with the nearest available number is precisely how this product would start
    // lying.
    let (mut monitor, mut registry, now) = monitor();
    play(&mut monitor, udp(1), now, 0, 200);
    let ids = registered(&mut monitor, &mut registry, now);
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    let endpoints = flat(&view);
    let endpoint = &endpoints[0];

    assert_eq!(endpoint.health, HealthView::CarryingTraffic);
    assert_eq!(endpoint.rtt_ms, None, "nothing measured a round trip");
    assert_eq!(endpoint.jitter_ms, None);
    assert_eq!(endpoint.loss_pct, None);

    let flow = endpoint.flow.expect("its own traffic is the other column");
    let updates = flow.updates_per_sec.expect("two hundred updates is plenty");
    assert!(
        (updates - 20.0).abs() < 1.0,
        "twenty updates a second: {updates}"
    );
    assert!(
        flow.arrival_mean_ms
            .is_some_and(|mean| (mean - 50.0).abs() < 1.0),
        "the cadence is fifty milliseconds"
    );
    assert!(
        flow.arrival_jitter_ms.is_some_and(|jitter| jitter < 1.0),
        "a regular stream is not a jittery one"
    );
    assert_eq!(flow.stall_ms, None, "traffic is flowing both ways");
    assert!(flow.received_bytes_per_sec > 0.0);
}

#[test]
fn an_endpoint_with_no_flow_events_has_no_flow_column() {
    // A machine without the one-time tracing setup discovers TCP endpoints from the
    // connection table and counts nothing. An empty column must read as absent, never as
    // zero throughput on an endpoint the user is playing over.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    assert_eq!(flat(&view)[0].flow, None);
}

#[test]
fn an_idle_endpoint_stops_reporting_a_flow_column() {
    // The passive figures live on the operating system's event clock, which cannot be
    // compared with ours — so this history has no way of knowing it has gone stale. An
    // endpoint the application stopped using would otherwise go on showing the arrival
    // pattern of a match that ended.
    let (mut monitor, mut registry, now) = monitor();
    play(&mut monitor, udp(1), now, 0, 200);
    let _ = registered(&mut monitor, &mut registry, now);

    let fresh = view(&mut monitor, now);
    assert!(flat(&fresh)[0].flow.is_some());

    // Liveness moves on the sweep, which is where the tracker ages everything.
    let later = now + Duration::from_secs(60);
    let _ = monitor.sweep(&mut registry, later);
    let stale = flat(&view(&mut monitor, later));
    assert_eq!(stale[0].liveness, nm_app::LivenessView::Idle);
    assert_eq!(stale[0].flow, None);
}

#[test]
fn sending_into_silence_reports_a_stall_rather_than_a_loss_figure() {
    // A one-way outage, seen without a probe. It is deliberately not called loss: only the
    // far end knows what it sent, so a datagram that never arrived is invisible from here.
    let (mut monitor, mut registry, now) = monitor();
    play(&mut monitor, udp(1), now, 0, 100);
    for index in 0..20 {
        monitor
            .observe(
                APP,
                udp(1),
                None,
                Some(FlowObservation::sent(
                    FlowInstant::from_origin(Duration::from_millis(5_100 + index * 50)),
                    64,
                )),
                now,
            )
            .unwrap();
    }
    let _ = registered(&mut monitor, &mut registry, now);

    let flow = flat(&view(&mut monitor, now))[0]
        .flow
        .expect("the endpoint is still in use");
    let stall = flow.stall_ms.expect("a second of unanswered sending");
    assert!(
        stall > 900.0 && stall < 1_200.0,
        "the stall is measured from the last arrival: {stall}"
    );
}

/// A TCP endpoint, which is the only kind the operating system publishes a round trip for.
fn tcp(last: u8) -> EndpointKey {
    EndpointKey::tcp(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, last)),
        443,
    ))
}

fn stack_rtt(endpoint: EndpointKey) -> PassiveRtt {
    PassiveRtt {
        endpoint,
        rtt: Duration::from_micros(24_500),
        min_rtt: Duration::from_millis(21),
        max_rtt: Duration::from_millis(90),
        established_for: Some(Duration::from_secs(600)),
    }
}

#[test]
fn the_operating_systems_own_round_trip_reaches_the_page_with_its_age() {
    // A real round trip to the endpoint, measured by the stack on the application's own
    // connection at no cost in packets. It arrives every few tens of seconds at best, so the
    // age travels with it — a figure that may be a minute old must not read as current.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, tcp(1), None, None, now).unwrap();
    let _ = registered(&mut monitor, &mut registry, now);
    monitor.note_passive_rtt(&stack_rtt(tcp(1)), now);

    let passive = flat(&view(&mut monitor, now + Duration::from_secs(12)))[0]
        .passive_rtt
        .expect("the stack published a round trip for this connection");

    assert!((passive.rtt_ms - 24.5).abs() < 0.001);
    assert!((passive.min_rtt_ms - 21.0).abs() < 0.001);
    assert!((passive.max_rtt_ms - 90.0).abs() < 0.001);
    assert!(
        (passive.age_secs - 12.0).abs() < 0.5,
        "the age is what stops a stale figure reading as live: {}",
        passive.age_secs
    );
}

#[test]
fn a_tcp_connection_reports_how_long_it_has_been_established() {
    // What the user asked for: telling a new endpoint from one that has been there all
    // match. TCP has a real answer and the operating system already sends it beside the
    // round trip — it was simply never parsed.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, tcp(1), None, None, now).unwrap();
    let _ = registered(&mut monitor, &mut registry, now);
    monitor.note_passive_rtt(&stack_rtt(tcp(1)), now);

    let age = flat(&view(&mut monitor, now + Duration::from_secs(30)))[0].age;

    assert_eq!(age.kind, EndpointAgeKindView::Established);
    assert!(
        (age.secs - 630.0).abs() < 0.5,
        "aged forward from the summary rather than frozen at it: {}",
        age.secs
    );
}

#[test]
fn a_udp_endpoint_reports_how_long_it_has_been_watched_instead() {
    // **Two facts, two words.** UDP has no establishment to report, so borrowing the word
    // would be a claim about a connection that does not exist. What can honestly be said is
    // how long this application has been watched talking to it.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let _ = registered(&mut monitor, &mut registry, now);

    let age = flat(&view(&mut monitor, now + Duration::from_secs(45)))[0].age;

    assert_eq!(age.kind, EndpointAgeKindView::Watched);
    assert!((age.secs - 45.0).abs() < 0.5, "{}", age.secs);
}

#[test]
fn a_connection_the_system_never_dated_falls_back_to_being_watched() {
    // The field has moved between Windows versions before. Losing it costs the reader the
    // better fact, never the figure — and never quietly: the kind says which one they got.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, tcp(1), None, None, now).unwrap();
    let _ = registered(&mut monitor, &mut registry, now);
    monitor.note_passive_rtt(
        &PassiveRtt {
            established_for: None,
            ..stack_rtt(tcp(1))
        },
        now,
    );

    let age = flat(&view(&mut monitor, now + Duration::from_secs(20)))[0].age;

    assert_eq!(age.kind, EndpointAgeKindView::Watched);
    assert!((age.secs - 20.0).abs() < 0.5, "{}", age.secs);
}

#[test]
fn a_tunnelled_endpoint_is_refused_the_stacks_round_trip() {
    // The same reason `select_kind` refuses a TCP-connect probe there: a connection to an
    // address a local tunnel remaps terminates on the tunnel, so the stack times the round
    // trip to the user's own router. That is a fake-*good* number, which is worse than a
    // fake-bad one — it would tell someone under censorship that their connection is fine.
    let (mut monitor, mut registry, now) = monitor();
    let sentinel = EndpointKey::tcp(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(198, 18, 0, 7)),
        443,
    ));
    monitor.observe(APP, sentinel, None, None, now).unwrap();
    let _ = registered(&mut monitor, &mut registry, now);
    monitor.note_passive_rtt(&stack_rtt(sentinel), now);

    let endpoints = flat(&view(&mut monitor, now));
    let endpoint = &endpoints[0];
    assert!(endpoint.tunnelled);
    assert_eq!(
        endpoint.passive_rtt, None,
        "the stack measured the tunnel, not the server"
    );
}

#[test]
fn a_round_trip_for_an_endpoint_nobody_is_watching_is_ignored() {
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, tcp(1), None, None, now).unwrap();
    let _ = registered(&mut monitor, &mut registry, now);
    monitor.note_passive_rtt(&stack_rtt(tcp(2)), now);

    assert_eq!(flat(&view(&mut monitor, now))[0].passive_rtt, None);
}

#[test]
fn an_endpoint_that_answers_for_itself_has_no_path_line() {
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(
        &mut monitor,
        ids[0],
        now,
        ProbeOutcome::Success(Rtt::from_micros(9_000)),
    );

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    let endpoints = flat(&view);
    assert!(
        endpoints[0].chart_path_ms.iter().all(Option::is_none),
        "an endpoint that answers needs nothing standing in for it"
    );
    assert!(endpoints[0].chart_rtt_ms.iter().any(Option::is_some));
}

// ------------------------------------------------- the page stops jumping

#[test]
fn a_health_change_shows_at_once_and_moves_the_row_only_once_it_holds() {
    // The failure this exists to fix: a list ordered worst-first re-sorts whenever any
    // member's health changes, and health near a threshold flickers — so the row someone is
    // reading swaps places with its neighbour a second after they started reading it.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    monitor.observe(APP, udp(2), None, None, now).unwrap();
    let ids = registered_by_ip(&mut monitor, &mut registry, now);
    // Both clean, so the order is the tie-break: 1.1.1.1 then 1.1.1.2.
    for key in [udp(1), udp(2)] {
        soak(
            &mut monitor,
            ids[&key.address.ip()],
            now,
            ProbeOutcome::Success(Rtt::from_micros(9_000)),
        );
    }
    let settled = now + WINDOW;
    assert_eq!(
        flat(&view(&mut monitor, settled))[0].address,
        "1.1.1.1:27015"
    );

    // The second endpoint starts losing packets. Its badge must say so immediately.
    for step in 0..8 {
        monitor.record(
            ids[&udp(2).address.ip()],
            ProbeSample::new(settled + Duration::from_secs(step), ProbeOutcome::Timeout),
        );
    }
    let moment = settled + Duration::from_secs(8);
    let endpoints = flat(&view(&mut monitor, moment));
    let broken = endpoints
        .iter()
        .find(|endpoint| endpoint.address == "1.1.1.2:27015")
        .expect("still listed");
    assert_eq!(
        broken.health,
        HealthView::Degraded,
        "the badge is never stale"
    );
    assert_eq!(
        endpoints[0].address, "1.1.1.1:27015",
        "and the row the user was reading has not moved yet"
    );

    // Held for long enough, the ordering adopts it.
    let later = moment + Duration::from_secs(6);
    for step in 0..6 {
        monitor.record(
            ids[&udp(2).address.ip()],
            ProbeSample::new(moment + Duration::from_secs(step), ProbeOutcome::Timeout),
        );
    }
    assert_eq!(
        flat(&view(&mut monitor, later))[0].address,
        "1.1.1.2:27015",
        "a change that persists does move the row"
    );
}

// ------------------------------------------- the first seconds are not findings

#[test]
fn a_new_endpoint_warms_up_instead_of_reporting_its_first_few_samples() {
    // The least informative samples the session will ever have: no window is full, the
    // fallback chain is still trying kinds, ranking has not run. The page used to present
    // all of it as measurement.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(
        &mut monitor,
        ids[0],
        now,
        ProbeOutcome::Success(Rtt::from_micros(9_000)),
    );

    let early = now + Duration::from_secs(u64::from(ENOUGH));
    let endpoints = flat(&view(&mut monitor, early));
    let endpoint = &endpoints[0];

    let remaining = endpoint
        .warmup_secs_remaining
        .expect("the window is nowhere near full");
    assert!(
        remaining > 0.0 && remaining <= WINDOW.as_secs_f64(),
        "the time left is stated rather than implied: {remaining}"
    );
    assert_eq!(
        endpoint.jitter_ms, None,
        "variation over a handful of samples is noise, not a finding"
    );
    assert_eq!(
        endpoint.loss_pct, None,
        "a percentage whose denominator is a handful is not a percentage"
    );
    assert!(
        endpoint.rtt_ms.is_some(),
        "a round trip survives: one reply is one real measurement of the route"
    );
}

#[test]
fn the_warm_up_ends() {
    // Not an indefinite spinner. Once the window behind the figures is real, they appear.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    soak(
        &mut monitor,
        ids[0],
        now,
        ProbeOutcome::Success(Rtt::from_micros(9_000)),
    );

    let endpoints = flat(&view(&mut monitor, now + WINDOW));
    assert_eq!(endpoints[0].warmup_secs_remaining, None);
    assert!(endpoints[0].jitter_ms.is_some());
    assert!(endpoints[0].loss_pct.is_some());
}

#[test]
fn a_warm_up_never_hides_something_already_certain() {
    // Filtering proven, nothing getting through, alive by its own traffic: those are
    // answers, and arriving fast is the point of them. Warm-up suppresses figures that are
    // still noisy, never states that are known.
    let (mut monitor, mut registry, now) = monitor();
    monitor
        .observe(APP, udp(1), None, Some(traffic(630_000)), now)
        .unwrap();
    monitor
        .observe(APP, udp(2), None, Some(traffic(0)), now)
        .unwrap();
    let ids = registered_by_ip(&mut monitor, &mut registry, now);
    fill(
        &mut monitor,
        ids[&udp(1).address.ip()],
        now,
        ProbeOutcome::Timeout,
    );
    fill(
        &mut monitor,
        ids[&udp(2).address.ip()],
        now,
        ProbeOutcome::Timeout,
    );
    monitor.note_probe_state(
        ids[&udp(1).address.ip()],
        Some(ProbeKind::TlsHello),
        false,
        true,
        true,
    );

    let early = now + Duration::from_secs(u64::from(ENOUGH));
    let endpoints = flat(&view(&mut monitor, early));

    assert!(
        endpoints
            .iter()
            .all(|endpoint| endpoint.warmup_secs_remaining.is_some()),
        "both are still warming up"
    );
    let alive = endpoints
        .iter()
        .find(|endpoint| endpoint.address.contains("1.1.1.1"))
        .expect("the endpoint carrying traffic");
    assert_eq!(alive.health, HealthView::CarryingTraffic);
    assert!(alive.filtering_confirmed, "proven filtering is knowledge");

    let dead = endpoints
        .iter()
        .find(|endpoint| endpoint.address.contains("1.1.1.2"))
        .expect("the silent endpoint");
    assert_eq!(
        dead.health,
        HealthView::Unreachable,
        "nothing gets through, and that is an answer rather than a noisy figure"
    );
}

#[test]
fn nothing_is_blamed_on_an_application_that_has_only_just_been_chosen() {
    // The verdict banner waits with the rest. What does not wait is the network underneath
    // it: the baselines have been measured since the session began, and an application being
    // new says nothing about them.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);

    let early = now + Duration::from_secs(u64::from(ENOUGH));
    let warming = warming(&mut monitor, early);
    let settled = view(&mut monitor, early);

    assert!(warming.warmup_secs_remaining.is_some());
    assert_eq!(
        warming.diagnosis.endpoints_affected, 0,
        "no endpoint of this application is being spoken for yet"
    );
    assert_ne!(
        warming.diagnosis.verdict,
        nm_app::VerdictView::RouteToThisApplication,
        "an application nobody has watched for a full window is not the culprit"
    );
    assert_eq!(
        settled.diagnosis.verdict,
        nm_app::VerdictView::RouteToThisApplication,
        "and once the window is real, the same evidence does reach that verdict"
    );
}

// ------------------------------------------------- the match traffic comes first

#[test]
fn the_match_traffic_is_a_group_of_its_own_and_comes_first() {
    // During a game the endpoints that decide whether it plays well are the UDP flows.
    // Ordered by severity alone they sit wherever their health happens to put them, between
    // a launcher's connection and a content network — so the endpoint the user came to look
    // at is the one they have to hunt for.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, tcp(1), None, None, now).unwrap();
    monitor.observe(APP, udp(2), None, None, now).unwrap();
    let ids = registered_by_ip(&mut monitor, &mut registry, now);
    // The TCP endpoint is the broken one, so severity alone would put it first.
    fill(
        &mut monitor,
        ids[&tcp(1).address.ip()],
        now,
        ProbeOutcome::Timeout,
    );
    fill(
        &mut monitor,
        ids[&udp(2).address.ip()],
        now,
        ProbeOutcome::Success(Rtt::from_micros(9_000)),
    );

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    assert_eq!(view.groups[0].transport, TransportView::Udp);
    assert_eq!(view.groups[1].transport, TransportView::Tcp);
    assert_eq!(view.groups[0].endpoints.len(), 1);
    assert_eq!(view.groups[1].endpoints.len(), 1);
    assert_eq!(
        view.groups[0].endpoints[0].health,
        HealthView::Ok,
        "a clean match endpoint still comes before a broken supporting one"
    );
}

#[test]
fn a_group_carries_its_own_distribution_so_a_folded_one_hides_nothing() {
    // TCP is demoted, never hidden: a login service or a content network with a filter on it
    // is what "I cannot get into the game" actually looks like. The group may only start
    // folded when every member is clean, which is what `needs_attention` decides.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, tcp(1), None, None, now).unwrap();
    monitor.observe(APP, tcp(2), None, None, now).unwrap();
    let ids = registered_by_ip(&mut monitor, &mut registry, now);
    fill(
        &mut monitor,
        ids[&tcp(1).address.ip()],
        now,
        ProbeOutcome::Timeout,
    );
    fill(
        &mut monitor,
        ids[&tcp(2).address.ip()],
        now,
        ProbeOutcome::Success(Rtt::from_micros(9_000)),
    );

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    let supporting = &view.groups[1];

    assert_eq!(supporting.counts.unreachable, 1);
    assert_eq!(supporting.counts.ok, 1);
    assert!(supporting.needs_attention);
    assert!(
        !view.groups[0].needs_attention,
        "an empty group has nothing to attend to"
    );
}

#[test]
fn severity_still_orders_the_endpoints_inside_a_group() {
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

    let view = view(&mut monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    let order: Vec<HealthView> = view.groups[0]
        .endpoints
        .iter()
        .map(|endpoint| endpoint.health)
        .collect();
    assert_eq!(order, vec![HealthView::Unreachable, HealthView::Ok]);
}

#[test]
fn it_names_the_network_an_endpoint_belongs_to() {
    // The whole point of the feature: an address is four numbers, and the name beside it is
    // the only part of the row a person can recognise without knowing what any of the
    // figures mean.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let _ = registered(&mut monitor, &mut registry, now);

    let endpoints = flat(&view(&mut monitor, now));
    let network = endpoints[0]
        .network
        .clone()
        .expect("the fixture directory covers this address");
    assert_eq!(network.asn, 64_500);
    assert_eq!(network.name.as_deref(), Some("EXAMPLE-NET Example Transit"));
    assert_eq!(network.country.as_deref(), Some("US"));
}

#[test]
fn an_endpoint_the_directory_does_not_know_is_left_unnamed() {
    // Absent stays absent. There is no nearest network to fall back to, and inventing one
    // would be the single worst thing this feature could do — a wrong name is not a missing
    // name, it is a false statement about where someone's traffic went.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(2), None, None, now).unwrap();
    let _ = registered(&mut monitor, &mut registry, now);

    let endpoints = flat(&view(&mut monitor, now));
    assert_eq!(endpoints[0].address, "1.1.1.2:27015");
    assert!(endpoints[0].network.is_none());
}

#[test]
fn with_the_directory_unloaded_nothing_is_named_and_nothing_else_changes() {
    // The state before the load lands and after the user switches the setting off. The row
    // must lose its label and keep every measurement — the names are enrichment, and a
    // failure to enrich may not cost anyone a figure.
    let (mut monitor, mut registry, now) = monitor();
    monitor.observe(APP, udp(1), None, None, now).unwrap();
    let ids = registered(&mut monitor, &mut registry, now);
    soak(
        &mut monitor,
        ids[0],
        now,
        ProbeOutcome::Success(Rtt::from_micros(9_000)),
    );
    let settled = now + WINDOW;

    let named = flat(&view_named(&mut monitor, settled, None, &names()));
    let unnamed = flat(&view_named(
        &mut monitor,
        settled,
        None,
        &NetworkNames::none(),
    ));

    assert!(named[0].network.is_some());
    assert!(unnamed[0].network.is_none());
    assert_eq!(named[0].address, unnamed[0].address);
    assert_eq!(named[0].health, unnamed[0].health);
    assert_eq!(named[0].rtt_ms, unnamed[0].rtt_ms);
}
