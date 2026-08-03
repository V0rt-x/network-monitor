//! User settings: what they are, how they are kept sane, and how they reach the disk.
//!
//! Two halves, deliberately separated. Everything above [`spawn_writer`] is pure: a struct,
//! its defaults, and a [`Settings::sanitized`] pass that a fuzzed file cannot get past. The
//! writer is the only part that touches a filesystem, and it debounces — a user dragging a
//! slider must not turn into a write per frame.
//!
//! Settings live on this machine and nowhere else. There is no account, no sync and no
//! remote default: the file below is the whole of it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::baselines;

/// Name of the settings file inside the application's configuration directory.
pub const FILE_NAME: &str = "settings.json";

/// The only UI language that exists so far. Russian is planned and purely additive.
pub const DEFAULT_LANGUAGE: &str = "en";

/// Country assumed until the user picks one.
///
/// A guess, and shown as one — the alternative is geo-detection, which means asking a
/// remote service where the user is. This product never does that.
pub const DEFAULT_COUNTRY: &str = "ru";

/// Shortest baseline probe interval the user may choose.
///
/// One second per target is the engine's own default; below it the baselines would start
/// competing with per-app monitoring for the global cap.
pub const MIN_BASELINE_INTERVAL_SECS: u32 = 1;

/// Longest baseline probe interval the user may choose.
pub const MAX_BASELINE_INTERVAL_SECS: u32 = 60;

/// Baseline probe interval out of the box.
///
/// Eight baseline targets at five seconds is 1.6 probes/s — comfortably inside the 32/s
/// global cap, leaving the bulk of it for the applications the user actually monitors.
pub const DEFAULT_BASELINE_INTERVAL_SECS: u32 = 5;

/// How long the writer waits for the user to stop fiddling before it writes.
pub const WRITE_DEBOUNCE: Duration = Duration::from_millis(750);

/// Everything the user can configure.
///
/// Deliberately *not* tolerant of missing fields: this type crosses the IPC boundary, and a
/// TypeScript type whose every field is optional would let the UI send half a
/// configuration and force every reader to guess at the rest. Tolerance for older files
/// lives in [`Stored`] instead, where it belongs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// UI language tag.
    pub language: String,
    /// Country whose domestic baseline list is monitored.
    pub country: String,
    /// Seconds between probes of one baseline target.
    pub baseline_interval_secs: u32,
    /// Whether the app starts with the user's session. Off unless asked for.
    pub autostart: bool,
    /// Whether the addresses a monitored game connects to are remembered between sessions.
    ///
    /// On by default, because without it a game reference pool is only as good as what the
    /// operator publishes — and for most titles that is nothing, leaving the app unable to
    /// tell "the game's servers are down" from "you cannot reach them". It is a setting all
    /// the same: it is the one thing this application writes to disk that describes what the
    /// user plays and where, and for the people this product is for that is worth a choice
    /// rather than an assumption. Turning it off also deletes what was already remembered.
    pub remember_game_servers: bool,
    /// Whether endpoints and route hops are labelled with the network that announces them.
    ///
    /// On by default: an address is four numbers, and the name of the network behind it is
    /// what makes the route panel readable to someone who did not come here knowing what an
    /// autonomous system is. It costs nothing to privacy — the directory is bundled, and
    /// looking an address up in it sends nothing anywhere.
    ///
    /// It is a setting because it costs memory. The directory is around 12 MB resident, a
    /// quarter of the core's budget, and it is the only part of this application a user
    /// might reasonably want to trade away on a machine where the game needs every
    /// megabyte. Switching it off frees that immediately rather than at the next restart.
    pub name_networks: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: DEFAULT_LANGUAGE.to_owned(),
            country: DEFAULT_COUNTRY.to_owned(),
            baseline_interval_secs: DEFAULT_BASELINE_INTERVAL_SECS,
            autostart: false,
            remember_game_servers: true,
            name_networks: true,
        }
    }
}

impl Settings {
    /// The same settings with every field forced into a usable range.
    ///
    /// Applied to whatever comes off the disk and to whatever the UI sends, so neither a
    /// hand-edited file nor a future UI bug can put the probe engine somewhere it must not
    /// go. Out-of-range values fall back to the default rather than to the nearest bound
    /// when the field is an identifier, where "nearest" means nothing.
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            language: if self.language == DEFAULT_LANGUAGE {
                self.language
            } else {
                DEFAULT_LANGUAGE.to_owned()
            },
            country: if baselines::has_country(&self.country) {
                self.country
            } else {
                DEFAULT_COUNTRY.to_owned()
            },
            baseline_interval_secs: self
                .baseline_interval_secs
                .clamp(MIN_BASELINE_INTERVAL_SECS, MAX_BASELINE_INTERVAL_SECS),
            autostart: self.autostart,
            remember_game_servers: self.remember_game_servers,
            name_networks: self.name_networks,
        }
    }

    /// The baseline probe interval as a duration.
    #[must_use]
    pub fn baseline_interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.baseline_interval_secs))
    }
}

/// The settings file as it may actually be found on disk.
///
/// Every field is optional and every unknown one is ignored, so a file written by an older
/// build — or by a newer one the user has since downgraded from — still loads, gaining the
/// defaults for whatever it does not mention. This is the only place that tolerance exists;
/// [`Settings`] itself stays strict.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stored {
    language: Option<String>,
    country: Option<String>,
    baseline_interval_secs: Option<u32>,
    autostart: Option<bool>,
    remember_game_servers: Option<bool>,
    name_networks: Option<bool>,
}

impl Stored {
    /// Fills the gaps with defaults.
    fn merged(self) -> Settings {
        let defaults = Settings::default();
        Settings {
            language: self.language.unwrap_or(defaults.language),
            country: self.country.unwrap_or(defaults.country),
            baseline_interval_secs: self
                .baseline_interval_secs
                .unwrap_or(defaults.baseline_interval_secs),
            autostart: self.autostart.unwrap_or(defaults.autostart),
            remember_game_servers: self
                .remember_game_servers
                .unwrap_or(defaults.remember_game_servers),
            name_networks: self.name_networks.unwrap_or(defaults.name_networks),
        }
    }
}

/// What went wrong reading the settings file, if anything.
///
/// Surfaced to the UI instead of being swallowed: settings silently reverting to defaults
/// is the kind of thing a user notices a week later and never trusts again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SettingsProblem {
    /// The file exists but could not be read.
    Unreadable,
    /// The file was read but is not valid settings; defaults are in use.
    Malformed,
    /// Settings could not be written back.
    NotWritable,
}

/// Settings as loaded, together with anything that went wrong loading them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The settings to use — always usable, defaults if the file was not.
    pub settings: Settings,
    /// What went wrong, if anything. A missing file is not a problem: it is a first run.
    pub problem: Option<SettingsProblem>,
}

/// Reads settings from `path`, falling back to defaults on anything unusable.
///
/// A malformed file is **not** overwritten here. The user's file is left exactly as it is
/// until they change a setting, so a parsing bug of ours cannot destroy a configuration
/// that a fixed build would have read fine.
#[must_use]
pub fn load(path: &Path) -> Loaded {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                settings: Settings::default(),
                problem: None,
            }
        }
        Err(_) => {
            return Loaded {
                settings: Settings::default(),
                problem: Some(SettingsProblem::Unreadable),
            }
        }
    };

    match serde_json::from_str::<Stored>(&raw) {
        Ok(stored) => Loaded {
            settings: stored.merged().sanitized(),
            problem: None,
        },
        Err(_) => Loaded {
            settings: Settings::default(),
            problem: Some(SettingsProblem::Malformed),
        },
    }
}

/// Writes settings to `path`, creating the directory if it is missing.
///
/// # Errors
///
/// Returns [`SettingsProblem::NotWritable`] if the directory or the file cannot be
/// written. The caller reports it; there is nothing useful to retry.
pub fn store(path: &Path, settings: &Settings) -> Result<(), SettingsProblem> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(|_| SettingsProblem::NotWritable)?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|_| SettingsProblem::NotWritable)?;
    std::fs::write(path, json).map_err(|_| SettingsProblem::NotWritable)
}

/// Persists settings in the background, coalescing rapid changes into one write.
///
/// The task ends when every sender is dropped, i.e. at shutdown. Writes happen on the
/// blocking pool so a slow disk cannot stall the probe engine sharing this runtime.
pub fn spawn_writer(path: PathBuf, mut changes: tokio::sync::watch::Receiver<Settings>) {
    tauri::async_runtime::spawn(async move {
        while changes.changed().await.is_ok() {
            // Wait out the burst: a slider dragged across its range is one write, not one
            // per pixel. Any change arriving during the wait restarts it.
            loop {
                tokio::select! {
                    () = tokio::time::sleep(WRITE_DEBOUNCE) => break,
                    another = changes.changed() => {
                        if another.is_err() {
                            return;
                        }
                    }
                }
            }

            // Cloned out of the watch before any await: a borrow held across one would
            // block every writer of the channel.
            let settings = changes.borrow_and_update().clone();
            let path = path.clone();
            if tokio::task::spawn_blocking(move || store(&path, &settings))
                .await
                .is_err()
            {
                return;
            }
        }
    });
}
