//! Tests for the bundled baseline target lists and their validation.
//!
//! In `tests/` rather than in a `#[cfg(test)]` module because `nm-app`'s library target
//! sets `test = false`; see `tests.manifest` for the Windows loader constraint behind it.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share. The
// reasoning is identical, so it is granted for this test-only file rather than pushed
// into every call site.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::net::IpAddr;

use nm_app::baselines::{
    countries, domestic, foreign, has_country, literal_address, BaselineGroup, ListedTarget,
    TargetList,
};
use nm_app::Error;

fn listed(address: &str, port: Option<u16>) -> ListedTarget {
    ListedTarget {
        id: "entry".to_owned(),
        label: "Entry".to_owned(),
        address: address.to_owned(),
        port,
    }
}

/// Every bundled list, foreign first.
fn all_lists() -> Vec<TargetList> {
    std::iter::once(foreign().expect("the bundled foreign list must validate"))
        .chain(countries().into_iter().map(|country| {
            domestic(country)
                .unwrap_or_else(|error| panic!("bundled list {country} must validate: {error}"))
        }))
        .collect()
}

#[test]
fn every_bundled_list_parses() {
    // The lists are compiled in, so a typo in one is a broken app rather than a broken
    // file — this test is what turns it back into a build-time failure.
    assert_eq!(all_lists().len(), countries().len() + 1);
    assert!(!countries().is_empty());
}

#[test]
fn bundled_lists_stay_within_the_probe_budget() {
    // Every entry costs budget for as long as the app runs; a list that quietly grew
    // would eat the cap that per-app monitoring needs.
    for list in all_lists() {
        assert!(!list.targets.is_empty(), "{}", list.id);
        assert!(list.targets.len() <= 6, "the {} list grew", list.id);
    }
}

#[test]
fn bundled_entries_carry_a_port_so_the_fallback_chain_has_somewhere_to_go() {
    // Without a port only ICMP can be used, and an endpoint that drops echoes would then
    // have no fallback at all.
    for list in all_lists() {
        for target in &list.targets {
            assert!(target.port.is_some(), "{}/{}", list.id, target.id);
        }
    }
}

#[test]
fn an_unknown_country_is_refused_rather_than_silently_empty() {
    assert!(matches!(domestic("xx"), Err(Error::UnknownCountry { .. })));
    assert!(has_country("ru"));
    assert!(!has_country("xx"));
}

#[test]
fn rejects_a_newer_schema_instead_of_guessing_at_it() {
    let json =
        r#"{"schemaVersion":2,"id":"ru","targets":[{"id":"a","label":"A","address":"1.1.1.1"}]}"#;
    let error = TargetList::parse("ru", json).unwrap_err().to_string();
    assert!(error.contains("schema version 2"), "{error}");
}

#[test]
fn rejects_a_list_loaded_under_the_wrong_name() {
    let json =
        r#"{"schemaVersion":1,"id":"ir","targets":[{"id":"a","label":"A","address":"1.1.1.1"}]}"#;
    assert!(TargetList::parse("ru", json).is_err());
}

#[test]
fn rejects_an_empty_list() {
    assert!(TargetList::parse("ru", r#"{"schemaVersion":1,"id":"ru","targets":[]}"#).is_err());
}

#[test]
fn rejects_duplicate_entry_identifiers() {
    // Two entries sharing a key would collide in the UI and in the snapshot map.
    let json = r#"{"schemaVersion":1,"id":"ru","targets":[
        {"id":"a","label":"A","address":"1.1.1.1"},
        {"id":"a","label":"B","address":"8.8.8.8"}
    ]}"#;
    let error = TargetList::parse("ru", json).unwrap_err().to_string();
    assert!(error.contains("duplicate"), "{error}");
}

#[test]
fn rejects_an_unknown_field_rather_than_ignoring_it() {
    // A misspelled key must not silently become a default.
    let json = r#"{"schemaVersion":1,"id":"ru","targets":[
        {"id":"a","label":"A","address":"1.1.1.1","prt":443}
    ]}"#;
    assert!(TargetList::parse("ru", json).is_err());
}

#[test]
fn rejects_malformed_json_naming_the_list() {
    let error = TargetList::parse("ru", "{").unwrap_err().to_string();
    assert!(error.contains("\"ru\""), "{error}");
}

#[test]
fn an_address_literal_needs_no_lookup() {
    let target = literal_address(&listed("1.1.1.1", Some(443))).unwrap();
    assert_eq!(target.ip, "1.1.1.1".parse::<IpAddr>().unwrap());
    assert_eq!(target.port, Some(443));
}

#[test]
fn an_address_literal_without_a_port_is_icmp_only() {
    let target = literal_address(&listed("2606:4700:4700::1111", None)).unwrap();
    assert_eq!(target.port, None);
}

#[test]
fn a_host_name_is_not_mistaken_for_an_address() {
    assert_eq!(literal_address(&listed("discord.com", Some(443))), None);
    // Nor is something that merely looks numeric.
    assert_eq!(literal_address(&listed("1.1.1.1.1", Some(443))), None);
}

#[test]
fn groups_are_listed_in_dashboard_order() {
    assert_eq!(
        BaselineGroup::ALL,
        [BaselineGroup::Domestic, BaselineGroup::Foreign]
    );
}
