//! Tests for the bundled reference pools, the learned-endpoint file, and the trickle.
//!
//! In `tests/` rather than in a `#[cfg(test)]` module because `nm-app`'s library target
//! sets `test = false`; see `tests.manifest` for the Windows loader constraint behind it.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use nm_app::apps::TargetChange;
use nm_app::pools::{BundledPool, LearnedPools, PoolMonitor, PoolSeeds, POOL_INTERVAL};
use nm_core::endpoint::AppId;
use nm_core::health::HealthThresholds;
use nm_core::sample::{ProbeOutcome, ProbeSample, Rtt};
use nm_core::target::{TargetAddress, TargetId, TargetRegistry};

/// The preset every test here pretends to be watching.
const PRESET: &str = "cs2";

fn app() -> AppId {
    AppId::new(1)
}

/// A documentation-range address, so nothing here can name a real machine.
fn address(last: u8) -> TargetAddress {
    TargetAddress::icmp(IpAddr::V4(Ipv4Addr::new(203, 0, 113, last)))
}

fn epoch(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

/// A scratch directory that cleans up after itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("nm-pools-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    fn file(&self) -> PathBuf {
        self.0.join("game-servers.json")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every address a batch of changes asks the probe engine to start measuring.
fn registered(changes: &[TargetChange]) -> Vec<TargetAddress> {
    changes
        .iter()
        .filter_map(|change| match change {
            TargetChange::Register { address, .. } => Some(*address),
            _ => None,
        })
        .collect()
}

/// Every handle a batch of changes asks the probe engine to stop measuring.
fn unregistered(changes: &[TargetChange]) -> Vec<TargetId> {
    changes
        .iter()
        .filter_map(|change| match change {
            TargetChange::Unregister { id } => Some(*id),
            _ => None,
        })
        .collect()
}

#[test]
fn the_bundled_pools_parse() {
    // They are compiled in, so a typo is a broken app rather than a broken file.
    let seeds = PoolSeeds::bundled().expect("the bundled pools must validate");
    assert!(
        !seeds.for_preset("cs2").is_empty(),
        "Counter-Strike has a published relay network and must be seeded from it"
    );
}

#[test]
fn one_relay_network_seeds_every_title_that_uses_it() {
    // Counter-Strike and Dota reach the player through the same relays; duplicating the
    // file per game would guarantee the copies drift apart.
    let seeds = PoolSeeds::bundled().unwrap();
    assert_eq!(seeds.for_preset("cs2"), seeds.for_preset("dota2"));
}

#[test]
fn a_title_with_no_published_reference_address_is_seeded_with_nothing() {
    // The ordinary case, and a real state rather than a gap to paper over: most operators
    // publish nothing that answers a probe, so those pools are whatever this machine learns.
    let seeds = PoolSeeds::bundled().unwrap();
    assert!(seeds.for_preset("valorant").is_empty());
}

#[test]
fn bundled_seeds_stay_inside_the_probe_budget() {
    let seeds = PoolSeeds::bundled().unwrap();
    for preset in ["cs2", "dota2"] {
        let count = seeds.for_preset(preset).len();
        assert!(count <= 12, "{preset} has {count} seeds");
    }
}

#[test]
fn rejects_a_pool_that_seeds_nothing() {
    let json = r#"{"schemaVersion":1,"id":"p","label":"P","applications":[],"targets":[
        {"id":"a","label":"A","address":"203.0.113.1"}
    ]}"#;
    let error = BundledPool::parse(json).unwrap_err().to_string();
    assert!(error.contains("seeds no application"), "{error}");
}

#[test]
fn rejects_a_pool_with_no_targets() {
    let json = r#"{"schemaVersion":1,"id":"p","label":"P","applications":["cs2"],"targets":[]}"#;
    let error = BundledPool::parse(json).unwrap_err().to_string();
    assert!(error.contains("no targets"), "{error}");
}

#[test]
fn rejects_a_pool_target_that_is_not_an_address() {
    // A name would be resolved once at start-up and then measured forever against whatever
    // it happened to point at — which for a relay network is the wrong machine within days.
    let json = r#"{"schemaVersion":1,"id":"p","label":"P","applications":["cs2"],"targets":[
        {"id":"a","label":"A","address":"relay.example.net"}
    ]}"#;
    let error = BundledPool::parse(json).unwrap_err().to_string();
    assert!(error.contains("not an address literal"), "{error}");
}

#[test]
fn rejects_a_newer_schema_instead_of_guessing_at_it() {
    let json = r#"{"schemaVersion":2,"id":"p","label":"P","applications":["cs2"],"targets":[
        {"id":"a","label":"A","address":"203.0.113.1"}
    ]}"#;
    assert!(BundledPool::parse(json).is_err());
}

#[test]
fn tracking_an_application_starts_probing_its_seeds_at_the_trickle() {
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(
        PoolSeeds::bundled().unwrap(),
        HealthThresholds::default(),
        true,
    );

    let changes = pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));

    assert!(!registered(&changes).is_empty());
    for change in &changes {
        if let TargetChange::Register {
            interval, source, ..
        } = change
        {
            assert_eq!(*interval, POOL_INTERVAL);
            // Never source-bound: a pool asks whether the game's infrastructure answers at
            // all, not what one application's route to it looks like.
            assert_eq!(*source, None);
        }
    }
}

#[test]
fn an_application_with_no_seeds_and_nothing_learned_has_no_pool_at_all() {
    // An absent pool can neither report an outage nor rule one out, so it must be `None`
    // rather than an empty reading that would read as clean.
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), true);

    let changes = pools.track(app(), "valorant", &[], &mut registry, epoch(1_000));

    assert!(changes.is_empty());
    assert_eq!(pools.reading(app(), std::time::Instant::now()), None);
}

#[test]
fn a_learned_endpoint_joins_the_pool_on_the_next_sweep() {
    // Not immediately: a burst of discovery must not become a burst of registrations.
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), true);
    pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));

    pools.observe(app(), address(10), epoch(1_001));
    let changes = pools.sweep(&mut registry, epoch(1_002));

    assert_eq!(registered(&changes), vec![address(10)]);
}

#[test]
fn nothing_is_remembered_when_the_user_asked_for_nothing_to_be() {
    // The whole promise of the setting: not "stop adding to a record" but "record nothing".
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), false);
    pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));

    pools.observe(app(), address(10), epoch(1_001));
    let changes = pools.sweep(&mut registry, epoch(1_002));

    assert!(changes.is_empty());
    assert!(!pools.take_learned_change());
    assert_eq!(pools.reading(app(), std::time::Instant::now()), None);
}

#[test]
fn a_sweep_that_changes_nothing_asks_the_probe_engine_for_nothing() {
    // It runs every five seconds for as long as a game is monitored; a steady pool must
    // cost nothing at all.
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(
        PoolSeeds::bundled().unwrap(),
        HealthThresholds::default(),
        true,
    );
    pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));

    assert!(pools.sweep(&mut registry, epoch(1_005)).is_empty());
    assert!(pools.sweep(&mut registry, epoch(1_010)).is_empty());
}

#[test]
fn forgetting_an_application_releases_its_pool() {
    // A trickle is only justified while the user is watching the game it describes.
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(
        PoolSeeds::bundled().unwrap(),
        HealthThresholds::default(),
        true,
    );
    let started = pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));

    let released = pools.forget(app(), &mut registry);

    assert_eq!(unregistered(&released).len(), registered(&started).len());
    assert!(registry.is_empty(), "nothing may be left being probed");
    assert_eq!(pools.reading(app(), std::time::Instant::now()), None);
}

#[test]
fn an_expired_entry_stops_being_probed() {
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), true);
    pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));
    pools.observe(app(), address(10), epoch(1_000));
    let started = pools.sweep(&mut registry, epoch(1_001));
    assert_eq!(registered(&started).len(), 1);

    // A fortnight and a day later.
    let released = pools.sweep(&mut registry, epoch(1_000 + 15 * 24 * 60 * 60));

    assert_eq!(unregistered(&released).len(), 1);
    assert_eq!(pools.reading(app(), std::time::Instant::now()), None);
}

#[test]
fn a_pool_reports_the_share_of_its_members_that_answer() {
    let now = std::time::Instant::now();
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), true);
    pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));
    for last in 10..14 {
        pools.observe(app(), address(last), epoch(1_000));
    }
    let changes = pools.sweep(&mut registry, epoch(1_001));

    // Every member answered half an hour ago — which is what earns it a place in the ratio
    // at all — and then two of them went silent. The old answer sits outside the window the
    // verdict is computed over, so it proves the member *can* answer without propping up a
    // figure about how it is doing now.
    let long_ago = now
        .checked_sub(Duration::from_secs(1_800))
        .expect("the test clock has a past");
    let mut answering = true;
    for change in &changes {
        if let TargetChange::Register { id, .. } = change {
            pools.record(
                *id,
                ProbeSample::new(long_ago, ProbeOutcome::Success(Rtt::from_micros(40_000))),
            );
            let outcome = if answering {
                ProbeOutcome::Success(Rtt::from_micros(40_000))
            } else {
                ProbeOutcome::Timeout
            };
            for _ in 0..3 {
                pools.record(*id, ProbeSample::new(now, outcome));
            }
            answering = !answering;
        }
    }

    let report = pools.reading(app(), now).expect("the pool has members");
    assert_eq!(report.seeded, 0);
    assert_eq!(report.learned, 4);
    assert_eq!(report.reading.unproven, 0);
    assert_eq!(report.reading.answering_ratio(), Some(0.5));
}

#[test]
fn learned_match_servers_that_answer_nothing_never_become_an_outage() {
    // The failure this rule exists to prevent, and it is the *normal* case: a learned
    // member is an endpoint a game connected to, and for a UDP title that is a match server
    // which answers nothing anyone can send while the match runs perfectly. Counted as
    // unreachable, a pool of those would report a working game as down on every match.
    let now = std::time::Instant::now();
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), true);
    pools.track(app(), "valorant", &[], &mut registry, epoch(1_000));
    for last in 10..14 {
        pools.observe(app(), address(last), epoch(1_000));
    }
    let changes = pools.sweep(&mut registry, epoch(1_001));

    for change in &changes {
        if let TargetChange::Register { id, .. } = change {
            for _ in 0..6 {
                pools.record(*id, ProbeSample::new(now, ProbeOutcome::Timeout));
            }
        }
    }

    let report = pools.reading(app(), now).expect("the pool has members");
    assert_eq!(report.reading.unproven, 4);
    assert_eq!(
        report.reading.answering_ratio(),
        None,
        "an address the app has no baseline for cannot say that anything changed"
    );
}

#[test]
fn a_probe_belonging_to_nothing_here_is_ignored() {
    // Baselines, application endpoints, services and pools share one probe engine, so every
    // monitor is told about every result and must drop the ones that are not its business.
    let now = std::time::Instant::now();
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(
        PoolSeeds::bundled().unwrap(),
        HealthThresholds::default(),
        true,
    );
    pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));

    let stranger = registry
        .insert(address(200), nm_core::target::TargetTag::AppEndpoint)
        .unwrap();
    pools.record(
        stranger,
        ProbeSample::new(now, ProbeOutcome::Success(Rtt::from_micros(1_000))),
    );

    let report = pools.reading(app(), now).unwrap();
    // The stranger's success must not have landed on a pool member: every seed is still
    // waiting to prove it answers.
    assert_eq!(report.reading.unproven, report.seeded);
    assert_eq!(report.reading.counts.total(), 0);
}

#[test]
fn learned_endpoints_survive_a_round_trip_through_the_file() {
    let scratch = Scratch::new("roundtrip");
    let mut store = LearnedPools::default();
    store.set(PRESET, vec![(address(10), epoch(1_700_000_000))]);

    store
        .store(&scratch.file())
        .expect("the directory is created");
    let loaded = LearnedPools::load(&scratch.file());

    assert_eq!(
        loaded.for_preset(PRESET),
        vec![(address(10), epoch(1_700_000_000))]
    );
}

#[test]
fn a_missing_file_is_a_first_run_rather_than_a_failure() {
    let scratch = Scratch::new("missing");
    assert!(LearnedPools::load(&scratch.file()).is_empty());
}

#[test]
fn an_unreadable_file_is_left_exactly_as_it_is() {
    // The same bargain the settings file makes: a parsing bug of ours must not destroy
    // something a fixed build would have read.
    let scratch = Scratch::new("garbage");
    std::fs::create_dir_all(&scratch.0).unwrap();
    std::fs::write(scratch.file(), "{ not json").unwrap();

    assert!(LearnedPools::load(&scratch.file()).is_empty());
    assert_eq!(
        std::fs::read_to_string(scratch.file()).unwrap(),
        "{ not json"
    );
}

#[test]
fn a_file_from_an_unknown_schema_is_not_guessed_at() {
    let scratch = Scratch::new("schema");
    std::fs::create_dir_all(&scratch.0).unwrap();
    std::fs::write(scratch.file(), r#"{"schemaVersion":99,"pools":{}}"#).unwrap();

    assert!(LearnedPools::load(&scratch.file()).is_empty());
}

#[test]
fn merging_keeps_what_was_learned_about_games_that_are_not_being_watched() {
    // Writing only the tracked applications would erase yesterday's memory of every other
    // title on the first save.
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), true);
    pools.track(app(), PRESET, &[], &mut registry, epoch(1_000));
    pools.observe(app(), address(10), epoch(1_000));

    let mut store = LearnedPools::default();
    store.set("dota2", vec![(address(50), epoch(900))]);
    pools.merge_learned_into(&mut store);

    assert_eq!(store.for_preset(PRESET), vec![(address(10), epoch(1_000))]);
    assert_eq!(store.for_preset("dota2"), vec![(address(50), epoch(900))]);
}

#[test]
fn what_was_learned_before_is_probed_again_next_session() {
    // The cold start the seeds cover for two titles and nothing covers for the rest.
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), true);

    let changes = pools.track(
        app(),
        "valorant",
        &[(address(10), epoch(1_000))],
        &mut registry,
        epoch(1_100),
    );

    assert_eq!(registered(&changes), vec![address(10)]);
}

#[test]
fn a_stored_entry_that_has_already_expired_is_never_probed() {
    let mut registry = TargetRegistry::new();
    let mut pools = PoolMonitor::new(PoolSeeds::empty(), HealthThresholds::default(), true);

    let changes = pools.track(
        app(),
        "valorant",
        &[(address(10), epoch(1_000))],
        &mut registry,
        epoch(1_000 + 30 * 24 * 60 * 60),
    );

    assert!(changes.is_empty());
    assert_eq!(pools.reading(app(), std::time::Instant::now()), None);
}
