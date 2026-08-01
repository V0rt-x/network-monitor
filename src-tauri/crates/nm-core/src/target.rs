//! Registry of everything the app probes.
//!
//! A target is an address the probe engine may send to. The same address can be reached
//! for more than one reason — a game server that is also on a baseline list, say — so a
//! target carries a *set* of tags rather than a single role. That is what lets the
//! registry deduplicate: one address is probed once, and its results feed every feature
//! that asked for it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::IpAddr;

use crate::Error;

/// Handle for a registered target.
///
/// Opaque and stable for as long as the target stays registered. Identifiers are never
/// reused within a session, so a stale handle resolves to [`None`] instead of silently
/// pointing at a different host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetId(u32);

impl TargetId {
    /// The raw identifier, for logging and debugging.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Fabricates a handle so sibling modules can be tested without a registry.
    ///
    /// Test-only on purpose: handles must otherwise come from [`TargetRegistry::insert`],
    /// which is what guarantees they map to a real address.
    #[cfg(test)]
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// Why a target is being probed.
///
/// Tags are the seam that keeps features from needing their own probe pipelines: a new
/// consumer of the probe engine adds a tag, not a code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TargetTag {
    /// An endpoint discovered belonging to an application the user monitors.
    AppEndpoint,
    /// A router on the way to an endpoint that answers nothing.
    ///
    /// Probed as a stand-in for a destination that cannot be measured directly — see
    /// [`crate::edge`]. It is a tag rather than a separate pipeline because the address that
    /// happens to be a game's path edge can equally be a baseline of its own, and then it is
    /// probed once and answers both.
    PathEdgeHop,
    /// A service known to be reachable inside the user's country.
    DomesticBaseline,
    /// A service typically degraded or blocked at the country's border.
    ForeignBaseline,
    /// An entry on the service status page.
    StatusService,
    /// A reference target of a monitored game's pool.
    ///
    /// Probed at a trickle to answer the one question no single endpoint can: whether the
    /// *game's* infrastructure is reachable, as distinct from the path to any one server.
    /// A tag rather than a pipeline of its own, because a pool member can equally be an
    /// endpoint the game is currently playing over — and then it is probed once and answers
    /// both.
    GameReferencePool,
}

/// Where a target lives on the network.
///
/// `port` is absent for targets that are only ever reached with ICMP, which has no
/// concept of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetAddress {
    /// Remote address.
    pub ip: IpAddr,
    /// Remote port, when the probe kind has one.
    pub port: Option<u16>,
}

impl TargetAddress {
    /// An address reached without a port, as ICMP is.
    #[must_use]
    pub const fn icmp(ip: IpAddr) -> Self {
        Self { ip, port: None }
    }

    /// An address reached on a specific port.
    #[must_use]
    pub const fn with_port(ip: IpAddr, port: u16) -> Self {
        Self {
            ip,
            port: Some(port),
        }
    }
}

/// A registered target and the reasons it is probed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    id: TargetId,
    address: TargetAddress,
    tags: BTreeSet<TargetTag>,
}

impl Target {
    /// This target's handle.
    #[must_use]
    pub const fn id(&self) -> TargetId {
        self.id
    }

    /// Where the target lives.
    #[must_use]
    pub const fn address(&self) -> TargetAddress {
        self.address
    }

    /// Whether the target is probed for this reason.
    #[must_use]
    pub fn has_tag(&self, tag: TargetTag) -> bool {
        self.tags.contains(&tag)
    }

    /// Every reason this target is probed, in a stable order.
    #[must_use]
    pub fn tags(&self) -> impl ExactSizeIterator<Item = TargetTag> + '_ {
        self.tags.iter().copied()
    }
}

/// The set of targets the probe engine may address.
///
/// Iteration order follows [`TargetId`], so scheduling decisions built on top of the
/// registry are reproducible rather than dependent on hash ordering.
#[derive(Debug, Clone, Default)]
pub struct TargetRegistry {
    targets: BTreeMap<TargetId, Target>,
    by_address: HashMap<TargetAddress, TargetId>,
    next_id: u32,
}

impl TargetRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `address` as probed for `tag`, returning its handle.
    ///
    /// If the address is already registered its existing handle is returned and the tag
    /// is added to it; an address is never probed twice because two features want it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetIdExhausted`] once `u32::MAX` targets have been registered
    /// in this session.
    pub fn insert(&mut self, address: TargetAddress, tag: TargetTag) -> Result<TargetId, Error> {
        if let Some(&id) = self.by_address.get(&address) {
            if let Some(target) = self.targets.get_mut(&id) {
                target.tags.insert(tag);
            }
            return Ok(id);
        }

        let id = TargetId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(Error::TargetIdExhausted)?;

        let mut tags = BTreeSet::new();
        tags.insert(tag);
        self.targets.insert(id, Target { id, address, tags });
        self.by_address.insert(address, id);
        Ok(id)
    }

    /// Removes one reason for probing a target.
    ///
    /// When the last tag goes the target itself is removed, so a feature switching off
    /// cannot leave an orphan being probed forever. Returns `true` if the target is gone.
    pub fn untag(&mut self, id: TargetId, tag: TargetTag) -> bool {
        let Some(target) = self.targets.get_mut(&id) else {
            return false;
        };
        target.tags.remove(&tag);
        if target.tags.is_empty() {
            self.remove(id);
            return true;
        }
        false
    }

    /// Removes a target outright, whatever it was tagged with.
    ///
    /// Returns `true` if it was registered.
    pub fn remove(&mut self, id: TargetId) -> bool {
        let Some(target) = self.targets.remove(&id) else {
            return false;
        };
        self.by_address.remove(&target.address);
        true
    }

    /// Looks up a target by handle.
    #[must_use]
    pub fn get(&self, id: TargetId) -> Option<&Target> {
        self.targets.get(&id)
    }

    /// Looks up the handle registered for an address.
    #[must_use]
    pub fn find(&self, address: TargetAddress) -> Option<TargetId> {
        self.by_address.get(&address).copied()
    }

    /// Every registered target, ordered by handle.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Target> + '_ {
        self.targets.values()
    }

    /// Every target probed for a given reason, ordered by handle.
    pub fn tagged(&self, tag: TargetTag) -> impl Iterator<Item = &Target> + '_ {
        self.targets.values().filter(move |t| t.has_tag(tag))
    }

    /// How many targets are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.targets.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    #[test]
    fn a_new_registry_is_empty() {
        let registry = TargetRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert_eq!(registry.find(TargetAddress::icmp(ip(1))), None);
    }

    #[test]
    fn registers_and_resolves_a_target() {
        let mut registry = TargetRegistry::new();
        let address = TargetAddress::icmp(ip(1));
        let id = registry.insert(address, TargetTag::AppEndpoint).unwrap();

        let target = registry.get(id).unwrap();
        assert_eq!(target.id(), id);
        assert_eq!(target.address(), address);
        assert!(target.has_tag(TargetTag::AppEndpoint));
        assert_eq!(registry.find(address), Some(id));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn the_same_address_is_registered_once_and_accumulates_tags() {
        let mut registry = TargetRegistry::new();
        let address = TargetAddress::icmp(ip(1));

        let first = registry.insert(address, TargetTag::AppEndpoint).unwrap();
        let second = registry
            .insert(address, TargetTag::ForeignBaseline)
            .unwrap();

        assert_eq!(
            first, second,
            "one address must not become two probe targets"
        );
        assert_eq!(registry.len(), 1);
        let tags: Vec<_> = registry.get(first).unwrap().tags().collect();
        assert_eq!(
            tags,
            vec![TargetTag::AppEndpoint, TargetTag::ForeignBaseline]
        );
    }

    #[test]
    fn addresses_differing_only_by_port_are_distinct_targets() {
        let mut registry = TargetRegistry::new();
        let bare = registry
            .insert(TargetAddress::icmp(ip(1)), TargetTag::AppEndpoint)
            .unwrap();
        let with_port = registry
            .insert(TargetAddress::with_port(ip(1), 443), TargetTag::AppEndpoint)
            .unwrap();
        let other_port = registry
            .insert(TargetAddress::with_port(ip(1), 80), TargetTag::AppEndpoint)
            .unwrap();

        assert_ne!(bare, with_port);
        assert_ne!(with_port, other_port);
        assert_eq!(registry.len(), 3);
    }

    #[test]
    fn untagging_the_last_reason_removes_the_target() {
        let mut registry = TargetRegistry::new();
        let address = TargetAddress::icmp(ip(1));
        let id = registry.insert(address, TargetTag::StatusService).unwrap();

        assert!(registry.untag(id, TargetTag::StatusService));
        assert!(registry.is_empty());
        assert_eq!(registry.get(id), None);
        assert_eq!(
            registry.find(address),
            None,
            "the address index must not keep a dangling entry"
        );
    }

    #[test]
    fn untagging_one_of_several_reasons_keeps_the_target() {
        let mut registry = TargetRegistry::new();
        let address = TargetAddress::icmp(ip(1));
        let id = registry.insert(address, TargetTag::AppEndpoint).unwrap();
        registry
            .insert(address, TargetTag::DomesticBaseline)
            .unwrap();

        assert!(!registry.untag(id, TargetTag::AppEndpoint));
        let target = registry.get(id).unwrap();
        assert!(!target.has_tag(TargetTag::AppEndpoint));
        assert!(target.has_tag(TargetTag::DomesticBaseline));
    }

    #[test]
    fn untagging_something_that_was_never_set_is_harmless() {
        let mut registry = TargetRegistry::new();
        let id = registry
            .insert(TargetAddress::icmp(ip(1)), TargetTag::AppEndpoint)
            .unwrap();
        assert!(!registry.untag(id, TargetTag::StatusService));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn removing_reports_whether_anything_was_there() {
        let mut registry = TargetRegistry::new();
        let id = registry
            .insert(TargetAddress::icmp(ip(1)), TargetTag::AppEndpoint)
            .unwrap();
        assert!(registry.remove(id));
        assert!(!registry.remove(id));
        assert!(!registry.untag(id, TargetTag::AppEndpoint));
    }

    #[test]
    fn identifiers_are_not_reused_after_removal() {
        // A handle held by a stale scheduler entry must never start pointing at a
        // different host.
        let mut registry = TargetRegistry::new();
        let first = registry
            .insert(TargetAddress::icmp(ip(1)), TargetTag::AppEndpoint)
            .unwrap();
        registry.remove(first);
        let second = registry
            .insert(TargetAddress::icmp(ip(2)), TargetTag::AppEndpoint)
            .unwrap();

        assert_ne!(first, second);
        assert_eq!(registry.get(first), None);
    }

    #[test]
    fn re_registering_a_removed_address_yields_a_fresh_handle() {
        let mut registry = TargetRegistry::new();
        let address = TargetAddress::icmp(ip(1));
        let first = registry.insert(address, TargetTag::AppEndpoint).unwrap();
        registry.remove(first);
        let second = registry.insert(address, TargetTag::AppEndpoint).unwrap();

        assert_ne!(first, second);
        assert_eq!(registry.find(address), Some(second));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn filters_by_tag() {
        let mut registry = TargetRegistry::new();
        let app = registry
            .insert(TargetAddress::icmp(ip(1)), TargetTag::AppEndpoint)
            .unwrap();
        let baseline = registry
            .insert(TargetAddress::icmp(ip(2)), TargetTag::DomesticBaseline)
            .unwrap();
        // A shared address belongs to both groups.
        registry
            .insert(TargetAddress::icmp(ip(1)), TargetTag::DomesticBaseline)
            .unwrap();

        let app_ids: Vec<_> = registry
            .tagged(TargetTag::AppEndpoint)
            .map(Target::id)
            .collect();
        let baseline_ids: Vec<_> = registry
            .tagged(TargetTag::DomesticBaseline)
            .map(Target::id)
            .collect();

        assert_eq!(app_ids, vec![app]);
        assert_eq!(baseline_ids, vec![app, baseline]);
        assert_eq!(registry.tagged(TargetTag::StatusService).count(), 0);
    }

    #[test]
    fn iteration_follows_handle_order() {
        let mut registry = TargetRegistry::new();
        let ids: Vec<_> = (1..=5)
            .map(|last| {
                registry
                    .insert(TargetAddress::icmp(ip(last)), TargetTag::AppEndpoint)
                    .unwrap()
            })
            .collect();

        let seen: Vec<_> = registry.iter().map(Target::id).collect();
        assert_eq!(seen, ids);
    }

    #[test]
    fn refuses_to_register_once_identifiers_run_out() {
        let mut registry = TargetRegistry::new();
        registry.next_id = u32::MAX - 1;

        // The last identifier is still handed out...
        let last = registry
            .insert(TargetAddress::icmp(ip(1)), TargetTag::AppEndpoint)
            .unwrap();
        assert_eq!(last.get(), u32::MAX - 1);

        // ...and the next attempt fails instead of wrapping onto an existing handle.
        assert_eq!(
            registry
                .insert(TargetAddress::icmp(ip(2)), TargetTag::AppEndpoint)
                .unwrap_err(),
            Error::TargetIdExhausted
        );
    }
}
