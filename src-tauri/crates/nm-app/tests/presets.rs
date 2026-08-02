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
    assert!(empty.label_of("Discord.exe").is_none());
}

// --------------------------------------------- labelling, which joins nothing

#[test]
fn a_shared_launcher_may_be_named_even_though_it_may_never_be_grouped() {
    // The whole point of separating the two jobs. Listing `steam.exe` in a *grouping* entry
    // would silently merge two games the user chose separately; naming it "Steam" joins
    // nothing to it, so the picker can stop reading like a task manager.
    let list = PresetList::bundled().unwrap();
    for shared in SHARED {
        assert!(
            list.matching(shared).is_none(),
            "{shared} is shared between applications and must not be grouped"
        );
    }
    assert_eq!(list.label_of("steam.exe"), Some("Steam"));
    assert!(
        list.matching("steam.exe").is_none(),
        "naming it must not have grouped it"
    );
}

#[test]
fn the_titles_the_verification_pass_names_are_labelled() {
    // Every one of these was read off a real installation on the machine this was built on;
    // a recalled name is a label that never fires and fails silently.
    let list = PresetList::bundled().unwrap();
    for (executable, expected) in [
        ("cs2.exe", "Counter-Strike 2"),
        ("dota2.exe", "Dota 2"),
        ("deadlock.exe", "Deadlock"),
        ("WorldOfTanks.exe", "World of Tanks"),
        ("ForzaHorizon4.exe", "Forza Horizon 4"),
    ] {
        assert_eq!(list.label_of(executable), Some(expected));
    }
}

#[test]
fn a_label_matches_however_its_name_is_cased() {
    let list = PresetList::bundled().unwrap();
    assert_eq!(list.label_of("STEAM.EXE"), Some("Steam"));
    assert_eq!(list.label_of("notepad.exe"), None);
}

#[test]
fn an_executable_that_is_grouped_may_not_also_be_labelled() {
    // Two sources for one name would make the answer depend on which list was consulted
    // first, which is to say on nothing the reader of the file can see.
    let json = r#"{
        "schemaVersion": 1,
        "applications": [{ "id": "one", "label": "One", "executables": ["game.exe"] }],
        "labels": [{ "id": "two", "label": "Two", "executable": "GAME.exe" }]
    }"#;
    let error = PresetList::parse(json).unwrap_err().to_string();
    assert!(
        error.contains("already named by the grouping entry"),
        "{error}"
    );
}

#[test]
fn an_executable_labelled_twice_is_refused() {
    let json = r#"{
        "schemaVersion": 1,
        "applications": [],
        "labels": [
            { "id": "one", "label": "One", "executable": "a.exe" },
            { "id": "two", "label": "Two", "executable": "A.EXE" }
        ]
    }"#;
    assert!(PresetList::parse(json)
        .unwrap_err()
        .to_string()
        .contains("labelled twice"));
}

#[test]
fn an_identifier_is_unique_across_both_lists() {
    let json = r#"{
        "schemaVersion": 1,
        "applications": [{ "id": "one", "label": "One", "executables": ["a.exe"] }],
        "labels": [{ "id": "one", "label": "Other", "executable": "b.exe" }]
    }"#;
    assert!(PresetList::parse(json)
        .unwrap_err()
        .to_string()
        .contains("duplicate id"));
}

// -------------------------------------- the generated catalogue of titles

#[test]
fn the_generated_catalogue_names_titles_the_curated_list_never_will() {
    // Nine thousand names, generated by `scripts/build-app-labels.mjs` from Discord's public
    // game-detection index and committed. It is why the picker stops reading like a task
    // manager for a library nobody could hand-verify.
    let list = PresetList::bundled().unwrap();
    assert_eq!(list.label_of("ForzaHorizon5.exe"), Some("Forza Horizon 5"));
    assert_eq!(list.label_of("Cyberpunk2077.exe"), Some("Cyberpunk 2077"));
}

#[test]
fn a_curated_name_always_beats_the_catalogue() {
    // The order is what makes a wrong generated name a one-line fix rather than an argument
    // with a table of nine thousand entries.
    let list = PresetList::bundled().unwrap();
    assert_eq!(list.label_of("cs2.exe"), Some("Counter-Strike 2"));
    assert_eq!(list.label_of("steam.exe"), Some("Steam"));
}

#[test]
fn the_catalogue_claims_no_short_or_generic_name() {
    // The two ways a file name stops identifying a program. `at.exe` is claimed by a tycoon
    // game from 2003 in the index and has shipped with Windows since NT; a confident wrong
    // name is worse than a file name, so the whole class is dropped at generation.
    let list = PresetList::bundled().unwrap();
    for risky in [
        "at.exe",
        "ai.exe",
        "game.exe",
        "launcher.exe",
        "client.exe",
        "java.exe",
        "python.exe",
        "cmd.exe",
        "explorer.exe",
    ] {
        assert_eq!(list.label_of(risky), None, "{risky} must stay unnamed");
    }
}

#[test]
fn a_hand_written_list_carries_no_catalogue() {
    // What keeps these tests readable: a list parsed from a string in a test says exactly
    // what the test wrote and nothing else.
    let json = r#"{ "schemaVersion": 1, "applications": [] }"#;
    assert_eq!(PresetList::parse(json).unwrap().label_of("dayz.exe"), None);
    assert_eq!(
        PresetList::bundled().unwrap().label_of("dayz.exe"),
        Some("DayZ")
    );
}

#[test]
fn a_file_without_labels_still_loads() {
    // An older file is a missing name, which is cosmetic; refusing it would cost every
    // grouping in it, which is not.
    let json = r#"{
        "schemaVersion": 1,
        "applications": [{ "id": "one", "label": "One", "executables": ["a.exe"] }]
    }"#;
    let list = PresetList::parse(json).unwrap();
    assert!(list.labels.is_empty());
    assert_eq!(list.label_of("a.exe"), Some("One"));
}
