//! Deciding, over time, which probe kind can actually measure one endpoint.
//!
//! A single probe cannot tell a filtered probe kind from a dead host: both produce silence.
//! The difference only appears across attempts and across kinds. If ICMP stays silent but a
//! TCP handshake to the same endpoint completes, then the host is up and echoes are being
//! dropped — and that is *proof*, not a guess. This state machine is where that inference
//! lives.
//!
//! It never invents a measurement. It decides what to try next and records what has been
//! ruled out, so the UI can say "ICMP is filtered on this path" when that has been
//! established and stay quiet when it has not.
//!
//! # Why silence has to be given a few chances
//!
//! Falling back on the first timeout would abandon ICMP over ordinary packet loss and spend
//! the rest of the session on a more expensive kind. Waiting too long leaves an endpoint
//! unmeasured while a working kind sits untried. [`SILENCE_BEFORE_FALLBACK`] consecutive
//! silences is the compromise: at the usual one-second interval that is a few seconds, which
//! moderate loss almost never produces in an unbroken run, and the cost of being wrong is
//! only that a cheaper kind is retried later.

use nm_core::address::AddressClass;
use nm_core::sample::ProbeOutcome;

use crate::probe::{preferred_kinds, ProbeKind};
use crate::Error;

/// How many consecutive silent probes make a kind suspect enough to step past.
pub const SILENCE_BEFORE_FALLBACK: u32 = 3;

/// How many consecutive refusals retire a probe kind that needs a port.
///
/// A refusal answers a different question from the one being asked. "Nothing is listening on
/// this port" is a fact about the port *we* chose, and a game's match server refusing a TCP
/// handshake on the port it plays UDP over is the normal case, not a fault — while the game
/// runs perfectly across it. Left in place, that answer keeps a kind that can never measure
/// the path occupying the endpoint forever, so nothing further is ever tried.
///
/// A run rather than a single answer, because a middlebox can reset intermittently, and
/// because the cost of waiting is a few seconds against the cost of being wrong.
///
/// It applies only to the kinds that address a port. An ICMP unreachable comes from a router
/// and is about the *destination*: every other kind would fail the same way, so stepping past
/// ICMP on one would swap a correct answer for an expensive silence.
pub const REFUSALS_BEFORE_FALLBACK: u32 = 3;

/// What to do next for an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChainStep {
    /// Probe with this kind.
    Probe(ProbeKind),
    /// Every kind has been ruled out; walk the path to learn where it stops instead.
    WalkThePath,
    /// Nothing left can say anything honest about this endpoint.
    ///
    /// Reached for an address a local tunnel remaps once the end-to-end probe has failed: a
    /// TTL walk from this machine would map the route to the tunnel, not to the destination,
    /// so offering it would be worse than admitting the gap.
    Nothing,
}

/// Why a probe kind was set aside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuledOutBecause {
    /// The probe itself reported being filtered — a reset on the hello, a local firewall.
    ItReportedFiltering,
    /// It went silent for long enough that another kind was worth trying.
    ItWentSilent,
    /// The endpoint refused it: nothing is listening on the port this kind had to use.
    ///
    /// A definitive answer, and an answer to the wrong question — see
    /// [`REFUSALS_BEFORE_FALLBACK`]. It says nothing whatever about filtering.
    ItFoundNoServiceThere,
    /// It cannot address this target at all.
    ///
    /// Nothing to do with the network: an ICMP backend with no IPv6 implementation, a
    /// connecting kind handed a target with no port. Without this the kind is retried
    /// forever and the endpoint sits there never measured, wearing the name of a probe that
    /// never ran — which is the failure mode this crate exists to avoid.
    ItCannotAddressThisTarget,
}

impl RuledOutBecause {
    /// Whether setting a kind aside for this reason is evidence that something filters it.
    ///
    /// False for [`Self::ItCannotAddressThisTarget`]: our own inability to form the probe
    /// says nothing whatever about the path, and letting it stand in for filtering would
    /// have the UI claim a network fact we never observed. False for
    /// [`Self::ItFoundNoServiceThere`] for the mirror reason: the endpoint answered, so the
    /// path plainly works — a closed port is the opposite of evidence that something is
    /// dropping our packets.
    const fn suggests_filtering(self) -> bool {
        matches!(self, Self::ItReportedFiltering | Self::ItWentSilent)
    }
}

/// A probe kind that has been set aside for an endpoint, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuledOut {
    /// The kind no longer being used.
    pub kind: ProbeKind,
    /// What set it aside.
    pub because: RuledOutBecause,
}

/// Tracks which probe kind is measuring one endpoint, and what has been ruled out.
///
/// One chain per endpoint. It holds no clock and issues no probes: the caller probes with
/// whatever [`FallbackChain::step`] returns and hands the outcome back to
/// [`FallbackChain::record`], which is what makes an entire session's worth of degradation
/// testable without a network.
#[derive(Debug, Clone)]
pub struct FallbackChain {
    class: AddressClass,
    order: Vec<ProbeKind>,
    position: usize,
    consecutive_silent: u32,
    consecutive_refused: u32,
    ruled_out: Vec<RuledOut>,
    filtering_confirmed: bool,
}

impl FallbackChain {
    /// Starts a chain for an endpoint of `class`, given the probe kinds that exist.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NothingUsable`] when no available kind can honestly measure an
    /// address of this class — the same refusal [`crate::probe::select_kind`] makes, at the
    /// point where a chain would otherwise be created that could never do anything.
    pub fn new(class: AddressClass, available: &[ProbeKind]) -> Result<Self, Error> {
        Self::starting_with(class, available, None)
    }

    /// Starts a chain that tries `preferred` before the rest of the honest order.
    ///
    /// The hint **reorders, it never admits**. A kind the address class refuses — anything
    /// but an end-to-end exchange for a tunnelled endpoint, anything at all for an address
    /// not worth probing — stays refused however loudly a data file asks for it, so a
    /// hand-edited list can shorten a wait but can never make the engine report a number a
    /// tunnel invented. A hint naming a kind this build does not have is likewise ignored
    /// rather than fatal.
    ///
    /// It exists because the cheapest-first order is the wrong opening move for a service
    /// whose front door is known to drop echoes: without the hint the chain spends
    /// [`SILENCE_BEFORE_FALLBACK`] whole check intervals — minutes, at a status page's
    /// cadence — reporting silence about a service that is answering perfectly on 443.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NothingUsable`] under exactly the conditions [`FallbackChain::new`]
    /// does; the hint cannot rescue an address no kind may honestly measure.
    pub fn starting_with(
        class: AddressClass,
        available: &[ProbeKind],
        preferred: Option<ProbeKind>,
    ) -> Result<Self, Error> {
        let mut order = preferred_kinds(class, available);
        if order.is_empty() {
            return Err(Error::NothingUsable { class });
        }
        if let Some(first) = preferred {
            if let Some(at) = order.iter().position(|kind| *kind == first) {
                let kind = order.remove(at);
                order.insert(0, kind);
            }
        }
        Ok(Self {
            class,
            order,
            position: 0,
            consecutive_silent: 0,
            consecutive_refused: 0,
            ruled_out: Vec::new(),
            filtering_confirmed: false,
        })
    }

    /// What to do for the next probe.
    #[must_use]
    pub fn step(&self) -> ChainStep {
        match self.current_kind() {
            Some(kind) => ChainStep::Probe(kind),
            // A TTL walk times routers with ICMP. That is informative for an address this
            // machine really routes to, and meaningless for one a local tunnel intercepts.
            None if self.class.trusts_transport_rtt() => ChainStep::WalkThePath,
            None => ChainStep::Nothing,
        }
    }

    /// The kind currently in use, if any kind is left.
    #[must_use]
    pub fn current_kind(&self) -> Option<ProbeKind> {
        self.order.get(self.position).copied()
    }

    /// Every kind set aside for this endpoint, in the order they were.
    #[must_use]
    pub fn ruled_out(&self) -> &[RuledOut] {
        &self.ruled_out
    }

    /// Whether a probe kind has been *proven* to be filtered on this path.
    ///
    /// True only once a later kind has succeeded after an earlier one was set aside: that is
    /// what separates "echoes are being dropped" from "the host is down", and without it the
    /// UI must not claim either.
    #[must_use]
    pub const fn filtering_confirmed(&self) -> bool {
        self.filtering_confirmed
    }

    /// Folds one probe result into the decision.
    pub fn record(&mut self, outcome: ProbeOutcome) {
        let Some(kind) = self.current_kind() else {
            // Walking the path, or out of options entirely: there is nothing left to demote.
            return;
        };

        match outcome {
            ProbeOutcome::Success(_) => {
                self.consecutive_silent = 0;
                self.consecutive_refused = 0;
                // Only a kind set aside *by the network* proves filtering. One we could not
                // form the probe for proves something about this build.
                if self
                    .ruled_out
                    .iter()
                    .any(|entry| entry.because.suggests_filtering())
                {
                    self.filtering_confirmed = true;
                }
            }
            // A refusal from a kind that had to pick a port answers a different question from
            // the one asked: nothing listens on *that port*. A game's match server refuses a
            // handshake on the port it plays UDP over while the match runs perfectly across
            // it, so leaving the kind in place would park the endpoint on an answer that can
            // never become a measurement.
            ProbeOutcome::Unreachable if kind.needs_a_port() => {
                self.consecutive_silent = 0;
                self.consecutive_refused = self.consecutive_refused.saturating_add(1);
                if self.consecutive_refused >= REFUSALS_BEFORE_FALLBACK {
                    self.set_aside(kind, RuledOutBecause::ItFoundNoServiceThere);
                }
            }
            // A definitive answer *about the destination*, delivered by a working path. It
            // says nothing against the probe kind, so it must not cost the cheapest kind its
            // place — an endpoint that is simply down would otherwise walk the whole chain,
            // and every kind after it would fail in exactly the same way.
            ProbeOutcome::Unreachable => self.consecutive_silent = 0,
            ProbeOutcome::Blocked => self.set_aside(kind, RuledOutBecause::ItReportedFiltering),
            ProbeOutcome::Timeout => {
                self.consecutive_silent = self.consecutive_silent.saturating_add(1);
                if self.consecutive_silent >= SILENCE_BEFORE_FALLBACK {
                    self.set_aside(kind, RuledOutBecause::ItWentSilent);
                }
            }
        }
    }

    /// Sets the current kind aside because this build cannot address the target with it.
    ///
    /// Separate from [`FallbackChain::record`] because it is not an outcome: no probe
    /// reached the network, so there is nothing to fold into a measurement. Doing nothing
    /// instead would retry the same impossible probe until the endpoint is forgotten.
    ///
    /// Does nothing once every kind is exhausted.
    pub fn cannot_address_target(&mut self) {
        let Some(kind) = self.current_kind() else {
            return;
        };
        self.set_aside(kind, RuledOutBecause::ItCannotAddressThisTarget);
    }

    /// Whether the current kind was set aside because it could not address the target.
    ///
    /// Reported apart from filtering because the two are different claims: one is about the
    /// network, the other about this build.
    #[must_use]
    pub fn unaddressable_kinds(&self) -> usize {
        self.ruled_out
            .iter()
            .filter(|entry| entry.because == RuledOutBecause::ItCannotAddressThisTarget)
            .count()
    }

    /// Returns to the preferred kind and forgets what was ruled out.
    ///
    /// Filtering is not permanent — a route change or a firewall edit can restore a kind that
    /// was dropped hours ago — but re-testing costs probes and risks a stretch of silence on
    /// an endpoint that was being measured perfectly well. The chain therefore never decides
    /// on its own when to try again; the scheduler does, rarely.
    pub fn reconsider(&mut self) {
        self.position = 0;
        self.consecutive_silent = 0;
        self.consecutive_refused = 0;
        self.ruled_out.clear();
        self.filtering_confirmed = false;
    }

    fn set_aside(&mut self, kind: ProbeKind, because: RuledOutBecause) {
        self.ruled_out.push(RuledOut { kind, because });
        self.position += 1;
        self.consecutive_silent = 0;
        self.consecutive_refused = 0;
    }
}

#[cfg(test)]
mod tests {
    use nm_core::sample::Rtt;

    use super::*;

    const ALL: &[ProbeKind] = &[
        ProbeKind::IcmpEcho,
        ProbeKind::TcpConnect,
        ProbeKind::TlsHello,
    ];

    fn chain() -> FallbackChain {
        FallbackChain::new(AddressClass::Routable, ALL).unwrap()
    }

    fn success() -> ProbeOutcome {
        ProbeOutcome::Success(Rtt::from_micros(12_000))
    }

    /// Feeds `outcome` in `times` times.
    fn repeat(chain: &mut FallbackChain, outcome: ProbeOutcome, times: u32) {
        for _ in 0..times {
            chain.record(outcome);
        }
    }

    #[test]
    fn a_direct_endpoint_starts_on_the_cheapest_kind() {
        let chain = chain();
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::IcmpEcho));
        assert!(chain.ruled_out().is_empty());
        assert!(!chain.filtering_confirmed());
    }

    #[test]
    fn an_endpoint_nothing_can_measure_is_refused_a_chain() {
        assert_eq!(
            FallbackChain::new(AddressClass::Loopback, ALL).unwrap_err(),
            Error::NothingUsable {
                class: AddressClass::Loopback
            }
        );
        assert!(FallbackChain::new(AddressClass::Routable, &[]).is_err());
    }

    #[test]
    fn a_kind_that_reports_filtering_is_set_aside_at_once() {
        // No reason to wait: the probe did not merely fail, it said it cannot work here.
        let mut chain = chain();
        chain.record(ProbeOutcome::Blocked);

        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TcpConnect));
        assert_eq!(
            chain.ruled_out(),
            &[RuledOut {
                kind: ProbeKind::IcmpEcho,
                because: RuledOutBecause::ItReportedFiltering
            }]
        );
    }

    #[test]
    fn silence_is_given_a_few_chances_before_a_kind_is_set_aside() {
        let mut chain = chain();
        repeat(
            &mut chain,
            ProbeOutcome::Timeout,
            SILENCE_BEFORE_FALLBACK - 1,
        );
        assert_eq!(
            chain.step(),
            ChainStep::Probe(ProbeKind::IcmpEcho),
            "ordinary packet loss must not cost the cheapest kind its place"
        );

        chain.record(ProbeOutcome::Timeout);
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TcpConnect));
        assert_eq!(
            chain.ruled_out(),
            &[RuledOut {
                kind: ProbeKind::IcmpEcho,
                because: RuledOutBecause::ItWentSilent
            }]
        );
    }

    #[test]
    fn the_run_of_silence_has_to_be_unbroken() {
        // Losing every other packet is a measurement of a bad link, not evidence that echoes
        // are filtered — and switching kinds would replace that measurement with a different
        // one instead of reporting it.
        let mut chain = chain();
        for _ in 0..10 {
            repeat(
                &mut chain,
                ProbeOutcome::Timeout,
                SILENCE_BEFORE_FALLBACK - 1,
            );
            chain.record(success());
        }
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::IcmpEcho));
        assert!(chain.ruled_out().is_empty());
    }

    #[test]
    fn a_definitive_unreachable_does_not_cost_a_kind_its_place() {
        // The endpoint is answering, with a "no". Walking the chain would swap a good
        // measurement for an expensive one that reports exactly the same thing.
        let mut chain = chain();
        repeat(&mut chain, ProbeOutcome::Unreachable, 20);
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::IcmpEcho));
        assert!(chain.ruled_out().is_empty());
    }

    #[test]
    fn a_closed_port_retires_the_kind_that_had_to_choose_one() {
        // Found by running the app against a live game: its match server refuses a TCP
        // handshake on the port it plays UDP over, which is normal — nothing listens on a
        // game port but the game. Left in place, that answer parks the endpoint on a kind
        // that can never measure anything, and the path walk is never reached.
        let mut chain = FallbackChain::new(AddressClass::Routable, ALL).unwrap();
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);
        assert_eq!(chain.current_kind(), Some(ProbeKind::TcpConnect));

        repeat(
            &mut chain,
            ProbeOutcome::Unreachable,
            REFUSALS_BEFORE_FALLBACK - 1,
        );
        assert_eq!(
            chain.current_kind(),
            Some(ProbeKind::TcpConnect),
            "one reset could be a middlebox; a run of them is the port"
        );

        chain.record(ProbeOutcome::Unreachable);
        assert_eq!(chain.current_kind(), Some(ProbeKind::TlsHello));
        assert_eq!(
            chain.ruled_out().last().unwrap().because,
            RuledOutBecause::ItFoundNoServiceThere
        );
    }

    #[test]
    fn a_closed_port_is_never_evidence_of_filtering() {
        // The mirror of the rule for a probe we could not form: the endpoint answered, so
        // the path plainly works. A closed port is the opposite of evidence that something
        // is dropping our packets, and the UI must not offer "try a VPN" because of one.
        // Only the closed port sets a kind aside here: an ICMP silence earlier in the chain
        // would be genuine evidence, and this test is about the refusal alone.
        let mut chain = FallbackChain::new(
            AddressClass::Routable,
            &[ProbeKind::TcpConnect, ProbeKind::TlsHello],
        )
        .unwrap();
        repeat(
            &mut chain,
            ProbeOutcome::Unreachable,
            REFUSALS_BEFORE_FALLBACK,
        );
        chain.record(success());

        assert_eq!(chain.current_kind(), Some(ProbeKind::TlsHello));
        assert!(!chain.filtering_confirmed());
    }

    #[test]
    fn a_run_of_refusals_has_to_be_unbroken() {
        let mut chain = FallbackChain::new(AddressClass::Routable, ALL).unwrap();
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);

        for _ in 0..10 {
            repeat(
                &mut chain,
                ProbeOutcome::Unreachable,
                REFUSALS_BEFORE_FALLBACK - 1,
            );
            chain.record(success());
        }
        assert_eq!(chain.current_kind(), Some(ProbeKind::TcpConnect));
    }

    #[test]
    fn a_game_endpoint_that_answers_no_probe_at_all_reaches_the_path_walk() {
        // The whole point of the rule, end to end: echoes filtered, both connecting kinds
        // refused on the game's own port. Nothing is left that can measure the endpoint, and
        // the route to it is the only honest thing remaining to measure.
        let mut chain = FallbackChain::new(AddressClass::Routable, ALL).unwrap();
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);
        repeat(
            &mut chain,
            ProbeOutcome::Unreachable,
            REFUSALS_BEFORE_FALLBACK,
        );
        repeat(
            &mut chain,
            ProbeOutcome::Unreachable,
            REFUSALS_BEFORE_FALLBACK,
        );

        assert_eq!(chain.step(), ChainStep::WalkThePath);
        assert!(!chain.filtering_confirmed());
    }

    #[test]
    fn filtering_is_only_claimed_once_another_kind_proves_the_host_is_up() {
        let mut chain = chain();
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);
        assert!(
            !chain.filtering_confirmed(),
            "silence alone is equally consistent with a dead host"
        );

        chain.record(success());
        assert!(
            chain.filtering_confirmed(),
            "a working TCP handshake proves the host is up and the echoes were dropped"
        );
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TcpConnect));
    }

    #[test]
    fn a_first_kind_that_simply_works_claims_nothing() {
        let mut chain = chain();
        repeat(&mut chain, success(), 50);
        assert!(!chain.filtering_confirmed());
        assert!(chain.ruled_out().is_empty());
    }

    #[test]
    fn the_chain_is_walked_in_preference_order() {
        let mut chain = chain();
        chain.record(ProbeOutcome::Blocked);
        assert_eq!(chain.current_kind(), Some(ProbeKind::TcpConnect));
        chain.record(ProbeOutcome::Blocked);
        assert_eq!(chain.current_kind(), Some(ProbeKind::TlsHello));
    }

    #[test]
    fn a_direct_endpoint_that_exhausts_every_kind_falls_back_to_the_path() {
        let mut chain = chain();
        repeat(&mut chain, ProbeOutcome::Blocked, 3);

        assert_eq!(chain.current_kind(), None);
        assert_eq!(chain.step(), ChainStep::WalkThePath);
        assert_eq!(chain.ruled_out().len(), 3);
    }

    #[test]
    fn a_tunnelled_endpoint_that_exhausts_its_kind_admits_the_gap() {
        // A TTL walk from this machine would map the route to the tunnel, not to the
        // destination. Offering it would be worse than saying nothing.
        let mut chain = FallbackChain::new(AddressClass::TunnelSentinel, ALL).unwrap();
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TlsHello));

        chain.record(ProbeOutcome::Blocked);
        assert_eq!(chain.step(), ChainStep::Nothing);
    }

    #[test]
    fn results_arriving_after_every_kind_is_gone_change_nothing() {
        let mut chain = chain();
        repeat(&mut chain, ProbeOutcome::Blocked, 3);
        let before = chain.ruled_out().len();

        // A path walk's outcome, or a probe still in flight when the last kind was dropped.
        chain.record(success());
        chain.record(ProbeOutcome::Timeout);

        assert_eq!(chain.step(), ChainStep::WalkThePath);
        assert_eq!(chain.ruled_out().len(), before);
        assert!(!chain.filtering_confirmed());
    }

    #[test]
    fn reconsidering_returns_to_the_cheapest_kind_with_a_clean_slate() {
        let mut chain = chain();
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);
        chain.record(success());
        assert!(chain.filtering_confirmed());

        chain.reconsider();
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::IcmpEcho));
        assert!(chain.ruled_out().is_empty());
        assert!(
            !chain.filtering_confirmed(),
            "a claim of filtering must not outlive the evidence for it"
        );
    }

    #[test]
    fn reconsidering_does_not_inherit_a_part_finished_run_of_silence() {
        let mut chain = chain();
        repeat(
            &mut chain,
            ProbeOutcome::Timeout,
            SILENCE_BEFORE_FALLBACK - 1,
        );
        chain.reconsider();

        chain.record(ProbeOutcome::Timeout);
        assert_eq!(
            chain.step(),
            ChainStep::Probe(ProbeKind::IcmpEcho),
            "a single timeout after a reset must not immediately abandon the kind"
        );
    }

    #[test]
    fn a_kind_that_cannot_address_the_target_is_set_aside_immediately() {
        // Unlike silence, this needs no run of evidence: nothing about the endpoint will
        // make a probe we cannot form start working.
        let mut chain = chain();
        chain.cannot_address_target();

        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TcpConnect));
        assert_eq!(chain.unaddressable_kinds(), 1);
        assert_eq!(
            chain.ruled_out()[0].because,
            RuledOutBecause::ItCannotAddressThisTarget
        );
    }

    #[test]
    fn being_unable_to_form_a_probe_is_never_evidence_of_filtering() {
        // The honesty rule: "ICMP is filtered here" is a claim about the network. Our own
        // missing IPv6 support is a claim about this build, and a later kind succeeding
        // proves only that the later kind works.
        let mut chain = chain();
        chain.cannot_address_target();
        chain.record(ProbeOutcome::Success(Rtt::from_micros(9_000)));

        assert!(!chain.filtering_confirmed());
    }

    #[test]
    fn a_kind_ruled_out_by_the_network_still_proves_filtering_afterwards() {
        // The complement of the test above: mixing the two reasons must not suppress a
        // genuine finding.
        let mut chain = chain();
        chain.cannot_address_target();
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);
        chain.record(ProbeOutcome::Success(Rtt::from_micros(9_000)));

        assert!(chain.filtering_confirmed());
    }

    #[test]
    fn stepping_past_every_kind_this_way_leaves_the_path_walk() {
        let mut chain = chain();
        for _ in 0..3 {
            chain.cannot_address_target();
        }

        assert_eq!(chain.current_kind(), None);
        assert_eq!(chain.step(), ChainStep::WalkThePath);
        // And once there is nothing left, saying it again changes nothing.
        chain.cannot_address_target();
        assert_eq!(chain.step(), ChainStep::WalkThePath);
    }

    #[test]
    fn a_hint_opens_on_the_kind_it_names() {
        let chain =
            FallbackChain::starting_with(AddressClass::Routable, ALL, Some(ProbeKind::TlsHello))
                .unwrap();
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TlsHello));
    }

    #[test]
    fn a_hint_reorders_rather_than_removing_the_rest() {
        // The point of the hint is to skip a wait, not to give up the fallbacks: a service
        // whose hinted kind stops working must still reach the others.
        let mut chain =
            FallbackChain::starting_with(AddressClass::Routable, ALL, Some(ProbeKind::TcpConnect))
                .unwrap();
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::IcmpEcho));
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TlsHello));
    }

    #[test]
    fn a_hint_cannot_admit_a_kind_a_tunnel_would_fake() {
        // The whole safety property in one test: a data file asking for the cheapest probe
        // on a tunnelled endpoint gets the honest one anyway, because the hint reorders a
        // list the address class has already filtered.
        let chain = FallbackChain::starting_with(
            AddressClass::TunnelSentinel,
            ALL,
            Some(ProbeKind::TcpConnect),
        )
        .unwrap();
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TlsHello));
    }

    #[test]
    fn a_hint_for_a_kind_this_build_lacks_is_ignored() {
        let chain = FallbackChain::starting_with(
            AddressClass::Routable,
            &[ProbeKind::TcpConnect, ProbeKind::TlsHello],
            Some(ProbeKind::IcmpEcho),
        )
        .unwrap();
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TcpConnect));
    }

    #[test]
    fn a_hint_cannot_rescue_an_address_nothing_may_measure() {
        assert_eq!(
            FallbackChain::starting_with(AddressClass::Loopback, ALL, Some(ProbeKind::TlsHello))
                .unwrap_err(),
            Error::NothingUsable {
                class: AddressClass::Loopback
            }
        );
    }

    #[test]
    fn reconsidering_returns_to_the_hinted_kind_not_the_cheapest() {
        let mut chain =
            FallbackChain::starting_with(AddressClass::Routable, ALL, Some(ProbeKind::TlsHello))
                .unwrap();
        repeat(&mut chain, ProbeOutcome::Timeout, SILENCE_BEFORE_FALLBACK);
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::IcmpEcho));

        chain.reconsider();
        assert_eq!(chain.step(), ChainStep::Probe(ProbeKind::TlsHello));
    }
}
