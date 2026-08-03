//! Tests for settings: defaults, sanitizing, and the round trip through a file.

// `clippy.toml` already allows panicking APIs inside `#[test]` functions — a failed
// assertion is the intended outcome — but not inside the helpers those tests share.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::PathBuf;

use nm_app::settings::{
    self, Settings, SettingsProblem, DEFAULT_BASELINE_INTERVAL_SECS, DEFAULT_COUNTRY,
    DEFAULT_LANGUAGE, MAX_BASELINE_INTERVAL_SECS, MIN_BASELINE_INTERVAL_SECS,
};

/// A path inside a fresh temporary directory, removed when the guard drops.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("nm-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        Self(directory)
    }

    fn file(&self) -> PathBuf {
        self.0.join(settings::FILE_NAME)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_defaults_are_usable_and_conservative() {
    let defaults = Settings::default();
    assert_eq!(defaults.language, DEFAULT_LANGUAGE);
    assert_eq!(defaults.country, DEFAULT_COUNTRY);
    assert_eq!(
        defaults.baseline_interval_secs,
        DEFAULT_BASELINE_INTERVAL_SECS
    );
    assert!(
        !defaults.autostart,
        "starting with the session must be opt-in, never assumed"
    );
    assert_eq!(
        defaults.clone().sanitized(),
        defaults,
        "defaults must survive sanitizing"
    );
}

#[test]
fn the_default_country_has_a_bundled_list() {
    // Otherwise a fresh install would monitor nothing on its first run.
    assert!(nm_app::targets::has_country(DEFAULT_COUNTRY));
}

#[test]
fn an_unknown_country_falls_back_rather_than_monitoring_nothing() {
    let sane = Settings {
        country: "zz".to_owned(),
        ..Settings::default()
    }
    .sanitized();
    assert_eq!(sane.country, DEFAULT_COUNTRY);
}

#[test]
fn an_unknown_language_falls_back_to_the_one_that_exists() {
    let sane = Settings {
        language: "kl".to_owned(),
        ..Settings::default()
    }
    .sanitized();
    assert_eq!(sane.language, DEFAULT_LANGUAGE);
}

#[test]
fn the_probe_interval_is_clamped_into_range() {
    // A hand-edited file must not be able to point the probe engine at zero seconds.
    let fast = Settings {
        baseline_interval_secs: 0,
        ..Settings::default()
    }
    .sanitized();
    assert_eq!(fast.baseline_interval_secs, MIN_BASELINE_INTERVAL_SECS);

    let slow = Settings {
        baseline_interval_secs: u32::MAX,
        ..Settings::default()
    }
    .sanitized();
    assert_eq!(slow.baseline_interval_secs, MAX_BASELINE_INTERVAL_SECS);
}

#[test]
fn the_interval_converts_to_a_duration() {
    let settings = Settings {
        baseline_interval_secs: 7,
        ..Settings::default()
    };
    assert_eq!(
        settings.baseline_interval(),
        std::time::Duration::from_secs(7)
    );
}

#[test]
fn a_missing_file_is_a_first_run_not_a_problem() {
    let scratch = Scratch::new("missing");
    let loaded = settings::load(&scratch.file());
    assert_eq!(loaded.settings, Settings::default());
    assert_eq!(loaded.problem, None);
}

#[test]
fn settings_survive_a_round_trip_through_the_file() {
    let scratch = Scratch::new("roundtrip");
    let wanted = Settings {
        language: DEFAULT_LANGUAGE.to_owned(),
        country: "ir".to_owned(),
        baseline_interval_secs: 12,
        autostart: true,
        remember_game_servers: false,
        name_networks: false,
    };

    settings::store(&scratch.file(), &wanted).expect("the directory must be created");
    let loaded = settings::load(&scratch.file());

    assert_eq!(loaded.settings, wanted);
    assert_eq!(loaded.problem, None);
}

#[test]
fn a_partial_file_gains_defaults_for_what_it_omits() {
    // A file written by an older build must keep working.
    let scratch = Scratch::new("partial");
    std::fs::create_dir_all(&scratch.0).unwrap();
    std::fs::write(scratch.file(), r#"{"country":"ir"}"#).unwrap();

    let loaded = settings::load(&scratch.file());
    assert_eq!(loaded.settings.country, "ir");
    assert_eq!(
        loaded.settings.baseline_interval_secs,
        DEFAULT_BASELINE_INTERVAL_SECS
    );
    assert_eq!(loaded.problem, None);
}

#[test]
fn a_file_from_a_newer_build_keeps_the_fields_this_one_understands() {
    let scratch = Scratch::new("newer");
    std::fs::create_dir_all(&scratch.0).unwrap();
    std::fs::write(
        scratch.file(),
        r#"{"country":"ir","somethingFromTheFuture":42}"#,
    )
    .unwrap();

    let loaded = settings::load(&scratch.file());
    assert_eq!(loaded.settings.country, "ir");
    assert_eq!(loaded.problem, None);
}

#[test]
fn a_file_on_disk_is_sanitized_on_the_way_in() {
    let scratch = Scratch::new("outofrange");
    std::fs::create_dir_all(&scratch.0).unwrap();
    std::fs::write(
        scratch.file(),
        r#"{"country":"zz","baselineIntervalSecs":0}"#,
    )
    .unwrap();

    let loaded = settings::load(&scratch.file());
    assert_eq!(loaded.settings.country, DEFAULT_COUNTRY);
    assert_eq!(
        loaded.settings.baseline_interval_secs,
        MIN_BASELINE_INTERVAL_SECS
    );
}

#[test]
fn a_malformed_file_reports_itself_and_is_not_overwritten() {
    // Silently resetting someone's configuration — and destroying the evidence — is how a
    // parsing bug of ours becomes their lost afternoon.
    let scratch = Scratch::new("malformed");
    std::fs::create_dir_all(&scratch.0).unwrap();
    std::fs::write(scratch.file(), "not json at all").unwrap();

    let loaded = settings::load(&scratch.file());
    assert_eq!(loaded.settings, Settings::default());
    assert_eq!(loaded.problem, Some(SettingsProblem::Malformed));
    assert_eq!(
        std::fs::read_to_string(scratch.file()).unwrap(),
        "not json at all"
    );
}

#[test]
fn writing_creates_the_directory_it_needs() {
    let scratch = Scratch::new("mkdir");
    assert!(!scratch.0.exists());
    settings::store(&scratch.file(), &Settings::default()).unwrap();
    assert!(scratch.file().exists());
}

#[test]
fn the_stored_file_is_readable_by_a_human() {
    // These files are part of what makes the app auditable; a single line of JSON is not.
    let scratch = Scratch::new("pretty");
    settings::store(&scratch.file(), &Settings::default()).unwrap();
    let raw = std::fs::read_to_string(scratch.file()).unwrap();
    assert!(raw.contains('\n'), "{raw}");
    assert!(raw.contains("baselineIntervalSecs"), "{raw}");
}
