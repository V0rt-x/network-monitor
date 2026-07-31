//! Grouping processes into the application the user actually chose.
//!
//! In `tests/` rather than in the module because `nm-app`'s library sets `test = false` —
//! an in-crate harness cannot start on Windows (see `tests.manifest`).
//!
//! The whole rule is driven from a process snapshot passed in, so everything here runs on
//! any operating system without a process ever being enumerated. The names are invented
//! and the identifiers are arbitrary; nothing observed on a real machine appears here.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use nm_app::applications::{candidates, Applications, MAX_PROCESSES_PER_APP};
use nm_app::presets::PresetList;
use nm_core::endpoint::{AppId, MAX_MONITORED_APPS};
use nm_platform::process::{Pid, ProcessInfo};

fn process(pid: u32, name: &str, parent: Option<u32>) -> ProcessInfo {
    ProcessInfo {
        pid: Pid::new(pid),
        name: name.to_owned(),
        parent: parent.map(Pid::new),
    }
}

/// A registry that groups by executable name and process tree alone.
fn plain() -> Applications {
    Applications::new(PresetList::empty())
}

/// A registry that also knows one two-executable application.
fn with_preset() -> Applications {
    let json = r#"{
        "schemaVersion": 1,
        "applications": [
            {
                "id": "example-game",
                "label": "Example Game",
                "executables": ["game.exe", "game-helper.exe"]
            }
        ]
    }"#;
    Applications::new(PresetList::parse(json).unwrap())
}

fn members(apps: &Applications, id: AppId) -> Vec<u32> {
    let app = apps
        .iter()
        .find(|app| app.id() == id)
        .expect("the application is monitored");
    let mut pids: Vec<u32> = app
        .members()
        .iter()
        .map(|member| member.pid.get())
        .collect();
    pids.sort_unstable();
    pids
}

// ------------------------------------------------------------- the three rules

#[test]
fn the_chosen_process_is_always_a_member() {
    let mut apps = plain();
    let snapshot = vec![process(100, "solitaire.exe", Some(4))];

    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();

    assert_eq!(members(&apps, id), vec![100]);
    assert_eq!(apps.app_of(Pid::new(100)), Some(id));
}

#[test]
fn every_process_running_the_same_executable_joins() {
    // An Electron application's helpers really are the same program, and the user asked to
    // watch the application rather than whichever of them opened the socket.
    let mut apps = plain();
    let snapshot = vec![
        process(100, "Chat.exe", Some(4)),
        process(101, "Chat.exe", Some(100)),
        process(102, "Chat.exe", Some(100)),
        process(200, "unrelated.exe", Some(4)),
    ];

    let id = apps.adopt(Pid::new(101), &snapshot).unwrap();

    assert_eq!(members(&apps, id), vec![100, 101, 102]);
    assert_eq!(apps.app_of(Pid::new(200)), None);
}

#[test]
fn a_launcher_adopts_the_title_it_starts() {
    // The one relation that catches launcher → game, and the reason a user can arm the
    // monitor before a match instead of scrambling for the picker once it has begun.
    let mut apps = plain();
    let mut snapshot = vec![process(100, "launcher.exe", Some(4))];
    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();
    assert_eq!(members(&apps, id), vec![100]);

    // The match starts.
    snapshot.push(process(300, "title.exe", Some(100)));
    snapshot.push(process(301, "anticheat.exe", Some(300)));
    apps.refresh(&snapshot);

    assert_eq!(
        members(&apps, id),
        vec![100, 300, 301],
        "a descendant of a member is a member, however deep"
    );
}

#[test]
fn an_ancestor_is_not_a_member() {
    // Descendants only. Adopting upwards would put the shell that started the game — and
    // therefore everything else it started — into the application.
    let mut apps = plain();
    let snapshot = vec![
        process(4, "shell.exe", None),
        process(100, "title.exe", Some(4)),
        process(101, "browser.exe", Some(4)),
    ];

    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();

    assert_eq!(members(&apps, id), vec![100]);
}

#[test]
fn a_preset_joins_executables_nothing_else_would() {
    // Neither the name rule nor the tree rule can see this: two different executables,
    // neither the parent of the other.
    let mut apps = with_preset();
    let snapshot = vec![
        process(100, "game.exe", Some(4)),
        process(200, "game-helper.exe", Some(4)),
        process(300, "other.exe", Some(4)),
    ];

    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();

    assert_eq!(members(&apps, id), vec![100, 200]);
    assert_eq!(
        apps.iter().next().unwrap().label(),
        "Example Game",
        "a preset names the application, not the executable that was clicked"
    );
    assert_eq!(apps.iter().next().unwrap().preset(), Some("example-game"));
}

#[test]
fn without_a_preset_an_application_is_named_after_its_executable() {
    let mut apps = plain();
    let snapshot = vec![process(100, "Chat.exe", Some(4))];
    apps.adopt(Pid::new(100), &snapshot).unwrap();

    let app = apps.iter().next().unwrap();
    assert_eq!(app.label(), "Chat.exe");
    assert_eq!(app.preset(), None);
}

#[test]
fn either_half_of_a_preset_forms_the_same_application() {
    let mut apps = with_preset();
    let snapshot = vec![
        process(100, "game.exe", Some(4)),
        process(200, "game-helper.exe", Some(4)),
    ];

    let id = apps.adopt(Pid::new(200), &snapshot).unwrap();

    assert_eq!(members(&apps, id), vec![100, 200]);
    assert_eq!(apps.iter().next().unwrap().label(), "Example Game");
}

#[test]
fn executable_names_are_matched_case_insensitively() {
    // Windows compares file names that way, and a game that re-launches itself with a
    // differently-cased argv[0] must not become a second application.
    let mut apps = plain();
    let snapshot = vec![
        process(100, "Game.exe", Some(4)),
        process(101, "GAME.EXE", Some(4)),
    ];

    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();

    assert_eq!(members(&apps, id), vec![100, 101]);
}

// ------------------------------------------------------------- membership is live

#[test]
fn a_process_that_exits_leaves_and_its_siblings_carry_on() {
    let mut apps = plain();
    let snapshot = vec![
        process(100, "Chat.exe", Some(4)),
        process(101, "Chat.exe", Some(100)),
    ];
    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();

    apps.refresh(&[process(101, "Chat.exe", Some(100))]);

    assert_eq!(members(&apps, id), vec![101]);
    assert_eq!(apps.app_of(Pid::new(100)), None);
    assert_eq!(apps.len(), 1, "losing a process is not losing the choice");
}

#[test]
fn a_member_survives_the_death_of_the_parent_it_was_adopted_through() {
    // The anti-cheat case: the shim starts the game and exits. The game is no longer any
    // member's descendant and does not share the chosen executable's name, and it is still
    // the application the user is watching.
    let mut apps = plain();
    let snapshot = vec![
        process(100, "shim.exe", Some(4)),
        process(300, "title.exe", Some(100)),
    ];
    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();
    assert_eq!(members(&apps, id), vec![100, 300]);

    apps.refresh(&[process(300, "title.exe", Some(100))]);

    assert_eq!(members(&apps, id), vec![300]);
}

#[test]
fn a_recycled_identifier_running_something_else_is_dropped() {
    // Windows reissues process identifiers. A member is kept only while the pid still
    // names the program that was adopted under it.
    let mut apps = plain();
    let snapshot = vec![
        process(100, "Chat.exe", Some(4)),
        process(101, "Chat.exe", Some(100)),
    ];
    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();

    apps.refresh(&[
        process(100, "Chat.exe", Some(4)),
        process(101, "notepad.exe", Some(4)),
    ]);

    assert_eq!(members(&apps, id), vec![100]);
}

#[test]
fn an_application_with_nothing_running_is_kept_and_catches_the_relaunch() {
    // Arming the monitor before a match: the choice outlives every process it was made
    // about, and the game is picked up when it appears.
    let mut apps = plain();
    let id = apps
        .adopt(Pid::new(100), &[process(100, "title.exe", Some(4))])
        .unwrap();

    apps.refresh(&[]);
    assert_eq!(members(&apps, id), Vec::<u32>::new());
    assert_eq!(apps.len(), 1);
    assert!(apps.watched_pids().is_empty());

    // The game is started again, with a new identifier.
    apps.refresh(&[process(999, "title.exe", Some(4))]);
    assert_eq!(members(&apps, id), vec![999]);
}

// ------------------------------------------------------------- conflicts and caps

#[test]
fn a_process_belongs_to_one_application_only() {
    let mut apps = plain();
    let snapshot = vec![
        process(100, "launcher.exe", Some(4)),
        process(300, "title.exe", Some(100)),
    ];
    let launcher = apps.adopt(Pid::new(100), &snapshot).unwrap();

    // The title is already the launcher's, so choosing it separately is refused rather
    // than measured twice under two identities.
    assert_eq!(apps.adopt(Pid::new(300), &snapshot), None);
    assert_eq!(members(&apps, launcher), vec![100, 300]);
    assert_eq!(apps.len(), 1);
}

#[test]
fn the_earlier_choice_keeps_a_contested_process() {
    // Two applications whose rules both reach one process: the tie breaks on the order the
    // user chose them, which is the only order they can see.
    let mut apps = plain();
    let snapshot = vec![
        process(100, "one.exe", Some(4)),
        process(200, "two.exe", Some(4)),
        process(300, "child.exe", Some(100)),
    ];
    let first = apps.adopt(Pid::new(100), &snapshot).unwrap();
    let second = apps.adopt(Pid::new(200), &snapshot).unwrap();

    // The contested child moves under the second application's tree.
    apps.refresh(&[
        process(100, "one.exe", Some(4)),
        process(200, "two.exe", Some(4)),
        process(300, "child.exe", Some(200)),
    ]);

    assert_eq!(members(&apps, first), vec![100, 300]);
    assert_eq!(members(&apps, second), vec![200]);
}

#[test]
fn a_process_that_is_not_running_cannot_be_chosen() {
    let mut apps = plain();
    assert_eq!(apps.adopt(Pid::new(100), &[]), None);
    assert!(apps.is_empty());
}

#[test]
fn the_five_application_cap_is_a_refusal_rather_than_an_eviction() {
    let mut apps = plain();
    let snapshot: Vec<ProcessInfo> = (0..=MAX_MONITORED_APPS)
        .map(|index| process(100 + index, &format!("app{index}.exe"), Some(4)))
        .collect();

    for index in 0..MAX_MONITORED_APPS {
        assert!(apps.adopt(Pid::new(100 + index), &snapshot).is_some());
    }
    assert_eq!(
        apps.adopt(Pid::new(100 + MAX_MONITORED_APPS), &snapshot),
        None
    );
    assert_eq!(apps.len(), usize::try_from(MAX_MONITORED_APPS).unwrap());
    assert!(
        apps.iter().all(|app| app.members().len() == 1),
        "refusing the sixth must not have disturbed the five"
    );
}

#[test]
fn membership_stops_at_the_per_application_cap() {
    // A user who picks a process with hundreds of descendants must not turn one click into
    // a set the size of the machine, tested against on every flow event.
    let mut apps = plain();
    let mut snapshot = vec![process(100, "root.exe", Some(4))];
    for index in 0..200 {
        snapshot.push(process(1_000 + index, "child.exe", Some(100)));
    }

    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();

    assert_eq!(members(&apps, id).len(), MAX_PROCESSES_PER_APP);
    assert!(
        apps.app_of(Pid::new(100)).is_some(),
        "the chosen process is taken first, so the cap can never evict it"
    );
}

#[test]
fn a_loop_in_the_parent_links_terminates() {
    // Reissued identifiers can make a snapshot's parent links circular. A walk that trusted
    // them would never finish.
    let mut apps = plain();
    let snapshot = vec![
        process(100, "a.exe", Some(102)),
        process(101, "b.exe", Some(100)),
        process(102, "c.exe", Some(101)),
    ];

    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();

    assert_eq!(members(&apps, id), vec![100, 101, 102]);
}

#[test]
fn a_process_that_is_its_own_parent_is_not_its_own_child() {
    let mut apps = plain();
    let snapshot = vec![process(100, "a.exe", Some(100))];
    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();
    assert_eq!(members(&apps, id), vec![100]);
}

// ------------------------------------------------------------- what the rest of the app asks

#[test]
fn forgetting_an_application_releases_its_processes() {
    let mut apps = plain();
    let snapshot = vec![
        process(100, "Chat.exe", Some(4)),
        process(101, "Chat.exe", Some(100)),
    ];
    let id = apps.adopt(Pid::new(100), &snapshot).unwrap();
    assert_eq!(apps.watched_pids(), vec![Pid::new(100), Pid::new(101)]);

    assert!(apps.forget(id));

    assert!(apps.is_empty());
    assert!(apps.watched_pids().is_empty());
    assert_eq!(apps.app_of(Pid::new(100)), None);
    assert!(!apps.forget(id), "forgetting twice is quiet");
}

#[test]
fn identities_are_never_zero_and_never_reused() {
    // A raw identifier crosses the IPC boundary, so a default-constructed zero arriving
    // back from the UI must not name anything; and an identifier freed by one application
    // must not silently start naming the next.
    let mut apps = plain();
    let snapshot = vec![
        process(100, "one.exe", Some(4)),
        process(200, "two.exe", Some(4)),
    ];

    let first = apps.adopt(Pid::new(100), &snapshot).unwrap();
    assert_ne!(first, AppId::new(0));
    apps.forget(first);
    let second = apps.adopt(Pid::new(200), &snapshot).unwrap();

    assert_ne!(first, second);
}

// ------------------------------------------------------------- what the picker offers

#[test]
fn the_picker_offers_applications_rather_than_processes() {
    // The failure this exists to fix: six identical Discord.exe rows, and the user asked to
    // pick one arbitrarily when what they want is Discord.
    let snapshot = vec![
        process(100, "Chat.exe", Some(4)),
        process(101, "Chat.exe", Some(100)),
        process(102, "Chat.exe", Some(100)),
        process(200, "game.exe", Some(4)),
    ];

    let offers = candidates(&PresetList::empty(), &snapshot);

    let labels: Vec<&str> = offers.iter().map(|offer| offer.label.as_str()).collect();
    assert_eq!(labels, vec!["Chat.exe", "game.exe"]);
    assert_eq!(offers[0].processes.len(), 3);
}

#[test]
fn a_preset_is_offered_under_its_own_name_once() {
    let presets = PresetList::parse(
        r#"{
            "schemaVersion": 1,
            "applications": [
                { "id": "example-game", "label": "Example Game",
                  "executables": ["game.exe", "game-helper.exe"] }
            ]
        }"#,
    )
    .unwrap();
    let snapshot = vec![
        process(100, "game.exe", Some(4)),
        process(200, "game-helper.exe", Some(4)),
    ];

    let offers = candidates(&presets, &snapshot);

    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].label, "Example Game");
    assert_eq!(offers[0].processes, vec![Pid::new(100), Pid::new(200)]);
}

#[test]
fn an_offer_is_seeded_from_the_process_whose_parent_is_not_one_of_its_own() {
    // The main process of a helpered application: the one whose children are worth
    // adopting when the user commits to watching it.
    let snapshot = vec![
        process(300, "Chat.exe", Some(101)),
        process(101, "Chat.exe", Some(4)),
        process(302, "Chat.exe", Some(101)),
    ];

    let offers = candidates(&PresetList::empty(), &snapshot);

    assert_eq!(offers[0].seed, Pid::new(101));
}

#[test]
fn the_picker_does_not_follow_the_process_tree() {
    // The tree rule catches a launcher starting a title *after* the user has committed to
    // watching it. Applied to an unfiltered process list it would collapse the machine into
    // whatever the shell started.
    let snapshot = vec![
        process(4, "shell.exe", None),
        process(100, "one.exe", Some(4)),
        process(200, "two.exe", Some(100)),
    ];

    let offers = candidates(&PresetList::empty(), &snapshot);

    assert_eq!(offers.len(), 3, "three names, three offers");
    for offer in &offers {
        assert_eq!(offer.processes.len(), 1);
    }
}

#[test]
fn offers_are_named_case_insensitively_and_ordered_the_same_every_time() {
    let snapshot = vec![
        process(100, "Zeta.exe", Some(4)),
        process(101, "alpha.exe", Some(4)),
        process(102, "ALPHA.EXE", Some(4)),
    ];

    let offers = candidates(&PresetList::empty(), &snapshot);

    assert_eq!(offers.len(), 2, "one executable, however it is cased");
    assert_eq!(offers[0].label.to_lowercase(), "alpha.exe");
    assert_eq!(offers[1].label, "Zeta.exe");
    assert_eq!(
        offers
            .iter()
            .map(|offer| offer.key.clone())
            .collect::<Vec<_>>(),
        candidates(&PresetList::empty(), &snapshot)
            .iter()
            .map(|offer| offer.key.clone())
            .collect::<Vec<_>>(),
        "a list the user is clicking in must not reshuffle between refreshes"
    );
}

#[test]
fn an_offer_can_be_monitored_and_the_group_is_the_same_either_way() {
    // What the picker shows and what monitoring adopts have to agree, or the user would
    // choose one thing and watch another.
    let mut apps = plain();
    let snapshot = vec![
        process(100, "Chat.exe", Some(4)),
        process(101, "Chat.exe", Some(100)),
    ];
    let offer = candidates(&PresetList::empty(), &snapshot)
        .into_iter()
        .next()
        .unwrap();

    let id = apps.adopt(offer.seed, &snapshot).unwrap();

    assert_eq!(
        members(&apps, id),
        offer
            .processes
            .iter()
            .map(|pid| pid.get())
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_empty_machine_offers_nothing_and_a_nameless_process_is_skipped() {
    assert!(candidates(&PresetList::empty(), &[]).is_empty());
    assert!(candidates(&PresetList::empty(), &[process(100, "", Some(4))]).is_empty());
}

#[test]
fn the_watched_set_follows_membership() {
    let mut apps = plain();
    apps.adopt(Pid::new(100), &[process(100, "launcher.exe", Some(4))])
        .unwrap();
    assert_eq!(apps.watched_pids(), vec![Pid::new(100)]);

    apps.refresh(&[
        process(100, "launcher.exe", Some(4)),
        process(300, "title.exe", Some(100)),
    ]);

    assert_eq!(apps.watched_pids(), vec![Pid::new(100), Pid::new(300)]);
}
