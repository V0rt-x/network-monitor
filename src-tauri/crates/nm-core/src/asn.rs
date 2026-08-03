//! Naming the far end: which network an address belongs to.
//!
//! Every other module in this crate measures something. This one does not measure at all —
//! it answers a question the user asks before any figure means anything to them: *what is
//! that?* An address is four numbers, and four numbers are not something a person can act
//! on. "Cloudflare", "Amazon", the name of their own provider — those are, and they are
//! what turns the route panel from a column of hops into a sentence about where the traffic
//! goes and where it stops going.
//!
//! # What it can claim, and what it cannot
//!
//! The table says which **autonomous system** announces an address: a number, the name its
//! holder registered, and the country that registration was made in. That is a fact about
//! routing, and it is stable enough to bundle — an address block changes hands over months,
//! not minutes.
//!
//! The registration country is **not a location**. An anycast address is announced from
//! dozens of cities at once, and a cloud provider registered in one country runs regions on
//! every continent; either can put the registry's answer thousands of kilometres from the
//! machine that actually replied. The measured round trip is the better evidence of
//! distance, and never the other way round. Callers must present the country as what it is,
//! and the user interface must not let it stand in for where a server is.
//!
//! # Why the data is bundled and this module is pure
//!
//! Asking a remote service to name each address — RDAP, whois, Team Cymru's DNS interface —
//! would tell a third party which servers this user is playing on, from a machine that may
//! be watched. That is exactly the phone-home the product promises never to make, so the
//! answer ships in the box instead (`assets/asn/`, and the README there for why downloading
//! it on demand fails the people this is for).
//!
//! Being a bundle rather than a service is also what lets the lookup live here, in the pure
//! core: it is a binary search over sorted arrays, with no I/O, no clock and no allocation
//! per query.
//!
//! # Cost
//!
//! Roughly 12 MB resident for the full bundled snapshot — about a quarter of the core's
//! memory budget, which is why the application loads it lazily and only when the user has
//! the feature switched on. [`AsnTable::approximate_heap_bytes`] reports the real figure so
//! the setting can state it rather than guess.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::Error;

/// Name of the range file, used in the error a malformed row produces.
pub const RANGES_FILE: &str = "ranges.tsv";

/// Name of the directory file, used in the error a malformed row produces.
pub const DIRECTORY_FILE: &str = "asn.tsv";

/// The autonomous system number upstream uses for address space nobody announces.
///
/// Rows carrying it are dropped when the bundle is generated, and skipped again here: an
/// unrouted block is not something this application ever asks about, and keeping it would
/// mean answering "AS0, Not routed" where the honest answer is nothing at all.
const NOT_ROUTED: u32 = 0;

/// A two-letter country code, as registered with a regional internet registry.
///
/// Deliberately not called a location. See the module documentation: this is where an
/// address block was *registered*, which for anycast and for any cloud provider is
/// routinely nowhere near the machine that answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CountryCode([u8; 2]);

impl CountryCode {
    /// Reads a code, accepting only two ASCII letters.
    ///
    /// Everything else — upstream's `None` marker, an empty field, a three-letter code from
    /// a hand-edited file — is [`None`] rather than an error. A missing country is an
    /// ordinary gap in the data, not a corrupt file, and it costs nothing: the number and
    /// the name are the useful half anyway.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let bytes = raw.as_bytes();
        let [first, second] = bytes else {
            return None;
        };
        if !first.is_ascii_alphabetic() || !second.is_ascii_alphabetic() {
            return None;
        }
        Some(Self([
            first.to_ascii_uppercase(),
            second.to_ascii_uppercase(),
        ]))
    }

    /// The code as text, always two uppercase ASCII letters.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // The bytes are ASCII by construction, so this cannot fail; the fallback exists
        // because library code here may not panic, not because it can be reached.
        std::str::from_utf8(&self.0).unwrap_or("")
    }
}

/// What the table knows about one address.
///
/// Borrowed from the table rather than owned, so a lookup allocates nothing. Callers that
/// need to keep it — a view crossing the IPC boundary, say — copy the pieces they want.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribution<'a> {
    /// The autonomous system number announcing the address.
    pub number: u32,
    /// The name its holder registered, or an empty string where the directory has no entry.
    ///
    /// Empty is possible and is not an error: the range file and the directory are two
    /// files, and a number present in one and missing from the other still names a true
    /// fact — which network announces the address — that the caller can show as `AS64500`.
    pub name: &'a str,
    /// The country the system was registered in, where the registry recorded one.
    pub country: Option<CountryCode>,
}

/// One announced IPv4 block, with both ends inclusive.
///
/// Twelve bytes, and there are around 450 000 of them, so the layout is not incidental.
#[derive(Debug, Clone, Copy)]
struct V4Range {
    start: Ipv4Addr,
    end: Ipv4Addr,
    number: u32,
}

/// One announced IPv6 block, with both ends inclusive.
///
/// [`Ipv6Addr`] rather than [`u128`] on purpose: it wraps a byte array, so it aligns to one
/// and this record is 36 bytes instead of the 48 that a `u128` pair's 16-byte alignment
/// would pad it to. Its ordering is the numeric one either way.
#[derive(Debug, Clone, Copy)]
struct V6Range {
    start: Ipv6Addr,
    end: Ipv6Addr,
    number: u32,
}

/// One autonomous system's registration, pointing into the name arena.
#[derive(Debug, Clone, Copy)]
struct Organisation {
    number: u32,
    name_at: u32,
    name_len: u16,
    country: Option<CountryCode>,
}

/// Sorted announcements, searchable by address.
///
/// Built through [`AsnTableBuilder`], which is what lets the application stream a
/// compressed bundle through it a line at a time instead of holding the decompressed text
/// and the finished table in memory at once.
#[derive(Debug, Default)]
pub struct AsnTable {
    v4: Vec<V4Range>,
    v6: Vec<V6Range>,
    organisations: Vec<Organisation>,
    names: String,
}

impl AsnTable {
    /// Builds a table from two whole files.
    ///
    /// A convenience for tests and for callers small enough not to care about peak memory;
    /// the application streams through [`AsnTableBuilder`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MalformedAsnRow`] naming the file and line of the first row that
    /// cannot be read, or [`Error::AsnDataTooLarge`] if the names exceed what the arena can
    /// index.
    pub fn parse(ranges: &str, directory: &str) -> Result<Self, Error> {
        let mut builder = AsnTableBuilder::default();
        for line in ranges.lines() {
            builder.push_range(line)?;
        }
        for line in directory.lines() {
            builder.push_organisation(line)?;
        }
        Ok(builder.finish())
    }

    /// Names the network an address belongs to, or [`None`] if nothing announces it.
    ///
    /// [`None`] is a real and common answer — unallocated space, a private address, a block
    /// the snapshot predates — and callers must show nothing rather than a guess.
    #[must_use]
    pub fn lookup(&self, address: IpAddr) -> Option<Attribution<'_>> {
        let number = match address {
            IpAddr::V4(v4) => Self::search_v4(&self.v4, v4)?,
            IpAddr::V6(v6) => Self::search_v6(&self.v6, v6)?,
        };
        Some(self.describe(number))
    }

    /// Finds the IPv4 block containing `address` among ranges sorted by their start.
    ///
    /// The blocks the bundle carries do not overlap. Where a hand-edited file makes them,
    /// the one with the greatest start that still contains the address wins and a block
    /// entirely shadowed by a later one is never consulted — chosen because it needs no
    /// second pass and because the alternative, rejecting the file, would turn a cosmetic
    /// mistake into a feature that refuses to load.
    ///
    /// Written out once per family rather than made generic: the two bodies are four lines
    /// each, and the abstraction that unified them cost more to read than both together.
    fn search_v4(ranges: &[V4Range], address: Ipv4Addr) -> Option<u32> {
        let index = ranges.partition_point(|range| range.start <= address);
        let candidate = ranges.get(index.checked_sub(1)?)?;
        (candidate.end >= address).then_some(candidate.number)
    }

    /// Finds the IPv6 block containing `address`. See [`search_v4`](Self::search_v4).
    fn search_v6(ranges: &[V6Range], address: Ipv6Addr) -> Option<u32> {
        let index = ranges.partition_point(|range| range.start <= address);
        let candidate = ranges.get(index.checked_sub(1)?)?;
        (candidate.end >= address).then_some(candidate.number)
    }

    /// Looks a number up in the directory, falling back to a nameless attribution.
    fn describe(&self, number: u32) -> Attribution<'_> {
        let Ok(index) = self
            .organisations
            .binary_search_by_key(&number, |entry| entry.number)
        else {
            return Attribution {
                number,
                name: "",
                country: None,
            };
        };
        let entry = self.organisations[index];
        let at = entry.name_at as usize;
        let name = self
            .names
            .get(at..at + usize::from(entry.name_len))
            .unwrap_or("");
        Attribution {
            number,
            name,
            country: entry.country,
        }
    }

    /// How many announced blocks the table holds.
    #[must_use]
    pub fn range_count(&self) -> usize {
        self.v4.len() + self.v6.len()
    }

    /// How many autonomous systems the directory names.
    #[must_use]
    pub fn organisation_count(&self) -> usize {
        self.organisations.len()
    }

    /// Whether the table can answer anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.v4.is_empty() && self.v6.is_empty()
    }

    /// Roughly how much memory the table holds, in bytes.
    ///
    /// Reported rather than estimated in a comment because the setting that switches this
    /// feature on states the cost to the user, and a stated cost that drifts from the real
    /// one is worse than no figure.
    #[must_use]
    pub fn approximate_heap_bytes(&self) -> usize {
        self.v4.capacity() * size_of::<V4Range>()
            + self.v6.capacity() * size_of::<V6Range>()
            + self.organisations.capacity() * size_of::<Organisation>()
            + self.names.capacity()
    }
}

/// Accumulates rows into a table, sorting once at the end.
///
/// Rows may arrive in any order and the two files may be interleaved; nothing is sorted
/// until [`finish`](Self::finish), because sorting 570 000 records once is cheaper than
/// keeping them ordered on the way in.
#[derive(Debug, Default)]
pub struct AsnTableBuilder {
    v4: Vec<V4Range>,
    v6: Vec<V6Range>,
    organisations: Vec<Organisation>,
    names: String,
    ranges_seen: usize,
    organisations_seen: usize,
}

impl AsnTableBuilder {
    /// A builder with room reserved for a bundle of the given size.
    ///
    /// Reserving matters here: without it, growing four vectors to half a million entries
    /// reallocates repeatedly and briefly holds twice the final table.
    #[must_use]
    pub fn with_capacity(ranges: usize, organisations: usize) -> Self {
        Self {
            v4: Vec::with_capacity(ranges),
            v6: Vec::new(),
            organisations: Vec::with_capacity(organisations),
            names: String::with_capacity(organisations * 24),
            ..Self::default()
        }
    }

    /// Reads one row of the range file: `range_start`, `range_end`, `as_number`.
    ///
    /// Blank lines and lines beginning with `#` are skipped, as are rows announcing
    /// [`NOT_ROUTED`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::MalformedAsnRow`] for a row with the wrong number of fields, an
    /// unparseable address or number, ends belonging to different address families, or an
    /// end below its start.
    pub fn push_range(&mut self, line: &str) -> Result<(), Error> {
        self.ranges_seen += 1;
        if skippable(line) {
            return Ok(());
        }
        let malformed = || Error::MalformedAsnRow {
            file: RANGES_FILE,
            line: self.ranges_seen,
            raw: truncated(line),
        };

        let mut fields = line.split('\t');
        let (Some(start), Some(end), Some(number), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(malformed());
        };
        let start: IpAddr = start.parse().map_err(|_| malformed())?;
        let end: IpAddr = end.parse().map_err(|_| malformed())?;
        let number: u32 = number.parse().map_err(|_| malformed())?;

        if number == NOT_ROUTED {
            return Ok(());
        }
        match (start, end) {
            (IpAddr::V4(start), IpAddr::V4(end)) if start <= end => {
                self.v4.push(V4Range { start, end, number });
            }
            (IpAddr::V6(start), IpAddr::V6(end)) if start <= end => {
                self.v6.push(V6Range { start, end, number });
            }
            _ => return Err(malformed()),
        }
        Ok(())
    }

    /// Reads one row of the directory file: `as_number`, `country`, `as_description`.
    ///
    /// The description is the rest of the line, so a name containing a tab survives intact.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MalformedAsnRow`] for a row with too few fields or an unparseable
    /// number, and [`Error::AsnDataTooLarge`] if the names outgrow the arena's index.
    pub fn push_organisation(&mut self, line: &str) -> Result<(), Error> {
        self.organisations_seen += 1;
        if skippable(line) {
            return Ok(());
        }
        let malformed = || Error::MalformedAsnRow {
            file: DIRECTORY_FILE,
            line: self.organisations_seen,
            raw: truncated(line),
        };

        let mut fields = line.splitn(3, '\t');
        let (Some(number), Some(country), Some(name)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(malformed());
        };
        let number: u32 = number.parse().map_err(|_| malformed())?;
        if number == NOT_ROUTED {
            return Ok(());
        }
        let name = name.trim();
        let name_len = u16::try_from(name.len()).map_err(|_| malformed())?;
        let name_at = u32::try_from(self.names.len()).map_err(|_| Error::AsnDataTooLarge)?;

        self.names.push_str(name);
        self.organisations.push(Organisation {
            number,
            name_at,
            name_len,
            country: CountryCode::parse(country),
        });
        Ok(())
    }

    /// Sorts what was pushed and hands back the finished table.
    ///
    /// Where one number is registered twice — two directory files concatenated, a
    /// hand-edited duplicate — the first row pushed wins, so a bundle's own entry cannot be
    /// displaced by something appended after it.
    #[must_use]
    pub fn finish(mut self) -> AsnTable {
        self.v4.sort_unstable_by_key(|range| range.start);
        self.v6.sort_unstable_by_key(|range| range.start);
        // A stable sort is what makes "the first row pushed wins" true after `dedup`.
        self.organisations.sort_by_key(|entry| entry.number);
        self.organisations.dedup_by_key(|entry| entry.number);
        AsnTable {
            v4: self.v4,
            v6: self.v6,
            organisations: self.organisations,
            names: self.names,
        }
    }
}

/// Whether a line carries no record: blank, whitespace, or a comment.
fn skippable(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

/// Keeps a rejected row short enough to be an error message rather than a paste.
fn truncated(line: &str) -> String {
    const LIMIT: usize = 80;
    match line.char_indices().nth(LIMIT) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Documentation ranges throughout: nothing here was observed on anyone's machine, and
    /// TEST-NET blocks are reserved precisely so that examples need not borrow real ones.
    const RANGES: &str = "\
192.0.2.0\t192.0.2.127\t64500
198.51.100.0\t198.51.100.255\t64501
203.0.113.0\t203.0.113.255\t64502
2001:db8::\t2001:db8::ffff\t64503";

    const DIRECTORY: &str = "\
64500\tUS\tEXAMPLE-ONE Example Networks, Inc.
64501\tNone\tEXAMPLE-TWO
64503\tde\tEXAMPLE-SIX";

    fn table() -> AsnTable {
        AsnTable::parse(RANGES, DIRECTORY).unwrap()
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    #[test]
    fn names_the_network_announcing_an_address() {
        let table = table();
        let found = table.lookup(ip("192.0.2.9")).unwrap();
        assert_eq!(found.number, 64500);
        assert_eq!(found.name, "EXAMPLE-ONE Example Networks, Inc.");
        assert_eq!(found.country.unwrap().as_str(), "US");
    }

    #[test]
    fn both_ends_of_a_block_are_inside_it() {
        // Off-by-one here would silently mis-name the edges of every allocation.
        let table = table();
        assert_eq!(table.lookup(ip("192.0.2.0")).unwrap().number, 64500);
        assert_eq!(table.lookup(ip("192.0.2.127")).unwrap().number, 64500);
        assert!(table.lookup(ip("192.0.2.128")).is_none());
    }

    #[test]
    fn an_address_no_one_announces_is_unnamed() {
        let table = table();
        // Between the first and second blocks, and below the first.
        assert!(table.lookup(ip("198.51.99.255")).is_none());
        assert!(table.lookup(ip("10.0.0.1")).is_none());
        // Above the last IPv4 block.
        assert!(table.lookup(ip("203.0.114.0")).is_none());
    }

    #[test]
    fn the_last_block_in_the_table_still_answers() {
        // `partition_point` returning the length is the case a naive index would drop.
        let table = table();
        assert_eq!(table.lookup(ip("203.0.113.255")).unwrap().number, 64502);
        assert_eq!(
            table.lookup(ip("2001:db8::ffff")).unwrap().number,
            64503,
            "the last IPv6 block too"
        );
    }

    #[test]
    fn the_two_families_never_answer_for_each_other() {
        let table = table();
        assert_eq!(table.lookup(ip("2001:db8::1")).unwrap().number, 64503);
        assert!(table.lookup(ip("2001:db9::1")).is_none());
        // An IPv4-mapped address is not the IPv4 address as far as this table is concerned.
        assert!(table.lookup(ip("::ffff:192.0.2.9")).is_none());
    }

    #[test]
    fn rows_may_arrive_in_any_order() {
        let shuffled = "\
2001:db8::\t2001:db8::ffff\t64503
203.0.113.0\t203.0.113.255\t64502
192.0.2.0\t192.0.2.127\t64500
198.51.100.0\t198.51.100.255\t64501";
        let table = AsnTable::parse(shuffled, DIRECTORY).unwrap();
        assert_eq!(table.lookup(ip("192.0.2.9")).unwrap().number, 64500);
        assert_eq!(table.lookup(ip("203.0.113.1")).unwrap().number, 64502);
        assert_eq!(table.lookup(ip("2001:db8::1")).unwrap().number, 64503);
    }

    #[test]
    fn an_unrouted_block_is_not_an_answer() {
        // Upstream marks unannounced space as AS0. Keeping it would answer "AS0, Not
        // routed" where the honest answer is that nothing is known.
        let ranges = "192.0.2.0\t192.0.2.255\t0";
        let table = AsnTable::parse(ranges, "0\tNone\tNot routed").unwrap();
        assert!(table.lookup(ip("192.0.2.9")).is_none());
        assert!(table.is_empty());
        assert_eq!(table.organisation_count(), 0);
    }

    #[test]
    fn a_missing_country_is_a_gap_and_not_a_failure() {
        let table = table();
        let found = table.lookup(ip("198.51.100.1")).unwrap();
        assert_eq!(found.name, "EXAMPLE-TWO");
        assert!(
            found.country.is_none(),
            "upstream's `None` marker is an absent country, not a country called None"
        );
    }

    #[test]
    fn a_number_missing_from_the_directory_still_names_its_network() {
        // The range file and the directory are two files. A number in one and not the other
        // is still a true fact about routing, and the caller can render it as `AS64502`.
        let table = table();
        let found = table.lookup(ip("203.0.113.1")).unwrap();
        assert_eq!(found.number, 64502);
        assert_eq!(found.name, "");
        assert!(found.country.is_none());
    }

    #[test]
    fn the_first_registration_of_a_number_wins() {
        let directory = "\
64500\tUS\tTHE-BUNDLED-ONE
64500\tFR\tAPPENDED-LATER";
        let table = AsnTable::parse(RANGES, directory).unwrap();
        let found = table.lookup(ip("192.0.2.9")).unwrap();
        assert_eq!(found.name, "THE-BUNDLED-ONE");
        assert_eq!(found.country.unwrap().as_str(), "US");
        assert_eq!(table.organisation_count(), 1);
    }

    #[test]
    fn an_empty_table_answers_nothing_rather_than_failing() {
        let table = AsnTable::parse("", "").unwrap();
        assert!(table.is_empty());
        assert_eq!(table.range_count(), 0);
        assert!(table.lookup(ip("192.0.2.9")).is_none());
    }

    #[test]
    fn blank_lines_and_comments_carry_no_records() {
        let ranges = "\
# generated by the command in assets/asn/README.md

192.0.2.0\t192.0.2.127\t64500
";
        let table = AsnTable::parse(ranges, DIRECTORY).unwrap();
        assert_eq!(table.range_count(), 1);
        assert_eq!(table.lookup(ip("192.0.2.9")).unwrap().number, 64500);
    }

    #[test]
    fn a_malformed_range_row_names_its_file_and_line() {
        // The README promises a botched regeneration fails loudly rather than quietly
        // losing half the table, and this is where that promise is kept.
        let ranges = "\
192.0.2.0\t192.0.2.127\t64500
198.51.100.0\tnot-an-address\t64501";
        let error = AsnTable::parse(ranges, DIRECTORY).unwrap_err();
        let Error::MalformedAsnRow { file, line, .. } = error else {
            panic!("expected a malformed-row error, got {error:?}");
        };
        assert_eq!(file, RANGES_FILE);
        assert_eq!(line, 2);
    }

    #[test]
    fn a_range_row_with_the_wrong_field_count_is_rejected() {
        for raw in [
            "192.0.2.0\t192.0.2.127",
            "192.0.2.0\t192.0.2.127\t64500\textra",
            "192.0.2.0 192.0.2.127 64500",
            "192.0.2.0\t192.0.2.127\tsixty-four-thousand",
        ] {
            assert!(
                AsnTable::parse(raw, DIRECTORY).is_err(),
                "{raw:?} should not parse"
            );
        }
    }

    #[test]
    fn a_range_whose_ends_disagree_or_run_backwards_is_rejected() {
        // Both would corrupt the sorted search rather than merely lose one row.
        assert!(AsnTable::parse("192.0.2.0\t2001:db8::\t64500", DIRECTORY).is_err());
        assert!(AsnTable::parse("192.0.2.127\t192.0.2.0\t64500", DIRECTORY).is_err());
        assert!(AsnTable::parse("2001:db8::ffff\t2001:db8::\t64500", DIRECTORY).is_err());
    }

    #[test]
    fn a_single_address_block_is_allowed() {
        let table = AsnTable::parse("192.0.2.9\t192.0.2.9\t64500", DIRECTORY).unwrap();
        assert_eq!(table.lookup(ip("192.0.2.9")).unwrap().number, 64500);
        assert!(table.lookup(ip("192.0.2.10")).is_none());
    }

    #[test]
    fn a_malformed_directory_row_names_its_file_and_line() {
        let directory = "64500\tUS\tEXAMPLE-ONE\nnot-a-number\tUS\tEXAMPLE-TWO";
        let error = AsnTable::parse(RANGES, directory).unwrap_err();
        let Error::MalformedAsnRow { file, line, .. } = error else {
            panic!("expected a malformed-row error, got {error:?}");
        };
        assert_eq!(file, DIRECTORY_FILE);
        assert_eq!(line, 2);
    }

    #[test]
    fn a_directory_row_missing_a_field_is_rejected() {
        assert!(AsnTable::parse(RANGES, "64500\tUS").is_err());
        assert!(AsnTable::parse(RANGES, "64500").is_err());
    }

    #[test]
    fn a_name_keeps_its_spaces_and_any_tab_inside_it() {
        // The description is the rest of the line, so a stray tab does not truncate a name
        // or turn one row into a parse failure.
        let directory = "64500\tUS\tEXAMPLE-ONE\tExample Networks";
        let table = AsnTable::parse(RANGES, directory).unwrap();
        assert_eq!(
            table.lookup(ip("192.0.2.9")).unwrap().name,
            "EXAMPLE-ONE\tExample Networks"
        );
    }

    #[test]
    fn surrounding_whitespace_is_not_part_of_a_name() {
        let table = AsnTable::parse(RANGES, "64500\tUS\t  EXAMPLE-ONE  ").unwrap();
        assert_eq!(table.lookup(ip("192.0.2.9")).unwrap().name, "EXAMPLE-ONE");
    }

    #[test]
    fn names_stay_separate_in_the_shared_arena() {
        // Every name lives in one string; a wrong offset or length would hand back a
        // neighbour's name, which is exactly the failure a user could never detect.
        let directory = "64500\tUS\tFIRST\n64501\tFR\tSECOND\n64502\tDE\tTHIRD";
        let table = AsnTable::parse(RANGES, directory).unwrap();
        assert_eq!(table.lookup(ip("192.0.2.9")).unwrap().name, "FIRST");
        assert_eq!(table.lookup(ip("198.51.100.1")).unwrap().name, "SECOND");
        assert_eq!(table.lookup(ip("203.0.113.1")).unwrap().name, "THIRD");
    }

    #[test]
    fn a_non_ascii_name_survives_the_arena() {
        // Offsets are byte offsets, so a multi-byte name is where a slicing mistake would
        // surface first.
        let table = AsnTable::parse(RANGES, "64500\tRU\tПример Телеком").unwrap();
        assert_eq!(
            table.lookup(ip("192.0.2.9")).unwrap().name,
            "Пример Телеком"
        );
    }

    #[test]
    fn country_codes_are_two_letters_uppercased() {
        assert_eq!(CountryCode::parse("de").unwrap().as_str(), "DE");
        assert_eq!(CountryCode::parse("US").unwrap().as_str(), "US");
        for raw in ["", "U", "USA", "None", "1A", "u5", " U"] {
            assert!(CountryCode::parse(raw).is_none(), "{raw:?}");
        }
    }

    #[test]
    fn a_rejected_row_is_quoted_short_enough_to_read() {
        let long = format!("192.0.2.0\t{}\t64500", "x".repeat(500));
        let error = AsnTable::parse(&long, DIRECTORY).unwrap_err();
        let Error::MalformedAsnRow { raw, .. } = error else {
            panic!("expected a malformed-row error");
        };
        assert!(
            raw.chars().count() <= 81,
            "quoted {} chars",
            raw.chars().count()
        );
        assert!(raw.ends_with('…'));
    }

    #[test]
    fn the_table_reports_what_it_holds() {
        let table = table();
        assert_eq!(table.range_count(), 4);
        assert_eq!(table.organisation_count(), 3);
        assert!(!table.is_empty());
        assert!(
            table.approximate_heap_bytes() > 0,
            "the setting states this figure to the user, so it may not be a stub"
        );
    }

    #[test]
    fn reserving_capacity_changes_nothing_about_the_answers() {
        let mut builder = AsnTableBuilder::with_capacity(4, 3);
        for line in RANGES.lines() {
            builder.push_range(line).unwrap();
        }
        for line in DIRECTORY.lines() {
            builder.push_organisation(line).unwrap();
        }
        let table = builder.finish();
        assert_eq!(table.range_count(), 4);
        assert_eq!(
            table.lookup(ip("192.0.2.9")).unwrap().name,
            "EXAMPLE-ONE Example Networks, Inc."
        );
    }
}
