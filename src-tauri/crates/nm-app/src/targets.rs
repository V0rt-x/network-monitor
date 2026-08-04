//! One inventory of everything the Network page measures.
//!
//! There used to be two: a *baseline* schema (`domestic/<country>.json`, `foreign.json`) and
//! a *service* schema (`services.json`), parsed by two modules, resolved by two functions
//! and rendered by two sets of components. The consequence was not merely duplication —
//! two of the four foreign baseline entries, `discord.com` and `api.steampowered.com`, were
//! literally the same addresses as two service endpoints. The same address was probed twice
//! and drawn twice, in two visual languages, under two names, spending the probe budget
//! twice for one fact.
//!
//! **A baseline is a tag, not a list.** "Domestic baseline" and "foreign baseline" are roles
//! a target plays — which is exactly why two of them turned out to be copies of service
//! entries — so there is one schema, and a target says which [`Section`] of the page it
//! belongs to. The two sections the verdict reads say so on the page.
//!
//! The lists are data (`assets/targets/`), compiled into the binary so the app never fetches
//! them and never phones home for them. `assets/targets/README.md` documents the schema and
//! the rules for adding an entry.
//!
//! # Names, not only addresses
//!
//! An entry may be a host name, resolved through the system resolver when monitoring starts.
//! This is not a convenience: public resolvers are anycast, so `1.1.1.1` measured from inside
//! a censored country usually terminates inside it and says nothing about the border. A name
//! belonging to a service actually hosted abroad resolves to an address on the far side of
//! it. A name that will not resolve is reported as unresolved rather than dropped — a
//! foreign baseline that quietly shrank to nothing would look like good news.

use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Duration;

use nm_core::target::TargetAddress;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Error;

/// The schema version this build understands.
///
/// Two, because version one was the pair of schemas this module replaced.
const SUPPORTED_SCHEMA: u32 = 2;

/// Port assumed for a host name with no port of its own.
///
/// Every bundled entry names one; this only covers a hand-edited file that forgot.
const DEFAULT_PORT: u16 = 443;

/// The bundled list of everything that is not a domestic baseline.
const FOREIGN_JSON: &str = include_str!("../../../../assets/targets/foreign.json");

/// The bundled service list — the gaming platforms and the infrastructure sections.
const SERVICES_JSON: &str = include_str!("../../../../assets/targets/services.json");

/// The bundled domestic lists, by country.
///
/// A new country is a new file and one line here — data, not a code path.
const DOMESTIC_JSON: &[(&str, &str)] = &[
    (
        "ru",
        include_str!("../../../../assets/targets/domestic/ru.json"),
    ),
    (
        "ir",
        include_str!("../../../../assets/targets/domestic/ir.json"),
    ),
];

/// Which section of the Network page a target is listed under.
///
/// One list, five sections, in the order [`Section::ALL`] states them. The first two are the
/// verdict's own evidence — [`Section::read_by_verdict`] — and since Phase 6.8 item 20 they
/// are drawn inside the verdict banner's own expander rather than as headings on the page:
/// they are not the user's services, and moving them one level down is what makes the
/// evidence one click from the claim it supports rather than a second inventory beside it.
/// The remaining three ([`Section::editable`]) are the tiles the page itself now shows, and
/// the only ones an edit chooser may offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    /// Expected to be reachable inside the user's country.
    Domestic,
    /// Typically degraded or blocked at the country's border.
    Foreign,
    /// A platform a player signs in to and buys or launches games through.
    GamingPlatform,
    /// Infrastructure the platforms and the games themselves are hosted on.
    ///
    /// Worth separating because the two fail differently and mean different things: one
    /// storefront being unreachable is that storefront's problem, while three clouds going
    /// quiet at once is the user's route out.
    Infrastructure,
    /// A service worth watching that is neither a storefront nor cloud infrastructure.
    Other,
}

impl Section {
    /// Every section, in the order the page shows them.
    pub const ALL: [Self; 5] = [
        Self::Domestic,
        Self::Foreign,
        Self::GamingPlatform,
        Self::Infrastructure,
        Self::Other,
    ];

    /// Whether a verdict is drawn from this section.
    ///
    /// The comparison between the first two is the whole diagnosis — it is what separates
    /// "my provider is broken" from "the way out of the country is" — and the page marks
    /// them so the verdict banner's own expander can show exactly this evidence and nothing
    /// else.
    #[must_use]
    pub const fn read_by_verdict(self) -> bool {
        matches!(self, Self::Domestic | Self::Foreign)
    }

    /// How a target in this section is judged.
    ///
    /// **The measurement layer does not merge, and this is where that is stated.** A
    /// baseline asks what the last several minutes have been like, which is a window; a
    /// platform asks whether it is reachable *now*, which at a check every forty-odd seconds
    /// a window answers badly at both ends — a service that died a minute ago still reads
    /// mostly green, and one that has just recovered still reads mostly red. One rule across
    /// both would be exactly the smoothing this product forbids.
    #[must_use]
    pub const fn judged_by_window(self) -> bool {
        self.read_by_verdict()
    }

    /// What the shared target registry knows this section's members as.
    ///
    /// The registry's tags stay as they were: an address that is *both* a baseline and an
    /// application's endpoint is one target whose single measurement answers for both, and
    /// the tag is what records which purposes asked for it.
    #[must_use]
    pub const fn tag(self) -> nm_core::target::TargetTag {
        match self {
            Self::Domestic => nm_core::target::TargetTag::DomesticBaseline,
            Self::Foreign => nm_core::target::TargetTag::ForeignBaseline,
            Self::GamingPlatform | Self::Infrastructure | Self::Other => {
                nm_core::target::TargetTag::StatusService
            }
        }
    }

    /// Whether an edit chooser may offer this section.
    ///
    /// `Domestic` and `Foreign` are the verdict's own evidence, not the user's services —
    /// item 20's rule is that editing changes what is *shown*, never what is *measured* for
    /// the verdict, so those two never appear in a catalogue a user could untick.
    #[must_use]
    pub const fn editable(self) -> bool {
        !self.read_by_verdict()
    }
}

/// One address of a target.
///
/// It carries no label of its own. A target's label is the operator's proper noun and the
/// only thing that would tell one anycast address from another; an endpoint already sits
/// under that name, and the written address — `store.steampowered.com` beside
/// `api.steampowered.com` — says what it is better than any word we could put there, in
/// every language, without a translation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedEndpoint {
    /// Stable identifier, unique within its target.
    pub id: String,
    /// An IP literal or a host name.
    pub address: String,
    /// Port to use for the probe kinds that need one.
    pub port: Option<u16>,
}

/// One entry exactly as written in a list file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListedTarget {
    /// Stable identifier, unique within its file.
    pub id: String,
    /// The operator's own name for it. A proper noun: shown as written, never translated.
    pub label: String,
    /// Which section it is listed under, when the file does not say for all of its entries.
    pub section: Option<Section>,
    /// Which probe kind to try first, when the list has an opinion.
    pub probe_kind: Option<ProbeKindHint>,
    /// Its addresses. More than one is normal — a storefront and a gateway fail apart.
    pub endpoints: Vec<ListedEndpoint>,
}

/// A parsed and validated list file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetList {
    /// Schema version of the file.
    pub schema_version: u32,
    /// Identifier of the list — a country code, `foreign`, or `services`.
    pub id: String,
    /// The section every entry belongs to, where a whole file shares one.
    pub section: Option<Section>,
    /// The entries.
    pub targets: Vec<ListedTarget>,
}

impl TargetList {
    /// Parses and validates a list, checking it really is the list that was asked for.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetList`] for malformed JSON, an unsupported schema version, a
    /// mismatched identifier, an empty list, a target with no endpoints, duplicate
    /// identifiers at either level, or an entry whose section neither it nor its file
    /// states. A silently half-loaded list would show as healthy, so every one of these is
    /// fatal to the list rather than survivable.
    pub fn parse(expected_id: &str, json: &str) -> Result<Self, Error> {
        let complain = |reason: String| Error::TargetList {
            list: expected_id.to_owned(),
            reason,
        };

        let list: Self =
            serde_json::from_str(json).map_err(|source| complain(source.to_string()))?;

        if list.schema_version != SUPPORTED_SCHEMA {
            return Err(complain(format!(
                "schema version {} is not supported (this build understands {SUPPORTED_SCHEMA})",
                list.schema_version
            )));
        }
        if list.id != expected_id {
            return Err(complain(format!(
                "the file declares id {:?} but was loaded as {expected_id:?}",
                list.id
            )));
        }
        if list.targets.is_empty() {
            return Err(complain("the list has no targets".to_owned()));
        }

        for (index, target) in list.targets.iter().enumerate() {
            if list.targets[..index]
                .iter()
                .any(|seen| seen.id == target.id)
            {
                return Err(complain(format!("duplicate target id {:?}", target.id)));
            }
            if target.section.or(list.section).is_none() {
                return Err(complain(format!(
                    "target {:?} names no section and the file states none",
                    target.id
                )));
            }
            if target.endpoints.is_empty() {
                return Err(complain(format!("target {:?} has no endpoints", target.id)));
            }
            for (at, endpoint) in target.endpoints.iter().enumerate() {
                if target.endpoints[..at]
                    .iter()
                    .any(|seen| seen.id == endpoint.id)
                {
                    return Err(complain(format!(
                        "duplicate endpoint id {:?} in target {:?}",
                        endpoint.id, target.id
                    )));
                }
            }
        }

        Ok(list)
    }

    /// Every written address in the file, in order.
    ///
    /// Exists for the rule that no address may be measured twice across the whole bundled
    /// inventory — the failure this module was written to end.
    pub fn written_addresses(&self) -> impl Iterator<Item = &str> + '_ {
        self.targets
            .iter()
            .flat_map(|target| target.endpoints.iter())
            .map(|endpoint| endpoint.address.as_str())
    }
}

/// Which probe kind a list asks to be tried first.
///
/// A *hint*, never a permission: it reorders the kinds
/// [`nm_probes::probe::preferred_kinds`] already judged honest for the address, and can
/// never introduce one that address class refuses. A hand-edited list can therefore shorten
/// a wait and cannot make the engine report a figure a tunnel invented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeKindHint {
    /// An ICMP echo.
    IcmpEcho,
    /// A bare TCP connection attempt.
    TcpConnect,
    /// A TLS `ClientHello`, timed to the first answering byte.
    TlsHello,
}

impl From<ProbeKindHint> for nm_probes::probe::ProbeKind {
    fn from(hint: ProbeKindHint) -> Self {
        match hint {
            ProbeKindHint::TcpConnect => Self::TcpConnect,
            ProbeKindHint::TlsHello => Self::TlsHello,
            ProbeKindHint::IcmpEcho => Self::IcmpEcho,
        }
    }
}

/// Country codes with a bundled domestic list, in file order.
#[must_use]
pub fn countries() -> Vec<&'static str> {
    DOMESTIC_JSON.iter().map(|(code, _)| *code).collect()
}

/// Whether a country code has a bundled domestic list.
#[must_use]
pub fn has_country(code: &str) -> bool {
    DOMESTIC_JSON.iter().any(|(known, _)| *known == code)
}

/// Loads the domestic list for a country.
///
/// # Errors
///
/// Returns [`Error::UnknownCountry`] when nothing is bundled for the code, or
/// [`Error::TargetList`] when the bundled file does not validate.
pub fn domestic(country: &str) -> Result<TargetList, Error> {
    let (_, json) = DOMESTIC_JSON
        .iter()
        .find(|(code, _)| *code == country)
        .ok_or_else(|| Error::UnknownCountry {
            country: country.to_owned(),
        })?;
    TargetList::parse(country, json)
}

/// Loads the foreign list.
///
/// # Errors
///
/// Returns [`Error::TargetList`] when the bundled file does not validate.
pub fn foreign() -> Result<TargetList, Error> {
    TargetList::parse("foreign", FOREIGN_JSON)
}

/// Loads the gaming-platform and infrastructure list.
///
/// # Errors
///
/// Returns [`Error::TargetList`] when the bundled file does not validate.
pub fn services() -> Result<TargetList, Error> {
    TargetList::parse("services", SERVICES_JSON)
}

/// One entry an edit chooser may offer, over the bundled catalogue only.
///
/// No free-text entry exists anywhere in this product: an address a user typed would be a
/// target this app then probed on their behalf, and the bundled lists are auditable
/// precisely because they are the only thing that is ever probed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEntry {
    /// Keyed exactly as [`ResolvedTarget::key`], so a stored selection lines up with the
    /// rows it selects without a translation step.
    pub key: String,
    /// The operator's name for it, shown as written.
    pub label: String,
    /// Which editable section it belongs to.
    pub section: Section,
}

/// Every entry an edit chooser may offer: the services list's targets, grouped by section.
///
/// `Domestic` and `Foreign` never appear here — see [`Section::editable`] — because they are
/// the verdict's own evidence rather than a user's services.
///
/// # Errors
///
/// Returns [`Error::TargetList`] if the bundled `services.json` does not validate.
pub fn catalogue() -> Result<Vec<CatalogueEntry>, Error> {
    let list = services()?;
    Ok(list
        .targets
        .iter()
        .map(|target| CatalogueEntry {
            key: format!("{}/{}", list.id, target.id),
            label: target.label.clone(),
            section: target
                .section
                .or(list.section)
                .unwrap_or(Section::Infrastructure),
        })
        .collect())
}

/// Every bundled list for one country, in the order the page shows their sections.
///
/// # Errors
///
/// Returns [`Error::UnknownCountry`] or [`Error::TargetList`] exactly as the loaders above,
/// and [`Error::TargetList`] when one address appears in two entries — see
/// [`refuse_duplicate_addresses`].
pub fn bundled(country: &str) -> Result<Vec<TargetList>, Error> {
    let lists = vec![domestic(country)?, foreign()?, services()?];
    refuse_duplicate_addresses(&lists)?;
    Ok(lists)
}

/// Refuses an inventory that would measure one address twice.
///
/// The failure this module exists to end, kept out by a check rather than by care: half of
/// the foreign baseline used to be a second probe of a row already on the page, in another
/// visual language and under another name, spending the budget twice for one fact. It is a
/// hard error rather than a deduplication, because which of the two entries the reader is
/// meant to see is a question only a person can answer.
///
/// # Errors
///
/// Returns [`Error::TargetList`] naming the address and the entries that share it.
pub fn refuse_duplicate_addresses(lists: &[TargetList]) -> Result<(), Error> {
    let mut seen: HashSet<&str> = HashSet::new();
    for list in lists {
        for target in &list.targets {
            for endpoint in &target.endpoints {
                if !seen.insert(endpoint.address.as_str()) {
                    return Err(Error::TargetList {
                        list: list.id.clone(),
                        reason: format!(
                            "address {:?} is already measured by another entry; a baseline is a \
                             tag on a target, not a second copy of it",
                            endpoint.address
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

/// One address of a target, ready to be probed or honestly marked as not resolvable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEndpoint {
    /// Identifier unique across the whole inventory, for the UI to key on.
    pub key: String,
    /// The address exactly as the list file wrote it, which is what the page shows: a name
    /// is what the user recognises, and the address behind it changes by the hour on a CDN.
    pub written_address: String,
    /// Where it actually lives, once resolved. [`None`] means the name did not resolve.
    pub address: Option<TargetAddress>,
}

/// A target and its endpoints, ready to be probed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    /// Identifier unique across the whole inventory.
    pub key: String,
    /// The operator's name for it.
    pub label: String,
    /// Which section it is listed under.
    pub section: Section,
    /// Which probe kind to open with, if the list said.
    pub probe_kind: Option<nm_probes::probe::ProbeKind>,
    /// Its endpoints, in list order.
    pub endpoints: Vec<ResolvedEndpoint>,
}

/// Resolves every entry of a list into something the probe engine can address.
///
/// Entries that are already IP literals cost no lookup. A name is resolved through the
/// system resolver — the same call any application on the machine makes — and one that fails
/// comes back with [`ResolvedEndpoint::address`] as [`None`] rather than vanishing: under
/// censorship a poisoned or blocked lookup is itself the finding, and a row that quietly
/// disappeared from the page would hide it.
pub async fn resolve_list(list: &TargetList) -> Vec<ResolvedTarget> {
    let mut resolved = Vec::with_capacity(list.targets.len());
    for target in &list.targets {
        // Validated at parse time; the fallback keeps this total rather than papering over
        // anything, since a list that reached here has a section for every entry.
        let section = target
            .section
            .or(list.section)
            .unwrap_or(Section::Infrastructure);

        let mut endpoints = Vec::with_capacity(target.endpoints.len());
        for listed in &target.endpoints {
            endpoints.push(ResolvedEndpoint {
                key: format!("{}/{}/{}", list.id, target.id, listed.id),
                written_address: listed.address.clone(),
                address: resolve_written(&listed.address, listed.port).await,
            });
        }

        resolved.push(ResolvedTarget {
            key: format!("{}/{}", list.id, target.id),
            label: target.label.clone(),
            section,
            probe_kind: target.probe_kind.map(Into::into),
            endpoints,
        });
    }
    resolved
}

/// Turns a written address into a probe target, resolving a name if that is what it is.
pub async fn resolve_written(written: &str, port: Option<u16>) -> Option<TargetAddress> {
    match written.parse::<IpAddr>() {
        Ok(ip) => Some(address_with_port(ip, port)),
        Err(_) => lookup(written, port).await,
    }
}

/// Pairs an address with the entry's port, if it has one.
fn address_with_port(ip: IpAddr, port: Option<u16>) -> TargetAddress {
    match port {
        Some(port) => TargetAddress::with_port(ip, port),
        None => TargetAddress::icmp(ip),
    }
}

/// Looks a host name up, preferring IPv4.
///
/// IPv4 first because the probe kinds behind it are the ones proven on this product's target
/// networks, and because an IPv6 answer on a host with no working IPv6 route would turn a
/// healthy service into a fabricated outage.
async fn lookup(host: &str, port: Option<u16>) -> Option<TargetAddress> {
    let addresses: Vec<IpAddr> = tokio::net::lookup_host((host, port.unwrap_or(DEFAULT_PORT)))
        .await
        .ok()?
        .map(|socket| socket.ip())
        .collect();

    let chosen = addresses
        .iter()
        .find(|ip| ip.is_ipv4())
        .or_else(|| addresses.first())?;
    Some(address_with_port(*chosen, port))
}

/// How often a target in each section is probed.
///
/// **The cadence difference is a number, not an architecture.** It used to be two
/// subsystems with two runners' worth of wiring; it is one field. The sections a verdict
/// reads are probed at the user's baseline interval, because a verdict that waited three
/// quarters of a minute for half its evidence would be answering a question the user asked
/// a minute ago. The rest are checked slowly: whether a platform is up changes on the scale
/// of minutes, and that list is probed whether or not the user is doing anything, so it must
/// cost close to nothing.
#[must_use]
pub fn interval_for(section: Section, baseline: Duration) -> Duration {
    if section.read_by_verdict() {
        baseline
    } else {
        SLOW_CHECK_INTERVAL
    }
}

/// How often a target no verdict reads is checked.
pub const SLOW_CHECK_INTERVAL: Duration = Duration::from_secs(45);

/// Whether a resolved target is measured this session, given the user's edit to the catalogue.
///
/// **The rule that must hold: editing changes what is *shown*, never what is *measured* for
/// the verdict.** A target the diagnosis reads is probed and reported whether or not the
/// user's selection includes it, so unticking a tile can never thin the sample the verdict is
/// drawn from — `Domestic` and `Foreign` bypass the selection entirely rather than merely
/// being defaulted into every one. Everything else costs real probe budget only when it is
/// actually wanted: an entry the user did not tick is not registered at all, which is what
/// lets ticking *fewer* buy the rest a shorter cadence rather than spending the same traffic
/// on rows nobody asked to see.
#[must_use]
pub fn is_selected(target: &ResolvedTarget, selection: &Option<Vec<String>>) -> bool {
    if target.section.read_by_verdict() {
        return true;
    }
    match selection {
        None => true,
        Some(keys) => keys.iter().any(|key| key == &target.key),
    }
}
