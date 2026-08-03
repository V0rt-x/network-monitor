//! The bundled autonomous-system directory: getting it into memory, and asking it things.
//!
//! [`nm_core::asn`] owns the lookup itself and knows nothing about files. This module is the
//! other half: it holds the two compressed assets, decompresses them off the runtime, and
//! hands the finished table to the view layer in the same shape the adapter names arrive in
//! — a snapshot passed by reference into one emission, never a global reached from a
//! rendering function.
//!
//! # Nothing here touches the network
//!
//! Both files are compiled in. There is no fetch, at first run or ever, and
//! `assets/asn/README.md` records why: every ready-made copy of this data on the internet
//! sits behind a CDN that is throttled or filtered in the countries this product is for, so
//! a downloaded directory would work everywhere except where it is needed.
//!
//! # Why it is loaded lazily, and on the blocking pool
//!
//! Around 570 000 announced blocks decompress and parse in a few hundred milliseconds and
//! settle at roughly 12 MB — a quarter of the core's memory budget. Neither cost is one to
//! pay during startup for a user who has the feature switched off, and neither may be paid
//! on the async runtime, where it would stall every probe in flight. So the load happens
//! once, on the blocking pool, only when the setting is on, and the names simply are not
//! there until it lands.
//!
//! Absent is a state the whole product already knows how to render: an endpoint with no name
//! shows no name. It never shows a guess, and it never waits for one.

use std::io::{BufRead, BufReader};
use std::net::IpAddr;
use std::sync::Arc;

use flate2::read::GzDecoder;
use nm_core::asn::{AsnTable, AsnTableBuilder};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The announced blocks, sorted, gzip-compressed.
const RANGES_GZ: &[u8] = include_bytes!("../../../../assets/asn/ranges.tsv.gz");

/// The directory of autonomous systems, gzip-compressed.
const DIRECTORY_GZ: &[u8] = include_bytes!("../../../../assets/asn/asn.tsv.gz");

/// The day the bundled snapshot was taken, as an ISO date.
///
/// Shown to the user beside the figures it explains, because a directory of networks is a
/// photograph of the internet on one day and an old photograph explains a wrong name. Kept
/// in step with `assets/asn/README.md` by a test in `tests/asn.rs`.
pub const SNAPSHOT_DATE: &str = "2026-08-03";

/// How many blocks the bundled snapshot holds, used to size the table in one allocation.
///
/// A hint, not a contract: a snapshot that has grown since simply reallocates once.
const EXPECTED_RANGES: usize = 573_125;

/// How many autonomous systems the bundled snapshot names.
const EXPECTED_ORGANISATIONS: usize = 86_628;

/// What went wrong loading the bundled directory.
///
/// Only reachable through a corrupt installation: the assets are compiled into the binary,
/// so there is no file for a user to lose or a disk to fail on. It is an error rather than
/// a panic because a broken names feature must cost the user their endpoint labels and
/// nothing else — the measurements underneath it are unaffected and must keep running.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The compiled-in data would not decompress.
    #[error("the bundled network directory could not be decompressed: {0}")]
    Decompress(#[from] std::io::Error),

    /// The data decompressed but does not parse.
    #[error("the bundled network directory is malformed: {0}")]
    Malformed(#[from] nm_core::Error),
}

/// Decompresses and parses both bundled assets.
///
/// **Blocking, and measured in hundreds of milliseconds.** Callers must put it on the
/// blocking pool; running it on the async runtime would hold up every probe for the
/// duration.
///
/// # Errors
///
/// Returns [`LoadError`] if the compiled-in assets cannot be decompressed or contain a row
/// the parser rejects — a corrupt build in either case.
pub fn load() -> Result<AsnTable, LoadError> {
    let mut builder = AsnTableBuilder::with_capacity(EXPECTED_RANGES, EXPECTED_ORGANISATIONS);
    for_each_line(RANGES_GZ, |line| builder.push_range(line))?;
    for_each_line(DIRECTORY_GZ, |line| builder.push_organisation(line))?;
    Ok(builder.finish())
}

/// Streams a gzip-compressed asset one line at a time.
///
/// Streamed rather than decompressed whole so the 17 MB of text and the 12 MB table never
/// exist at once, and the line buffer is reused so half a million rows cost one allocation
/// rather than half a million.
fn for_each_line(
    compressed: &[u8],
    mut consume: impl FnMut(&str) -> Result<(), nm_core::Error>,
) -> Result<(), LoadError> {
    let mut reader = BufReader::new(GzDecoder::new(compressed));
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        consume(line.trim_end_matches(['\r', '\n']))?;
    }
}

/// The loaded directory, or the absence of one, as the view layer sees it.
///
/// Cloned per emission and shared rather than copied — the table behind the [`Arc`] is the
/// one loaded copy, and cloning this is two pointer bumps.
#[derive(Debug, Clone, Default)]
pub struct NetworkNames(Option<Arc<AsnTable>>);

impl NetworkNames {
    /// Wraps a freshly loaded table.
    #[must_use]
    pub fn of(table: AsnTable) -> Self {
        Self(Some(Arc::new(table)))
    }

    /// The empty directory: every address goes unnamed.
    ///
    /// What the view layer holds before the load lands, and after the user switches the
    /// feature off.
    #[must_use]
    pub fn none() -> Self {
        Self(None)
    }

    /// Names the network an address belongs to, if the directory is loaded and knows it.
    #[must_use]
    pub fn name_of(&self, address: IpAddr) -> Option<NetworkView> {
        let found = self.0.as_ref()?.lookup(address)?;
        Some(NetworkView {
            asn: found.number,
            // An empty registered name is a real gap in the data, and `null` is how the
            // rest of this boundary spells one. The UI falls back to the number, which is
            // still true.
            name: (!found.name.is_empty()).then(|| found.name.to_owned()),
            country: found.country.map(|code| code.as_str().to_owned()),
        })
    }

    /// Whether anything is loaded at all.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.0.is_some()
    }

    /// How many announced blocks are loaded, for the settings panel to state.
    #[must_use]
    pub fn range_count(&self) -> usize {
        self.0.as_ref().map_or(0, |table| table.range_count())
    }

    /// Roughly how much memory the loaded directory occupies.
    #[must_use]
    pub fn approximate_heap_bytes(&self) -> usize {
        self.0
            .as_ref()
            .map_or(0, |table| table.approximate_heap_bytes())
    }
}

/// Which network an address belongs to, as the UI receives it.
///
/// Three separate fields rather than one formatted string: the name belongs at level one,
/// the number and the country are level-two detail, and a backend that pre-formats them
/// into a sentence takes that choice away from the page and out of the translators' hands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkView {
    /// The autonomous system number announcing the address.
    pub asn: u32,
    /// The name its holder registered, or `null` where the directory has none.
    pub name: Option<String>,
    /// The two-letter country the system was **registered** in, where one is recorded.
    ///
    /// **Not a location, and the UI must never present it as one.** An anycast address is
    /// announced from dozens of cities at once and a cloud provider registered in one
    /// country runs regions on every continent; the measured round trip is the better
    /// evidence of distance, never this.
    pub country: Option<String>,
}
