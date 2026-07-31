//! The baseline target lists, and turning them into things the probe engine can address.
//!
//! Two lists are monitored at once: a *domestic* one, chosen by country, holding services
//! expected to work inside it, and a *foreign* one holding services typically degraded or
//! blocked at a border. Comparing the two is the whole point — it is what separates "my
//! provider is broken" from "the way out of the country is".
//!
//! The lists are data (`assets/targets/`), compiled into the binary so the app never
//! fetches them and never phones home for them. `assets/targets/README.md` documents the
//! schema and the rules for adding an entry.
//!
//! # Names, not only addresses
//!
//! An entry may be a host name, resolved through the system resolver when monitoring
//! starts. This is not a convenience: public resolvers are anycast, so `1.1.1.1` measured
//! from inside a censored country usually terminates inside it and says nothing about the
//! border. A name belonging to a service actually hosted abroad resolves to an address on
//! the far side of it. A name that will not resolve is reported as unresolved rather than
//! dropped — a foreign baseline that quietly shrank to nothing would look like good news.

use std::net::IpAddr;

use nm_core::target::TargetAddress;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Error;

/// The schema version this build understands.
const SUPPORTED_SCHEMA: u32 = 1;

/// Port assumed for a host name with no port of its own.
///
/// Every bundled entry names one; this only covers a hand-edited file that forgot.
const DEFAULT_PORT: u16 = 443;

/// The bundled foreign list.
const FOREIGN_JSON: &str = include_str!("../../../../assets/targets/foreign.json");

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

/// Which baseline a target belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BaselineGroup {
    /// Expected to be reachable inside the user's country.
    Domestic,
    /// Typically degraded or blocked at the country's border.
    Foreign,
}

impl BaselineGroup {
    /// Both groups, in the order the dashboard shows them.
    pub const ALL: [Self; 2] = [Self::Domestic, Self::Foreign];
}

/// One entry exactly as written in a list file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedTarget {
    /// Stable identifier, unique within its list.
    pub id: String,
    /// Operator's own name for the service. A proper noun: shown as written, not
    /// translated.
    pub label: String,
    /// An IP literal or a host name.
    pub address: String,
    /// Port to use for the probe kinds that need one.
    pub port: Option<u16>,
}

/// A parsed and validated list file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetList {
    /// Schema version of the file.
    pub schema_version: u32,
    /// Identifier of the list — a country code, or `foreign`.
    pub id: String,
    /// The entries.
    pub targets: Vec<ListedTarget>,
}

impl TargetList {
    /// Parses and validates a list, checking it really is the list that was asked for.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetList`] for malformed JSON, an unsupported schema version, a
    /// mismatched identifier, an empty list or duplicate entry identifiers. A silently
    /// half-loaded baseline would show as healthy, so every one of these is fatal to the
    /// list rather than survivable.
    pub fn parse(expected_id: &str, json: &str) -> Result<Self, Error> {
        let list: Self = serde_json::from_str(json).map_err(|source| Error::TargetList {
            list: expected_id.to_owned(),
            reason: source.to_string(),
        })?;

        let complain = |reason: String| Error::TargetList {
            list: expected_id.to_owned(),
            reason,
        };

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
        }

        Ok(list)
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

/// A baseline entry ready to be probed, or honestly marked as not resolvable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineTarget {
    /// Which baseline it belongs to.
    pub group: BaselineGroup,
    /// Identifier unique across both lists, for the UI to key on.
    pub key: String,
    /// The operator's name for the service.
    pub label: String,
    /// The address exactly as the list file wrote it.
    pub written_address: String,
    /// Where it actually lives, once resolved. [`None`] means the name did not resolve.
    pub address: Option<TargetAddress>,
}

impl BaselineTarget {
    /// Builds an unresolved entry from a list.
    fn pending(group: BaselineGroup, list_id: &str, listed: &ListedTarget) -> Self {
        Self {
            group,
            key: format!("{list_id}/{}", listed.id),
            label: listed.label.clone(),
            written_address: listed.address.clone(),
            address: None,
        }
    }
}

/// The address an entry names, when it is already an IP literal.
///
/// Kept separate from name resolution so the common case — and the decision of whether a
/// lookup is needed at all — is a pure function with tests, and so an entry that is
/// already an address never causes a DNS query.
#[must_use]
pub fn literal_address(listed: &ListedTarget) -> Option<TargetAddress> {
    let ip: IpAddr = listed.address.parse().ok()?;
    Some(address_with_port(ip, listed.port))
}

/// Pairs an address with the entry's port, if it has one.
fn address_with_port(ip: IpAddr, port: Option<u16>) -> TargetAddress {
    match port {
        Some(port) => TargetAddress::with_port(ip, port),
        None => TargetAddress::icmp(ip),
    }
}

/// Resolves every entry of a list into something the probe engine can address.
///
/// Entries that are already IP literals cost no lookup. A name is resolved through the
/// system resolver — the same call any application on the machine makes — and one that
/// fails comes back with [`BaselineTarget::address`] as [`None`] rather than vanishing.
pub async fn resolve_list(group: BaselineGroup, list: &TargetList) -> Vec<BaselineTarget> {
    let mut resolved = Vec::with_capacity(list.targets.len());
    for listed in &list.targets {
        let mut target = BaselineTarget::pending(group, &list.id, listed);
        target.address = match literal_address(listed) {
            Some(address) => Some(address),
            None => lookup(&listed.address, listed.port).await,
        };
        resolved.push(target);
    }
    resolved
}

/// Looks a host name up, preferring IPv4.
///
/// IPv4 first because the probe kinds behind it are the ones proven on this product's
/// target networks, and because an IPv6 answer on a host with no working IPv6 route would
/// turn a healthy service into a fabricated outage.
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
