//! The status-page service list, and turning it into things the probe engine can address.
//!
//! The status page answers one question: *is it them or me*. A user whose game will not
//! connect wants to know whether the platform behind it is reachable from where they are
//! sitting, before they start blaming their own line — and, under censorship, whether it is
//! reachable at all.
//!
//! Like the baselines, the list is data (`assets/targets/services.json`) compiled into the
//! binary, so the app never fetches it and cannot be made to.
//! `assets/targets/README.md` documents the schema and the rules for adding an entry.
//!
//! # What a check does and does not claim
//!
//! A check reaches an operator's published front door. That is a fact about **the path from
//! this machine to that host**, and it is stated as one: a service the app cannot reach may
//! be perfectly healthy for everyone else, which is exactly what a user in a filtered
//! network needs to be able to tell. Nothing here says a company's service is down.

use nm_core::target::TargetAddress;
use nm_probes::probe::ProbeKind;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::baselines;
use crate::Error;

/// The schema version this build understands.
const SUPPORTED_SCHEMA: u32 = 1;

/// The bundled service list.
const SERVICES_JSON: &str = include_str!("../../../../assets/targets/services.json");

/// Which shelf of the status page a service sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ServiceGroup {
    /// A platform a player signs in to and buys or launches games through.
    GamingPlatform,
    /// Infrastructure the platforms and the games themselves are hosted on.
    ///
    /// Worth separating because the two fail differently and mean different things: one
    /// storefront being unreachable is that storefront's problem, while three clouds going
    /// quiet at once is the user's route out.
    Infrastructure,
}

impl ServiceGroup {
    /// Both shelves, in the order the page shows them.
    pub const ALL: [Self; 2] = [Self::GamingPlatform, Self::Infrastructure];
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

impl From<ProbeKindHint> for ProbeKind {
    fn from(hint: ProbeKindHint) -> Self {
        match hint {
            ProbeKindHint::IcmpEcho => Self::IcmpEcho,
            ProbeKindHint::TcpConnect => Self::TcpConnect,
            ProbeKindHint::TlsHello => Self::TlsHello,
        }
    }
}

/// One endpoint of a service, exactly as written in the list file.
///
/// It carries no label of its own, unlike a baseline entry. A baseline's label is the
/// operator's proper noun and the only thing that would tell one anycast address from
/// another; a service endpoint already sits under the operator's name on the card, and the
/// written address — `store.steampowered.com` beside `api.steampowered.com` — says what it
/// is better than any word we could put there, in every language, without a translation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedEndpoint {
    /// Stable identifier, unique within its service.
    pub id: String,
    /// An IP literal or, almost always, a host name.
    pub address: String,
    /// Port to use for the probe kinds that need one.
    pub port: Option<u16>,
}

/// One service exactly as written in the list file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListedService {
    /// Stable identifier, unique in the file. A React key, and the tag a verdict names.
    pub id: String,
    /// The operator's own name for the service. A proper noun: shown as written, never
    /// translated.
    pub label: String,
    /// Which shelf it sits on.
    pub group: ServiceGroup,
    /// Which probe kind to try first, when the list has an opinion.
    pub probe_kind: Option<ProbeKindHint>,
    /// Its endpoints. More than one is normal — a storefront and a gateway fail apart.
    pub endpoints: Vec<ListedEndpoint>,
}

/// A parsed and validated service list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceList {
    /// Schema version of the file.
    pub schema_version: u32,
    /// The services.
    pub services: Vec<ListedService>,
}

impl ServiceList {
    /// Parses and validates a service list.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetList`] for malformed JSON, an unsupported schema version, an
    /// empty list, a service with no endpoints, or duplicate identifiers at either level. A
    /// half-loaded status page would show missing services as absent rather than as
    /// unchecked, so every one of these is fatal to the list rather than survivable.
    pub fn parse(json: &str) -> Result<Self, Error> {
        let complain = |reason: String| Error::TargetList {
            list: "services".to_owned(),
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
        if list.services.is_empty() {
            return Err(complain("the list has no services".to_owned()));
        }
        for (index, service) in list.services.iter().enumerate() {
            if list.services[..index]
                .iter()
                .any(|seen| seen.id == service.id)
            {
                return Err(complain(format!("duplicate service id {:?}", service.id)));
            }
            if service.endpoints.is_empty() {
                return Err(complain(format!(
                    "service {:?} has no endpoints",
                    service.id
                )));
            }
            for (at, endpoint) in service.endpoints.iter().enumerate() {
                if service.endpoints[..at]
                    .iter()
                    .any(|seen| seen.id == endpoint.id)
                {
                    return Err(complain(format!(
                        "duplicate endpoint id {:?} in service {:?}",
                        endpoint.id, service.id
                    )));
                }
            }
        }

        Ok(list)
    }

    /// Loads the bundled list.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetList`] when the bundled file does not validate, which a test
    /// makes sure cannot reach a release.
    pub fn bundled() -> Result<Self, Error> {
        Self::parse(SERVICES_JSON)
    }
}

/// One endpoint of a service, ready to be probed or honestly marked as not resolvable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEndpoint {
    /// Identifier unique across the whole list, for the UI to key on.
    pub key: String,
    /// The address exactly as the list file wrote it, which is what the page shows: a name
    /// is what the user recognises, and the address behind it changes by the hour on a CDN.
    pub written_address: String,
    /// Where it actually lives, once resolved. [`None`] means the name did not resolve.
    pub address: Option<TargetAddress>,
}

/// A service and its endpoints, ready to be probed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedService {
    /// Its identifier from the list.
    pub id: String,
    /// The operator's name for it.
    pub label: String,
    /// Which shelf it sits on.
    pub group: ServiceGroup,
    /// Which probe kind to open with, if the list said.
    pub probe_kind: Option<ProbeKind>,
    /// Its endpoints, in list order.
    pub endpoints: Vec<ServiceEndpoint>,
}

/// Resolves every service of a list into something the probe engine can address.
///
/// Names, not literals, is the norm here and deliberately so: a platform's front door lives
/// on a content network whose address depends on where the user is, and pinning one address
/// in a bundled file would measure whichever edge the *developer* was nearest. A name that
/// does not resolve comes back with [`ServiceEndpoint::address`] as [`None`] rather than
/// vanishing — under censorship a poisoned or blocked lookup is itself the finding, and a
/// service that quietly disappeared from the page would hide it.
pub async fn resolve_list(list: &ServiceList) -> Vec<ResolvedService> {
    let mut resolved = Vec::with_capacity(list.services.len());
    for service in &list.services {
        let mut endpoints = Vec::with_capacity(service.endpoints.len());
        for listed in &service.endpoints {
            endpoints.push(ServiceEndpoint {
                key: format!("{}/{}", service.id, listed.id),
                written_address: listed.address.clone(),
                address: baselines::resolve_written(&listed.address, listed.port).await,
            });
        }
        resolved.push(ResolvedService {
            id: service.id.clone(),
            label: service.label.clone(),
            group: service.group,
            probe_kind: service.probe_kind.map(ProbeKind::from),
            endpoints,
        });
    }
    resolved
}
