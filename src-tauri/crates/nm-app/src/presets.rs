//! Known-application presets: which executables belong to one application.
//!
//! An application is not a process, and most of the grouping needs no data at all — the
//! executable name catches an Electron application's helpers, and the process tree catches
//! a launcher starting a title. This file is for what those two rules cannot see: a title
//! whose companion executable is neither its parent nor its namesake.
//!
//! The list is `assets/apps/presets.json`, compiled in with [`include_str!`] exactly like
//! the baseline target lists, so the app never fetches it and cannot be made to.
//! `assets/apps/README.md` documents the schema and the rules for adding an entry —
//! including the one that matters most: never list an executable that several applications
//! share, or picking one game would silently drag another into it.
//!
//! # Why `deny_unknown_fields`
//!
//! The same reasoning as the target lists. A misspelled key here is an executable that will
//! never be grouped, and a grouping that silently fails to happen is invisible: the user
//! sees an application that is missing half its endpoints and no reason why. Loud is the
//! only safe failure.

use serde::Deserialize;

use crate::Error;

/// The schema version this build understands.
const SUPPORTED_SCHEMA: u32 = 1;

/// The bundled presets.
const PRESETS_JSON: &str = include_str!("../../../../assets/apps/presets.json");

/// One application whose processes cannot be grouped from the operating system alone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    /// Stable identifier, unique in the file. Never shown to the user.
    pub id: String,
    /// The application's own name for itself. A proper noun: shown as written, never
    /// translated.
    pub label: String,
    /// Executable names that belong to this application, compared case-insensitively —
    /// which is how Windows compares them.
    ///
    /// The first is the one the application is named after when several are running.
    pub executables: Vec<String>,
}

impl Preset {
    /// Whether an executable name belongs to this application.
    #[must_use]
    pub fn covers(&self, executable: &str) -> bool {
        self.executables
            .iter()
            .any(|known| known.eq_ignore_ascii_case(executable))
    }
}

/// The parsed preset file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresetList {
    /// Schema version of the file.
    pub schema_version: u32,
    /// The applications, in file order.
    pub applications: Vec<Preset>,
}

impl PresetList {
    /// Parses and validates a preset file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AppPresets`] for malformed JSON, an unsupported schema version, an
    /// empty or duplicated identifier, a preset with no executables, or — the one that
    /// actually protects the user — an executable claimed by two presets. That last one
    /// would make grouping depend on file order, which is to say on nothing the user can
    /// see, so it is refused rather than resolved.
    pub fn parse(json: &str) -> Result<Self, Error> {
        let list: Self = serde_json::from_str(json).map_err(|source| Error::AppPresets {
            reason: source.to_string(),
        })?;

        let complain = |reason: String| Error::AppPresets { reason };

        if list.schema_version != SUPPORTED_SCHEMA {
            return Err(complain(format!(
                "schema version {} is not supported (this build understands {SUPPORTED_SCHEMA})",
                list.schema_version
            )));
        }

        for (index, preset) in list.applications.iter().enumerate() {
            if preset.id.is_empty() || preset.label.is_empty() {
                return Err(complain(format!("preset {index} has an empty id or label")));
            }
            if preset.executables.is_empty() {
                return Err(complain(format!(
                    "preset {:?} lists no executables",
                    preset.id
                )));
            }
            if preset.executables.iter().any(String::is_empty) {
                return Err(complain(format!(
                    "preset {:?} lists an empty executable name",
                    preset.id
                )));
            }
            for earlier in &list.applications[..index] {
                if earlier.id == preset.id {
                    return Err(complain(format!("duplicate preset id {:?}", preset.id)));
                }
                if let Some(shared) = preset
                    .executables
                    .iter()
                    .find(|executable| earlier.covers(executable))
                {
                    return Err(complain(format!(
                        "{shared:?} is claimed by both {:?} and {:?}",
                        earlier.id, preset.id
                    )));
                }
            }
        }

        Ok(list)
    }

    /// Loads the bundled presets.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AppPresets`] when the bundled file does not validate. A test
    /// asserts it does, so this is a build-time guarantee rather than a runtime hope.
    pub fn bundled() -> Result<Self, Error> {
        Self::parse(PRESETS_JSON)
    }

    /// A list that groups nothing.
    ///
    /// What the app falls back to if the bundled file ever failed to load: the executable
    /// name and the process tree still group most applications correctly, so losing the
    /// presets costs the awkward titles and nothing else.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            schema_version: SUPPORTED_SCHEMA,
            applications: Vec::new(),
        }
    }

    /// The preset an executable belongs to, if any.
    #[must_use]
    pub fn matching(&self, executable: &str) -> Option<&Preset> {
        self.applications
            .iter()
            .find(|preset| preset.covers(executable))
    }
}
