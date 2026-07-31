//! The bundled known-application presets, and what a malformed one must do.
//!
//! In `tests/` rather than in the module because `nm-app`'s library sets `test = false` —
//! an in-crate harness cannot start on Windows (see `tests.manifest`).

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use nm_app::presets::PresetList;

/// Executables that belong to more than one application and must never be listed.
///
/// A shared launcher or anti-cheat in a preset would silently merge two applications the
/// user chose separately, and the merged endpoint list is wrong in a way they cannot see.
/// The rule is written down in `assets/apps/README.md`; this is what enforces it.
const SHARED: &[&str] = &[
    "steam.exe",
    "EpicGamesLauncher.exe",
    "RiotClientServices.exe",
    "EADesktop.exe",
    "Battle.net.exe",
    "EasyAntiCheat.exe",
    "EasyAntiCheat_EOS.exe",
    "BEService.exe",
    "vgtray.exe",
    "explorer.exe",
    "svchost.exe",
];

#[test]
fn the_bundled_presets_are_valid() {
    let list = PresetList::bundled().expect("the bundled presets must load");
    assert!(!list.applications.is_empty());
}

#[test]
fn the_bundled_presets_cover_the_titles_the_plan_names() {
    let list = PresetList::bundled().unwrap();
    for id in [
        "discord",
        "dota2",
        "cs2",
        "apex-legends",
        "valorant",
        "fortnite",
    ] {
        assert!(
            list.applications.iter().any(|preset| preset.id == id),
            "no preset for {id}"
        );
    }
}

#[test]
fn no_preset_claims_an_executable_several_applications_share() {
    let list = PresetList::bundled().unwrap();
    for shared in SHARED {
        assert!(
            list.matching(shared).is_none(),
            "{shared} is shared between applications and must not be in a preset"
        );
    }
}

#[test]
fn a_preset_matches_however_its_name_is_cased() {
    let list = PresetList::bundled().unwrap();
    let discord = list.matching("discord.exe").expect("Discord is bundled");
    assert_eq!(discord.label, "Discord");
    assert_eq!(
        list.matching("DISCORD.EXE").map(|preset| &preset.id),
        Some(&discord.id)
    );
}

#[test]
fn an_unknown_executable_matches_nothing() {
    let list = PresetList::bundled().unwrap();
    assert!(list.matching("notepad.exe").is_none());
    assert!(list.matching("").is_none());
}

#[test]
fn an_executable_claimed_twice_is_refused() {
    // The failure this validation exists for: with the same name in two presets, which
    // application a process joins would depend on file order — that is, on nothing the
    // user can see.
    let json = r#"{
        "schemaVersion": 1,
        "applications": [
            { "id": "one", "label": "One", "executables": ["shared.exe"] },
            { "id": "two", "label": "Two", "executables": ["own.exe", "SHARED.exe"] }
        ]
    }"#;
    let error = PresetList::parse(json).unwrap_err().to_string();
    assert!(error.contains("SHARED.exe"), "{error}");
}

#[test]
fn a_duplicate_identifier_is_refused() {
    let json = r#"{
        "schemaVersion": 1,
        "applications": [
            { "id": "one", "label": "One", "executables": ["a.exe"] },
            { "id": "one", "label": "Other", "executables": ["b.exe"] }
        ]
    }"#;
    assert!(PresetList::parse(json)
        .unwrap_err()
        .to_string()
        .contains("duplicate preset id"));
}

#[test]
fn a_preset_with_no_executables_is_refused() {
    let json = r#"{
        "schemaVersion": 1,
        "applications": [{ "id": "one", "label": "One", "executables": [] }]
    }"#;
    assert!(PresetList::parse(json)
        .unwrap_err()
        .to_string()
        .contains("lists no executables"));

    let empty_name = r#"{
        "schemaVersion": 1,
        "applications": [{ "id": "one", "label": "One", "executables": [""] }]
    }"#;
    assert!(PresetList::parse(empty_name)
        .unwrap_err()
        .to_string()
        .contains("empty executable name"));
}

#[test]
fn an_unsupported_schema_version_is_refused() {
    let json = r#"{ "schemaVersion": 99, "applications": [] }"#;
    assert!(PresetList::parse(json)
        .unwrap_err()
        .to_string()
        .contains("schema version 99"));
}

#[test]
fn a_misspelled_key_is_loud_rather_than_ignored() {
    // A key nothing reads is an executable that will never be grouped, and a grouping that
    // silently fails to happen looks exactly like an application with missing endpoints.
    let json = r#"{
        "schemaVersion": 1,
        "applications": [
            { "id": "one", "label": "One", "executable": ["a.exe"], "executables": ["a.exe"] }
        ]
    }"#;
    assert!(PresetList::parse(json).is_err());
}

#[test]
fn an_empty_list_groups_nothing_and_is_not_an_error() {
    // What the app falls back to if the bundled file were ever unusable: the name and tree
    // rules still work, so only the awkward titles are lost.
    let empty = PresetList::empty();
    assert!(empty.matching("Discord.exe").is_none());
    assert!(empty.applications.is_empty());
}
