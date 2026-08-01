//! Tests for the bundled status-page service list and its validation.
//!
//! In `tests/` rather than in a `#[cfg(test)]` module because `nm-app`'s library target
//! sets `test = false`; see `tests.manifest` for the Windows loader constraint behind it.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::time::Duration;

use nm_app::services::{ProbeKindHint, ServiceGroup, ServiceList};
use nm_app::status::{CHECK_INTERVAL, HISTORY_CAPACITY, TIMELINE_POINTS};

fn bundled() -> ServiceList {
    ServiceList::bundled().expect("the bundled service list must validate")
}

/// Every endpoint of every bundled service.
fn endpoints() -> Vec<(String, String)> {
    bundled()
        .services
        .iter()
        .flat_map(|service| {
            service.endpoints.iter().map(|endpoint| {
                (
                    format!("{}/{}", service.id, endpoint.id),
                    endpoint.address.clone(),
                )
            })
        })
        .collect()
}

#[test]
fn the_bundled_list_parses() {
    // The list is compiled in, so a typo in it is a broken app rather than a broken file —
    // this test is what turns that back into a build-time failure.
    assert!(!bundled().services.is_empty());
}

#[test]
fn the_bundled_list_covers_both_shelves() {
    let services = bundled().services;
    for group in ServiceGroup::ALL {
        assert!(
            services.iter().any(|service| service.group == group),
            "{group:?} has no services, so the page would show an empty heading"
        );
    }
}

#[test]
fn the_bundled_list_stays_far_inside_the_probe_budget() {
    // The status page is probed whether or not the user is doing anything, so it is the one
    // list whose cost has to be negligible rather than merely acceptable. The product's cap
    // is 32 probes a second across everything.
    let count = endpoints().len();
    // A list this size converts to a float exactly; the fallback keeps the cast total.
    let per_sec =
        f64::from(u32::try_from(count).unwrap_or(u32::MAX)) / CHECK_INTERVAL.as_secs_f64();
    assert!(
        per_sec < 1.0,
        "{count} endpoints at one check every {CHECK_INTERVAL:?} is {per_sec:.2} probes/s"
    );
}

#[test]
fn every_bundled_endpoint_carries_a_port() {
    // Without one only ICMP can be used, and a front door that drops echoes would then have
    // no fallback at all — which for a status page means a permanently red card about a
    // service that is up.
    for service in bundled().services {
        for endpoint in service.endpoints {
            assert!(
                endpoint.port.is_some(),
                "{}/{} has no port",
                service.id,
                endpoint.id
            );
        }
    }
}

#[test]
fn every_bundled_endpoint_is_a_published_name_rather_than_an_address() {
    // A platform's front door lives on a content network whose address depends on where the
    // user is. Pinning one would measure whichever edge the *developer* was nearest, and
    // would go stale silently.
    for (key, address) in endpoints() {
        assert!(
            address.parse::<std::net::IpAddr>().is_err(),
            "{key} names a literal address"
        );
    }
}

#[test]
fn bundled_endpoint_keys_are_unique_across_the_whole_list() {
    // They are React keys and the map key the monitor records checks under.
    let mut keys: Vec<String> = endpoints().into_iter().map(|(key, _)| key).collect();
    let total = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), total);
}

#[test]
fn the_timeline_never_asks_for_more_checks_than_are_retained() {
    const { assert!(TIMELINE_POINTS <= HISTORY_CAPACITY) }
}

#[test]
fn the_check_interval_stays_in_the_range_a_status_page_is_worth() {
    // Faster spends budget on a question that changes by the minute; slower and a card is
    // stale for longer than a user will wait before deciding the app is broken.
    assert!(CHECK_INTERVAL >= Duration::from_secs(30));
    assert!(CHECK_INTERVAL <= Duration::from_secs(60));
}

#[test]
fn rejects_a_newer_schema_instead_of_guessing_at_it() {
    let json = r#"{"schemaVersion":2,"services":[
        {"id":"a","label":"A","group":"infrastructure","endpoints":[
            {"id":"e","address":"example.net","port":443}
        ]}
    ]}"#;
    let error = ServiceList::parse(json).unwrap_err().to_string();
    assert!(error.contains("schema version 2"), "{error}");
}

#[test]
fn rejects_an_empty_list() {
    assert!(ServiceList::parse(r#"{"schemaVersion":1,"services":[]}"#).is_err());
}

#[test]
fn rejects_a_service_with_no_endpoints() {
    // It would render as a card that can never say anything, forever.
    let json = r#"{"schemaVersion":1,"services":[
        {"id":"a","label":"A","group":"infrastructure","endpoints":[]}
    ]}"#;
    let error = ServiceList::parse(json).unwrap_err().to_string();
    assert!(error.contains("no endpoints"), "{error}");
}

#[test]
fn rejects_duplicate_service_identifiers() {
    let json = r#"{"schemaVersion":1,"services":[
        {"id":"a","label":"A","group":"infrastructure","endpoints":[
            {"id":"e","address":"example.net","port":443}
        ]},
        {"id":"a","label":"B","group":"infrastructure","endpoints":[
            {"id":"e","address":"example.org","port":443}
        ]}
    ]}"#;
    let error = ServiceList::parse(json).unwrap_err().to_string();
    assert!(error.contains("duplicate service"), "{error}");
}

#[test]
fn rejects_duplicate_endpoint_identifiers_within_one_service() {
    let json = r#"{"schemaVersion":1,"services":[
        {"id":"a","label":"A","group":"infrastructure","endpoints":[
            {"id":"e","address":"example.net","port":443},
            {"id":"e","address":"example.org","port":443}
        ]}
    ]}"#;
    let error = ServiceList::parse(json).unwrap_err().to_string();
    assert!(error.contains("duplicate endpoint"), "{error}");
}

#[test]
fn rejects_an_unknown_field_rather_than_ignoring_it() {
    // A misspelled key is a service that would never be checked, and must be loud.
    let json = r#"{"schemaVersion":1,"services":[
        {"id":"a","label":"A","grup":"infrastructure","endpoints":[
            {"id":"e","address":"example.net","port":443}
        ]}
    ]}"#;
    assert!(ServiceList::parse(json).is_err());
}

#[test]
fn rejects_an_unknown_group_rather_than_shelving_it_somewhere() {
    let json = r#"{"schemaVersion":1,"services":[
        {"id":"a","label":"A","group":"somewhereElse","endpoints":[
            {"id":"e","address":"example.net","port":443}
        ]}
    ]}"#;
    assert!(ServiceList::parse(json).is_err());
}

#[test]
fn a_probe_kind_hint_is_optional_and_parses_by_name() {
    let json = r#"{"schemaVersion":1,"services":[
        {"id":"a","label":"A","group":"infrastructure","probeKind":"tlsHello","endpoints":[
            {"id":"e","address":"example.net","port":443}
        ]},
        {"id":"b","label":"B","group":"infrastructure","endpoints":[
            {"id":"e","address":"example.org","port":443}
        ]}
    ]}"#;
    let list = ServiceList::parse(json).unwrap();
    assert_eq!(list.services[0].probe_kind, Some(ProbeKindHint::TlsHello));
    assert_eq!(list.services[1].probe_kind, None);
}

#[test]
fn rejects_a_probe_kind_the_engine_has_no_prober_for() {
    let json = r#"{"schemaVersion":1,"services":[
        {"id":"a","label":"A","group":"infrastructure","probeKind":"httpHead","endpoints":[
            {"id":"e","address":"example.net","port":443}
        ]}
    ]}"#;
    assert!(ServiceList::parse(json).is_err());
}
