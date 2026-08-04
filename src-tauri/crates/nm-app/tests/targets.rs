//! Tests for the bundled target inventory and its validation.
//!
//! One inventory, where there were two schemas parsed by two modules — and where two of the
//! four foreign baseline entries were literally the same addresses as two service endpoints,
//! probed twice and drawn twice for one fact. The last test in this file is what stops that
//! coming back.
//!
//! In `tests/` rather than in a `#[cfg(test)]` module because `nm-app`'s library target sets
//! `test = false`; see `tests.manifest` for the Windows loader constraint behind it.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share. The
// reasoning is identical, so it is granted for this test-only file rather than pushed into
// every call site.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::HashMap;

use nm_app::targets::{
    bundled, countries, domestic, foreign, has_country, is_selected, refuse_duplicate_addresses,
    services, ProbeKindHint, ResolvedTarget, Section, TargetList, SLOW_CHECK_INTERVAL,
};
use nm_app::Error;

/// Every bundled list for every country, which is the whole inventory this build ships.
fn all_lists() -> Vec<TargetList> {
    let mut lists = vec![
        foreign().expect("the bundled foreign list must validate"),
        services().expect("the bundled service list must validate"),
    ];
    for country in countries() {
        lists.push(
            domestic(country)
                .unwrap_or_else(|error| panic!("bundled list {country} must validate: {error}")),
        );
    }
    lists
}

/// Which section every bundled target falls in, across every list.
fn sections() -> Vec<Section> {
    all_lists()
        .iter()
        .flat_map(|list| {
            list.targets
                .iter()
                .map(|target| {
                    target
                        .section
                        .or(list.section)
                        .expect("parsing refuses a target with no section")
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn every_bundled_list_parses() {
    // The lists are compiled in, so a typo in one is a broken app rather than a broken file.
    // This test is what turns it back into a build-time failure.
    assert_eq!(all_lists().len(), countries().len() + 2);
    assert!(!countries().is_empty());
}

#[test]
fn every_section_has_something_in_it() {
    // A page with an empty heading is a page that looks broken. More to the point, an empty
    // domestic or foreign section would leave the verdict with half its evidence and no way
    // to say so.
    //
    // `Other` is exempt on purpose: it exists for an entry that fits neither bundled group,
    // and until a release adds one, an empty `Other` is the honest state rather than a
    // fixture entry invented to satisfy this test — the page already draws no heading for a
    // group with nothing in it (item 4's rule, reused for item 18's third group).
    let sections = sections();
    for section in Section::ALL {
        if section == Section::Other {
            continue;
        }
        assert!(
            sections.contains(&section),
            "nothing is listed under {section:?}"
        );
    }
}

#[test]
fn no_address_is_measured_twice_across_the_whole_inventory() {
    // The failure the merge exists to end. `discord.com` and `api.steampowered.com` were in
    // `foreign.json` *and* in `services.json`: half of one baseline was a second probe of a
    // row already on the page, in another visual language and under another name, spending
    // the budget twice for one fact.
    //
    // A hard error rather than a deduplication, because which of two entries the reader is
    // meant to see is a question only a person can answer.
    for country in countries() {
        bundled(country)
            .unwrap_or_else(|error| panic!("the inventory for {country} measures twice: {error}"));
    }
}

#[test]
fn a_duplicate_address_is_refused_rather_than_quietly_deduplicated() {
    let list = TargetList::parse(
        "foreign",
        r#"{
            "schemaVersion": 2,
            "id": "foreign",
            "section": "foreign",
            "targets": [
                { "id": "a", "label": "A", "endpoints": [{ "id": "x", "address": "example.net", "port": 443 }] },
                { "id": "b", "label": "B", "endpoints": [{ "id": "y", "address": "example.net", "port": 443 }] }
            ]
        }"#,
    )
    .expect("the file itself is well formed — the clash is between two entries");

    let error = refuse_duplicate_addresses(&[list]).expect_err("one address, two entries");
    assert!(matches!(error, Error::TargetList { .. }));
    assert!(format!("{error}").contains("example.net"));
}

#[test]
fn the_foreign_sample_is_not_two_resolvers_in_a_trenchcoat() {
    // The trap the merge had to avoid. Strip the duplicated entries out naïvely and the
    // foreign baseline collapses to two anycast DNS resolvers, which are famously reachable
    // almost everywhere — a thin and biased sample for the one verdict that decides whether
    // to suggest a VPN. They stayed in the evidence as tagged targets, probed once.
    let list = foreign().expect("the bundled foreign list must validate");
    assert!(
        list.targets.len() >= 4,
        "the foreign section has only {} entries",
        list.targets.len()
    );
    let named_services = list
        .targets
        .iter()
        .filter(|target| !target.label.contains("DNS"))
        .count();
    assert!(
        named_services >= 2,
        "the foreign section is {named_services} services and the rest resolvers"
    );
}

#[test]
fn the_bundled_inventory_stays_within_the_probe_budget() {
    // Every entry costs budget for as long as the app runs, and the slow sections are probed
    // whether or not the user is doing anything. The product's cap is 32 probes a second
    // across everything, and per-application monitoring needs almost all of it.
    for country in countries() {
        let lists = bundled(country).expect("the inventory must validate");
        let mut per_sec = 0.0_f64;
        for list in &lists {
            for target in &list.targets {
                let section = target.section.or(list.section).expect("validated at parse");
                // The fastest the user can set the baseline interval to, so this is the
                // worst case rather than the default.
                let interval = if section.read_by_verdict() {
                    1.0
                } else {
                    SLOW_CHECK_INTERVAL.as_secs_f64()
                };
                per_sec +=
                    f64::from(u32::try_from(target.endpoints.len()).unwrap_or(u32::MAX)) / interval;
            }
        }
        assert!(
            per_sec < 16.0,
            "the {country} inventory costs {per_sec:.1} probes/s at the fastest interval"
        );
    }
}

#[test]
fn every_bundled_endpoint_carries_a_port() {
    // Without one only ICMP can be used, and a front door that drops echoes would then have
    // no fallback at all — which means a permanently red row about a service that is up.
    for list in all_lists() {
        for target in &list.targets {
            for endpoint in &target.endpoints {
                assert!(
                    endpoint.port.is_some(),
                    "{}/{}/{} has no port",
                    list.id,
                    target.id,
                    endpoint.id
                );
            }
        }
    }
}

#[test]
fn bundled_keys_are_unique_across_the_whole_inventory() {
    // The keys are React keys and the identity a row is held still by; two rows sharing one
    // would swap under the reader.
    for country in countries() {
        let mut seen: HashMap<String, &str> = HashMap::new();
        for list in &bundled(country).expect("the inventory must validate") {
            for target in &list.targets {
                let key = format!("{}/{}", list.id, target.id);
                assert!(
                    seen.insert(key.clone(), &list.id).is_none(),
                    "duplicate target key {key}"
                );
            }
        }
    }
}

#[test]
fn an_unknown_country_is_refused_rather_than_silently_empty() {
    assert!(has_country("ru"));
    assert!(!has_country("zz"));
    assert!(matches!(
        domestic("zz"),
        Err(Error::UnknownCountry { country }) if country == "zz"
    ));
}

#[test]
fn rejects_a_newer_schema_instead_of_guessing_at_it() {
    let error = TargetList::parse(
        "foreign",
        r#"{ "schemaVersion": 99, "id": "foreign", "section": "foreign", "targets": [] }"#,
    )
    .expect_err("a schema this build does not understand");
    assert!(format!("{error}").contains("99"));
}

#[test]
fn rejects_a_list_loaded_under_the_wrong_name() {
    let error = TargetList::parse(
        "ru",
        r#"{ "schemaVersion": 2, "id": "ir", "section": "domestic", "targets": [] }"#,
    )
    .expect_err("a file that is not the one that was asked for");
    assert!(format!("{error}").contains("ir"));
}

#[test]
fn rejects_an_empty_list() {
    // A baseline that quietly shrank to nothing would read as good news.
    assert!(TargetList::parse(
        "foreign",
        r#"{ "schemaVersion": 2, "id": "foreign", "section": "foreign", "targets": [] }"#,
    )
    .is_err());
}

#[test]
fn rejects_a_target_with_no_endpoints() {
    assert!(TargetList::parse(
        "foreign",
        r#"{ "schemaVersion": 2, "id": "foreign", "section": "foreign",
             "targets": [{ "id": "a", "label": "A", "endpoints": [] }] }"#,
    )
    .is_err());
}

#[test]
fn rejects_a_target_no_one_gave_a_section() {
    // The section decides which heading a row appears under, how often it is probed and
    // which rule judges it. A default would be a guess about all three.
    let error = TargetList::parse(
        "services",
        r#"{ "schemaVersion": 2, "id": "services",
             "targets": [{ "id": "a", "label": "A",
                           "endpoints": [{ "id": "x", "address": "example.net", "port": 443 }] }] }"#,
    )
    .expect_err("neither the entry nor the file names a section");
    assert!(format!("{error}").contains("section"));
}

#[test]
fn rejects_duplicate_identifiers_at_either_level() {
    assert!(TargetList::parse(
        "foreign",
        r#"{ "schemaVersion": 2, "id": "foreign", "section": "foreign", "targets": [
             { "id": "a", "label": "A", "endpoints": [{ "id": "x", "address": "a.example.net", "port": 443 }] },
             { "id": "a", "label": "B", "endpoints": [{ "id": "x", "address": "b.example.net", "port": 443 }] }] }"#,
    )
    .is_err());

    assert!(TargetList::parse(
        "foreign",
        r#"{ "schemaVersion": 2, "id": "foreign", "section": "foreign", "targets": [
             { "id": "a", "label": "A", "endpoints": [
                { "id": "x", "address": "a.example.net", "port": 443 },
                { "id": "x", "address": "b.example.net", "port": 443 }] }] }"#,
    )
    .is_err());
}

#[test]
fn rejects_an_unknown_field_rather_than_ignoring_it() {
    // A misspelled key is a target that will never be probed, and it must be loud.
    assert!(TargetList::parse(
        "foreign",
        r#"{ "schemaVersion": 2, "id": "foreign", "section": "foreign", "targets": [
             { "id": "a", "label": "A", "colour": "red",
               "endpoints": [{ "id": "x", "address": "a.example.net", "port": 443 }] }] }"#,
    )
    .is_err());
}

#[test]
fn rejects_an_unknown_section_rather_than_shelving_it_somewhere() {
    assert!(TargetList::parse(
        "services",
        r#"{ "schemaVersion": 2, "id": "services", "targets": [
             { "id": "a", "label": "A", "section": "somewhereElse",
               "endpoints": [{ "id": "x", "address": "a.example.net", "port": 443 }] }] }"#,
    )
    .is_err());
}

#[test]
fn rejects_malformed_json_naming_the_list() {
    let error = TargetList::parse("ru", "{ not json").expect_err("malformed");
    assert!(matches!(&error, Error::TargetList { list, .. } if list == "ru"));
}

#[test]
fn a_probe_kind_hint_is_optional_and_parses_by_name() {
    let list = TargetList::parse(
        "services",
        r#"{ "schemaVersion": 2, "id": "services", "targets": [
             { "id": "a", "label": "A", "section": "infrastructure", "probeKind": "tlsHello",
               "endpoints": [{ "id": "x", "address": "a.example.net", "port": 443 }] },
             { "id": "b", "label": "B", "section": "infrastructure",
               "endpoints": [{ "id": "y", "address": "b.example.net", "port": 443 }] }] }"#,
    )
    .expect("both forms are valid");

    assert_eq!(list.targets[0].probe_kind, Some(ProbeKindHint::TlsHello));
    assert_eq!(list.targets[1].probe_kind, None);
}

#[test]
fn rejects_a_probe_kind_the_engine_has_no_prober_for() {
    assert!(TargetList::parse(
        "services",
        r#"{ "schemaVersion": 2, "id": "services", "targets": [
             { "id": "a", "label": "A", "section": "infrastructure", "probeKind": "smokeSignal",
               "endpoints": [{ "id": "x", "address": "a.example.net", "port": 443 }] }] }"#,
    )
    .is_err());
}

#[test]
fn the_sections_a_verdict_reads_are_the_first_two() {
    // The order the page shows them in, and the pair whose comparison *is* the diagnosis.
    assert_eq!(
        Section::ALL,
        [
            Section::Domestic,
            Section::Foreign,
            Section::GamingPlatform,
            Section::Infrastructure,
            Section::Other,
        ]
    );
    assert!(Section::Domestic.read_by_verdict());
    assert!(Section::Foreign.read_by_verdict());
    assert!(!Section::GamingPlatform.read_by_verdict());
    assert!(!Section::Infrastructure.read_by_verdict());
    assert!(!Section::Other.read_by_verdict());
}

#[test]
fn a_verdict_bearing_section_is_judged_by_a_window_and_the_rest_by_their_last_checks() {
    // The measurement layer is the one thing that did *not* merge. A baseline asks what the
    // last several minutes have been like; a platform asks whether it is reachable now, and
    // at a check every forty-odd seconds a window answers that badly at both ends.
    assert!(Section::Domestic.judged_by_window());
    assert!(Section::Foreign.judged_by_window());
    assert!(!Section::GamingPlatform.judged_by_window());
    assert!(!Section::Infrastructure.judged_by_window());
    assert!(!Section::Other.judged_by_window());
}

#[test]
fn only_the_verdicts_own_evidence_is_uneditable() {
    // `Domestic` and `Foreign` are not services and are not the user's to remove; the other
    // three are the bundled catalogue an edit chooser may offer.
    assert!(!Section::Domestic.editable());
    assert!(!Section::Foreign.editable());
    assert!(Section::GamingPlatform.editable());
    assert!(Section::Infrastructure.editable());
    assert!(Section::Other.editable());
}

#[test]
fn the_catalogue_offers_no_verdict_evidence() {
    // The chooser is over the bundled services list only. A stray `domestic` or `foreign`
    // entry here would let a user untick evidence the verdict is still reading, silently
    // breaking item 20's rule that editing changes what is shown, never what is measured.
    let catalogue = nm_app::targets::catalogue().expect("the bundled catalogue must parse");
    assert!(!catalogue.is_empty());
    for entry in &catalogue {
        assert!(
            entry.section.editable(),
            "{:?} is not editable and must not be offered",
            entry.section
        );
    }
}

/// A resolved target with no endpoints, which is all `is_selected` ever looks at.
fn resolved(key: &str, section: Section) -> ResolvedTarget {
    ResolvedTarget {
        key: key.to_owned(),
        label: key.to_owned(),
        section,
        probe_kind: None,
        endpoints: Vec::new(),
    }
}

#[test]
fn a_verdict_bearing_target_is_always_selected() {
    // Item 20's rule: editing changes what is shown, never what is measured for the
    // verdict. An empty selection would otherwise silently thin the domestic and foreign
    // evidence the diagnosis reads.
    let domestic = resolved("ru/yandex-dns", Section::Domestic);
    let foreign = resolved("foreign/discord", Section::Foreign);
    assert!(is_selected(&domestic, &None));
    assert!(is_selected(&domestic, &Some(Vec::new())));
    assert!(is_selected(&foreign, &Some(Vec::new())));
}

#[test]
fn no_selection_at_all_measures_the_whole_catalogue() {
    let target = resolved("services/ea", Section::GamingPlatform);
    assert!(is_selected(&target, &None));
}

#[test]
fn an_editable_target_is_measured_only_when_its_key_is_selected() {
    let target = resolved("services/ea", Section::GamingPlatform);
    assert!(is_selected(
        &target,
        &Some(vec!["services/ea".to_owned(), "services/aws".to_owned()])
    ));
    assert!(!is_selected(
        &target,
        &Some(vec!["services/aws".to_owned()])
    ));
    assert!(!is_selected(&target, &Some(Vec::new())));
}

#[test]
fn every_catalogue_key_matches_a_resolved_target() {
    // The selection is stored by key, and it must line up with the key the runtime actually
    // registers, or a stored selection would silently select nothing.
    let list = services().expect("the bundled service list must validate");
    let catalogue = nm_app::targets::catalogue().expect("the bundled catalogue must parse");
    let expected: Vec<String> = list
        .targets
        .iter()
        .map(|target| format!("services/{}", target.id))
        .collect();
    let keys: Vec<String> = catalogue.into_iter().map(|entry| entry.key).collect();
    assert_eq!(keys, expected);
}
