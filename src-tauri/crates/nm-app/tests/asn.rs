//! Tests for the bundled autonomous-system directory: that it loads, and what it claims.
//!
//! Every address here is either a documented constant this project already probes as a
//! target (`1.1.1.1`, `8.8.8.8`, `9.9.9.9` and Cloudflare's published IPv6 resolver) or
//! reserved documentation space. Nothing was observed on anyone's machine, which is the rule
//! `CLAUDE.md` sets for anything committed.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::net::IpAddr;
use std::sync::OnceLock;

use nm_app::asn::{self, NetworkNames, SNAPSHOT_DATE};

/// Loads the real bundle once for every test that needs it.
///
/// It is a few hundred milliseconds of decompression and parsing, and paying that per test
/// would make this the slowest file in the suite for no added confidence.
fn bundled() -> &'static NetworkNames {
    static LOADED: OnceLock<NetworkNames> = OnceLock::new();
    LOADED.get_or_init(|| NetworkNames::of(asn::load().expect("the bundled directory must load")))
}

fn ip(raw: &str) -> IpAddr {
    raw.parse().expect("a test address must parse")
}

#[test]
fn the_bundled_directory_loads() {
    // The one test that would catch a botched regeneration of the assets, a file truncated
    // in the repository, or a change to the generating command that silently altered the
    // format the parser expects.
    let names = bundled();
    assert!(names.is_loaded());
    assert!(
        names.range_count() > 400_000,
        "only {} blocks loaded — the bundle looks truncated",
        names.range_count()
    );
}

#[test]
fn it_names_the_operator_of_an_address_that_operator_publishes() {
    let found = bundled()
        .name_of(ip("1.1.1.1"))
        .expect("a public resolver's network must be in the directory");
    assert_eq!(found.asn, 13335);
    let name = found.name.expect("that network is registered under a name");
    assert!(
        name.to_ascii_uppercase().contains("CLOUDFLARE"),
        "expected the operator's name, got {name:?}"
    );
}

#[test]
fn it_names_the_other_bundled_resolvers_too() {
    // One lookup could be luck. These confirm the table is aligned across its whole range
    // rather than happening to be right at one address — the failure mode of an off-by-one
    // in a sorted search is a neighbour's name, which nobody could spot by eye.
    for (address, expected) in [("8.8.8.8", 15169_u32), ("9.9.9.9", 19281)] {
        let found = bundled()
            .name_of(ip(address))
            .expect("a public resolver's network must be in the directory");
        assert_eq!(found.asn, expected, "{address}");
        assert!(found.name.is_some(), "{address}");
    }
}

#[test]
fn an_ipv6_address_is_named_as_readily_as_an_ipv4_one() {
    let found = bundled()
        .name_of(ip("2606:4700:4700::1111"))
        .expect("the IPv6 half of the table must be loaded and searchable");
    assert_eq!(found.asn, 13335);
}

#[test]
fn a_registration_country_is_two_uppercase_letters_or_absent() {
    let found = bundled().name_of(ip("1.1.1.1")).expect("must be named");
    if let Some(country) = found.country {
        assert_eq!(country.len(), 2, "{country:?}");
        assert!(
            country.chars().all(|c| c.is_ascii_uppercase()),
            "{country:?}"
        );
    }
}

#[test]
fn addresses_that_belong_to_no_one_are_left_unnamed() {
    // A private address, this machine, and documentation space. Naming any of these would
    // be inventing a fact, so the page shows nothing instead.
    for raw in ["192.168.1.1", "127.0.0.1", "10.0.0.1", "192.0.2.1"] {
        assert!(bundled().name_of(ip(raw)).is_none(), "{raw}");
    }
}

#[test]
fn an_unloaded_directory_names_nothing_and_says_so() {
    // The state the view layer holds before the load lands and after the user switches the
    // feature off. It must answer immediately — not stall, and not guess.
    let names = NetworkNames::none();
    assert!(!names.is_loaded());
    assert_eq!(names.range_count(), 0);
    assert_eq!(names.approximate_heap_bytes(), 0);
    assert!(names.name_of(ip("1.1.1.1")).is_none());
}

#[test]
fn the_loaded_directory_stays_inside_its_share_of_the_memory_budget() {
    // The core's budget is 50 MB in total, and this feature is one label on a row. If it
    // ever grew past a quarter of that, bundling would need rethinking rather than quietly
    // spending the headroom the budget exists to protect.
    let bytes = bundled().approximate_heap_bytes();
    assert!(
        bytes < 16 * 1024 * 1024,
        "the directory holds {bytes} bytes, past its share of the core budget"
    );
}

#[test]
fn the_stated_snapshot_date_matches_the_one_recorded_beside_the_assets() {
    // The date is shown to the user to explain a name that may have gone stale. Two copies
    // of it drifting apart would turn that explanation into a second wrong claim.
    let readme = include_str!("../../../../assets/asn/README.md");
    assert!(
        readme.contains(SNAPSHOT_DATE),
        "README.md does not mention {SNAPSHOT_DATE}"
    );
}
