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
use nm_app::discovery::PassiveRtt;
use nm_app::{AppProcessView, AppView, HealthCountsView, HealthView, ProbeKindView, TransportView};
use nm_core::address::AddressPolicy;
use nm_core::endpoint::{AppId, EndpointKey, LifecyclePolicy};
use nm_core::flow::{FlowInstant, FlowObservation};
use nm_core::health::HealthThresholds;
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

fn view(monitor: &AppMonitor, now: Instant) -> AppView {
    AppView::of(
        APP_ID,
        "game.exe".to_owned(),
        vec![AppProcessView {
            pid: PID,
            name: "game.exe".to_owned(),
        }],
        monitor.chart_ages_secs(),
        &adapters(),
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
        .observe(APP, sentinel, Some(egress), Some(traffic(4_096)), now)
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
    fill(&mut monitor, ids[0], now, ProbeOutcome::Timeout);

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    let endpoint = &view.endpoints[0];

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

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    assert_eq!(view.endpoints[0].health, HealthView::Unreachable);
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

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    assert_eq!(view.endpoints[0].health, HealthView::Degraded);
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
    assert!(
        endpoint.chart_rtt_ms.iter().all(Option::is_none),
        "an unmeasured endpoint occupies the chart's axis and draws nothing on it"
    );
}

#[test]
fn an_application_with_nothing_discovered_renders_empty_rather_than_missing() {
    let (monitor, _registry, now) = monitor();
    let view = view(&monitor, now);
    assert!(view.endpoints.is_empty());
    assert_eq!(view.counts, HealthCountsView::default());
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

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    let slots = view.chart_age_secs.len();
    assert!(slots > 1);
    assert_eq!(
        view.chart_age_secs.last().copied(),
        Some(0.0),
        "the axis ends at now, so a slow endpoint trails off to the left"
    );
    for endpoint in &view.endpoints {
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

    let view = view(&monitor, now + Duration::from_secs(11));
    let drawn = &view.endpoints[0].chart_rtt_ms;

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

    let view = view(&monitor, now + Duration::from_secs(2));

    assert!(
        view.endpoints[0].chart_rtt_ms.contains(&Some(90.0)),
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

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    let endpoint = &view.endpoints[0];

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

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));
    assert_eq!(view.endpoints[0].flow, None);
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

    let fresh = view(&monitor, now);
    assert!(fresh.endpoints[0].flow.is_some());

    // Liveness moves on the sweep, which is where the tracker ages everything.
    let later = now + Duration::from_secs(60);
    let _ = monitor.sweep(&mut registry, later);
    let stale = view(&monitor, later);
    assert_eq!(stale.endpoints[0].liveness, nm_app::LivenessView::Idle);
    assert_eq!(stale.endpoints[0].flow, None);
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

    let flow = view(&monitor, now).endpoints[0]
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

    let endpoint = &view(&monitor, now + Duration::from_secs(12)).endpoints[0];
    let passive = endpoint
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

    let endpoint = &view(&monitor, now).endpoints[0];
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

    assert_eq!(view(&monitor, now).endpoints[0].passive_rtt, None);
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

    let view = view(&monitor, now + Duration::from_secs(u64::from(ENOUGH)));

    assert!(
        view.endpoints[0].chart_path_ms.iter().all(Option::is_none),
        "an endpoint that answers needs nothing standing in for it"
    );
    assert!(view.endpoints[0].chart_rtt_ms.iter().any(Option::is_some));
}
