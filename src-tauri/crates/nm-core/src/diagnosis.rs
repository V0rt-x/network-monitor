//! Turning every signal the app holds into one actionable, network-level statement.
//!
//! Everything else in this crate measures. This is the only module that *concludes*, and
//! the discipline it works under is the product's whole reason for existing: it states
//! **network-level facts and nothing else**. It never says a game is broken, never says a
//! company's service is down, and never names a national border on the strength of one
//! traceroute. What it says is where the evidence stops agreeing — because that, and not a
//! number, is what tells a user whether to wait, to call their provider, or to turn on a
//! VPN.
//!
//! # The comparison is the diagnosis
//!
//! No single measurement can distinguish the cases below; every pair of them looks
//! identical from one endpoint. What separates them is that the app measures several things
//! at once and they disagree in characteristic ways:
//!
//! | domestic | foreign | app's endpoints | game's pool | reads as |
//! |---|---|---|---|---|
//! | bad | bad | — | — | the user's own line or provider |
//! | clean | bad | — | — | the path out of the country |
//! | clean | clean | bad | clean | the route to this application |
//! | clean | clean | bad | all silent | the game's servers |
//! | clean | clean | bad | partly silent | part of the game's servers |
//!
//! # Absence of knowledge is never a finding
//!
//! Every rule below distinguishes "measured and bad" from "could not be measured". A
//! network that filters every probe produces [`Verdict::NothingMeasurable`], never a
//! confident verdict about a border — the same rule `nm_probes::chain` follows when it
//! refuses to claim filtering without proof, applied one level up.

use crate::health::{Health, HealthCounts};
use crate::pool::PoolReading;

/// What the evidence adds up to, as a network-level statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Verdict {
    /// Too little has been measured to say anything at all.
    ///
    /// The state for the first seconds after start-up, and it must never be shown as
    /// anything reassuring.
    NotEnoughEvidence,
    /// Probes are being filtered wherever they were sent, so nothing was measured.
    ///
    /// A real and important state on this product's target networks: it says the app cannot
    /// see, which is different from — and far more honest than — reporting what it cannot
    /// see as an outage.
    NothingMeasurable,
    /// Everything measured is within its thresholds.
    Clear,
    /// Services inside the user's own country are degraded or unreachable.
    ///
    /// The one verdict that points *inward*: if the domestic baseline is failing, the border
    /// cannot be the explanation, because the traffic never reaches it. Stated as the user's
    /// own line or provider and no more precisely, because from here those two are the same
    /// observation.
    LocalNetworkOrProvider,
    /// Domestic services answer normally and foreign ones do not.
    ///
    /// This is the corroborated version of the claim `nm_core::path` deliberately refuses to
    /// make on its own. One traceroute dying at a long-haul link is not evidence; that same
    /// route dying while every domestic service answers perfectly is. Even then it names a
    /// *path*, not a policy: throttling, a blocked route and a broken transit link all look
    /// like this, and the app cannot tell them apart.
    CrossBorderPath,
    /// The general network is fine and this application's own endpoints are not.
    ///
    /// The case a game accelerator exists for: the route this application's traffic takes is
    /// worse than the routes everything else takes.
    RouteToThisApplication,
    /// The game's own reference targets have all gone silent while the network is fine.
    ///
    /// The strongest statement this module makes about a game, and still a network-level
    /// one: the addresses that belong to that game's infrastructure are not answering from
    /// here. It does not say the game is broken.
    GameServersUnreachable,
    /// Part of the game's reference pool is silent while the rest answers.
    ///
    /// A partial outage — some of a game's regions gone while others serve normally — which
    /// a single endpoint could never show, and which is why the pool exists.
    GameServersPartlyUnreachable,
}

impl Verdict {
    /// Whether this verdict describes a problem the user can act on.
    #[must_use]
    pub const fn is_actionable(self) -> bool {
        !matches!(
            self,
            Self::NotEnoughEvidence | Self::NothingMeasurable | Self::Clear
        )
    }
}

/// What a monitored application contributes to a diagnosis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppEvidence {
    /// How its endpoints are distributed across the health states.
    pub endpoints: HealthCounts,
    /// What its game's reference pool says, when it has one.
    ///
    /// [`None`] for an application with no pool — a title whose operator publishes no
    /// reference address and whose servers this machine has never seen. Without it the two
    /// game-server verdicts are simply unavailable, and the diagnosis stops at the route.
    pub pool: Option<PoolReading>,
}

/// Everything the diagnosis reads.
///
/// A plain struct of already-computed verdicts rather than raw samples: each field is the
/// product of a rule that is unit-tested where it lives, and re-deriving any of them here
/// would put two answers to the same question in the codebase.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evidence {
    /// The domestic baseline group's headline verdict.
    pub domestic: Health,
    /// The foreign baseline group's headline verdict.
    pub foreign: Health,
    /// The application being diagnosed, when the verdict is about one.
    pub app: Option<AppEvidence>,
}

impl Evidence {
    /// The evidence for the general network alone, with no application in view.
    #[must_use]
    pub const fn baselines(domestic: Health, foreign: Health) -> Self {
        Self {
            domestic,
            foreign,
            app: None,
        }
    }

    /// The same evidence with an application's endpoints and pool folded in.
    #[must_use]
    pub const fn about(mut self, app: AppEvidence) -> Self {
        self.app = Some(app);
        self
    }
}

/// A verdict and the evidence it actually covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnosis {
    /// What the evidence says.
    pub verdict: Verdict,
    /// How many of the application's endpoints the verdict is about.
    ///
    /// Zero for a verdict about the general network, which is about all of them or none.
    /// The requirement `CLAUDE.md` and Phase 4 both state: a verdict covers the endpoints it
    /// explains and says which, because partial failure inside one application is the normal
    /// case under filtering rather than an edge one. "Two of seven endpoints" is a different
    /// message from "your game is unreachable", and only one of them is true.
    pub endpoints_affected: usize,
    /// How many endpoints the application has in total.
    pub endpoints_total: usize,
}

impl Diagnosis {
    /// A verdict about the general network, covering no particular endpoint.
    const fn general(verdict: Verdict) -> Self {
        Self {
            verdict,
            endpoints_affected: 0,
            endpoints_total: 0,
        }
    }
}

/// Reads the evidence and states what it supports.
///
/// The rules run in order, and the order is the argument: the general network is settled
/// before anything is claimed about an application, because an application's endpoints
/// failing while the whole network is failing says nothing about that application.
///
/// It cannot invent a verdict. Every branch that runs out of evidence lands on
/// [`Verdict::NotEnoughEvidence`] or [`Verdict::NothingMeasurable`], and neither of those
/// may be rendered as good news.
#[must_use]
pub fn diagnose(evidence: &Evidence) -> Diagnosis {
    // Nothing has been measured yet at all. First, because every rule below reads a verdict
    // that would otherwise be `Unknown` and mean nothing.
    if !evidence.domestic.is_known() && !evidence.foreign.is_known() {
        return Diagnosis::general(Verdict::NotEnoughEvidence);
    }

    // Every probe that could have said something was filtered. This is a statement about
    // what the app can see, and it outranks every verdict below precisely because those
    // would all be guesses built on an absence.
    if evidence.domestic == Health::Blocked && evidence.foreign == Health::Blocked {
        return Diagnosis::general(Verdict::NothingMeasurable);
    }

    // The domestic baseline is the one that points inward. Traffic to a service inside the
    // country never reaches a border, so a border cannot explain its failure.
    if evidence.domestic.is_known() && !evidence.domestic.is_answering() {
        return Diagnosis::general(Verdict::LocalNetworkOrProvider);
    }
    if evidence.domestic == Health::Degraded {
        return Diagnosis::general(Verdict::LocalNetworkOrProvider);
    }

    // Domestic services answer and foreign ones do not: the corroboration a single
    // traceroute could never supply.
    if evidence.foreign.is_known() && !evidence.foreign.is_answering() {
        return Diagnosis::general(Verdict::CrossBorderPath);
    }
    if evidence.foreign == Health::Degraded {
        return Diagnosis::general(Verdict::CrossBorderPath);
    }

    // The general network is as good as this app can measure. Anything left is about the
    // application in view — and with none in view there is nothing further to say.
    let Some(app) = evidence.app else {
        return Diagnosis::general(
            if evidence.domestic.is_known() || evidence.foreign.is_known() {
                Verdict::Clear
            } else {
                Verdict::NotEnoughEvidence
            },
        );
    };

    diagnose_app(&app)
}

/// The half of the rules that need an application in view.
fn diagnose_app(app: &AppEvidence) -> Diagnosis {
    let counts = app.endpoints;
    let total = counts.total();
    // Endpoints that are failing in a way a *path* explains. An endpoint proven alive by its
    // own traffic is not one of them — that is a game's match server behaving normally — and
    // neither is one whose probes were merely filtered, which measured nothing.
    let affected = counts.unreachable + counts.degraded;

    let verdict = |verdict: Verdict| Diagnosis {
        verdict,
        endpoints_affected: affected,
        endpoints_total: total,
    };

    if total == 0 || counts.known() == 0 {
        return verdict(Verdict::NotEnoughEvidence);
    }

    // What the game's own infrastructure says, when there is a pool to ask. Checked before
    // the route, because "the game's servers are not answering from here" explains this
    // application's failing endpoints and the route does not explain the pool.
    if let Some(pool) = app.pool {
        match pool.answering_ratio() {
            Some(ratio) if ratio <= 0.0 && pool.judged() > 0 => {
                return verdict(Verdict::GameServersUnreachable);
            }
            Some(ratio) if ratio < 1.0 => {
                return verdict(Verdict::GameServersPartlyUnreachable);
            }
            // A pool answering everywhere, or one that could not be judged at all. Neither
            // supports a claim about the game's servers, so the rules fall through to the
            // route — which is exactly right for the first and honest for the second.
            _ => {}
        }
    }

    if affected > 0 {
        return verdict(Verdict::RouteToThisApplication);
    }
    // Nothing is failing in a way a path explains. If some endpoints were filtered and
    // nothing else was measured, saying "clear" would be claiming an absence as a finding.
    if counts.ok == 0 && counts.carrying_traffic == 0 {
        return verdict(Verdict::NothingMeasurable);
    }
    verdict(Verdict::Clear)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A distribution built from the states that matter, in the order they are named.
    fn counts(ok: usize, degraded: usize, unreachable: usize) -> HealthCounts {
        HealthCounts {
            ok,
            degraded,
            unreachable,
            ..HealthCounts::default()
        }
    }

    /// A pool reading with `answering` of `total` members answering.
    fn pool(answering: usize, unreachable: usize) -> PoolReading {
        PoolReading {
            counts: counts(answering, 0, unreachable),
            unproven: 0,
            rtt_ms: None,
            loss_pct: None,
        }
    }

    /// A pool whose every probe was filtered: judged nothing.
    fn filtered_pool(members: usize) -> PoolReading {
        PoolReading {
            counts: HealthCounts {
                blocked: members,
                ..HealthCounts::default()
            },
            unproven: 0,
            rtt_ms: None,
            loss_pct: None,
        }
    }

    fn app(endpoints: HealthCounts, pool: Option<PoolReading>) -> AppEvidence {
        AppEvidence { endpoints, pool }
    }

    #[test]
    fn nothing_measured_yet_says_so_rather_than_anything_reassuring() {
        let diagnosis = diagnose(&Evidence::baselines(Health::Unknown, Health::Unknown));
        assert_eq!(diagnosis.verdict, Verdict::NotEnoughEvidence);
        assert!(!diagnosis.verdict.is_actionable());
    }

    #[test]
    fn every_probe_filtered_is_reported_as_not_being_able_to_see() {
        // The alternative — calling it a border — would be inventing a finding out of an
        // absence, on exactly the networks where being wrong matters most.
        let diagnosis = diagnose(&Evidence::baselines(Health::Blocked, Health::Blocked));
        assert_eq!(diagnosis.verdict, Verdict::NothingMeasurable);
    }

    #[test]
    fn both_baselines_healthy_and_no_application_is_clear() {
        let diagnosis = diagnose(&Evidence::baselines(Health::Ok, Health::Ok));
        assert_eq!(diagnosis.verdict, Verdict::Clear);
        assert_eq!(diagnosis.endpoints_total, 0);
    }

    #[test]
    fn a_failing_domestic_baseline_points_inward() {
        // Traffic to a service inside the country never reaches a border, so a border
        // cannot be the explanation.
        for domestic in [Health::Unreachable, Health::Degraded, Health::Blocked] {
            let diagnosis = diagnose(&Evidence::baselines(domestic, Health::Unreachable));
            assert_eq!(
                diagnosis.verdict,
                Verdict::LocalNetworkOrProvider,
                "{domestic:?}"
            );
        }
    }

    #[test]
    fn a_failing_domestic_baseline_outranks_a_failing_foreign_one() {
        let diagnosis = diagnose(&Evidence::baselines(Health::Unreachable, Health::Ok));
        assert_eq!(diagnosis.verdict, Verdict::LocalNetworkOrProvider);
    }

    #[test]
    fn domestic_clean_with_foreign_dead_is_the_path_out_of_the_country() {
        let diagnosis = diagnose(&Evidence::baselines(Health::Ok, Health::Unreachable));
        assert_eq!(diagnosis.verdict, Verdict::CrossBorderPath);
    }

    #[test]
    fn a_foreign_baseline_whose_probes_are_all_filtered_is_still_the_path_out() {
        // Filtering that stops at the border is one of the things a border does, and the
        // domestic baseline answering is what makes it a finding rather than a guess.
        let diagnosis = diagnose(&Evidence::baselines(Health::Ok, Health::Blocked));
        assert_eq!(diagnosis.verdict, Verdict::CrossBorderPath);
    }

    #[test]
    fn a_merely_degraded_foreign_baseline_is_still_the_path_out() {
        // Throttling rather than blocking, which is the more common shape and reads the same
        // way here: the app cannot tell them apart and does not pretend to.
        let diagnosis = diagnose(&Evidence::baselines(Health::Ok, Health::Degraded));
        assert_eq!(diagnosis.verdict, Verdict::CrossBorderPath);
    }

    #[test]
    fn a_general_verdict_covers_no_particular_endpoint() {
        let diagnosis = diagnose(&Evidence::baselines(Health::Ok, Health::Unreachable));
        assert_eq!(diagnosis.endpoints_affected, 0);
        assert_eq!(diagnosis.endpoints_total, 0);
    }

    #[test]
    fn a_clean_network_and_a_clean_application_is_clear() {
        let evidence = Evidence::baselines(Health::Ok, Health::Ok)
            .about(app(counts(4, 0, 0), Some(pool(8, 0))));
        assert_eq!(diagnose(&evidence).verdict, Verdict::Clear);
    }

    #[test]
    fn a_clean_network_and_a_clean_pool_blames_the_route_to_the_application() {
        // The case a game accelerator exists for: everything else is fine, the game's own
        // infrastructure is fine, and the path this application's traffic takes is not.
        let evidence = Evidence::baselines(Health::Ok, Health::Ok)
            .about(app(counts(5, 0, 2), Some(pool(8, 0))));
        let diagnosis = diagnose(&evidence);

        assert_eq!(diagnosis.verdict, Verdict::RouteToThisApplication);
        assert_eq!(diagnosis.endpoints_affected, 2);
        assert_eq!(diagnosis.endpoints_total, 7);
    }

    #[test]
    fn a_silent_pool_blames_the_games_own_infrastructure() {
        let evidence = Evidence::baselines(Health::Ok, Health::Ok)
            .about(app(counts(1, 0, 3), Some(pool(0, 8))));
        assert_eq!(diagnose(&evidence).verdict, Verdict::GameServersUnreachable);
    }

    #[test]
    fn a_partly_silent_pool_reports_a_partial_outage() {
        // The thing no single endpoint could show: some of a game's regions gone while
        // others serve normally.
        let evidence = Evidence::baselines(Health::Ok, Health::Ok)
            .about(app(counts(2, 0, 2), Some(pool(4, 4))));
        assert_eq!(
            diagnose(&evidence).verdict,
            Verdict::GameServersPartlyUnreachable
        );
    }

    #[test]
    fn a_pool_that_could_not_be_judged_never_becomes_a_verdict_about_the_game() {
        // Every pool probe filtered says nothing about the game's servers, so the rules must
        // fall through to what *was* measured.
        let evidence = Evidence::baselines(Health::Ok, Health::Ok)
            .about(app(counts(3, 0, 1), Some(filtered_pool(8))));
        assert_eq!(diagnose(&evidence).verdict, Verdict::RouteToThisApplication);
    }

    #[test]
    fn a_pool_of_members_that_have_never_answered_never_blames_the_game() {
        // The state of every pool built purely from a UDP title's own match servers, which
        // answer nothing by design. Left unguarded this would report an outage on every
        // match of every such game — so it must fall through to what was actually measured.
        let evidence = Evidence::baselines(Health::Ok, Health::Ok).about(app(
            counts(3, 0, 1),
            Some(PoolReading {
                counts: HealthCounts::default(),
                unproven: 6,
                rtt_ms: None,
                loss_pct: None,
            }),
        ));
        assert_eq!(diagnose(&evidence).verdict, Verdict::RouteToThisApplication);
    }

    #[test]
    fn an_application_with_no_pool_stops_at_the_route() {
        // Most titles publish no reference address, so this is the ordinary case rather than
        // an edge one, and the verdict must be the weaker true one rather than the stronger
        // guess.
        let evidence =
            Evidence::baselines(Health::Ok, Health::Ok).about(app(counts(3, 0, 1), None));
        assert_eq!(diagnose(&evidence).verdict, Verdict::RouteToThisApplication);
    }

    #[test]
    fn a_match_server_proven_alive_by_its_traffic_is_not_a_failing_endpoint() {
        // The normal state of every UDP game server: nothing we can send is answered, while
        // the match runs perfectly. Counting it as a failure would report a working game as
        // broken, which is the one lie this product exists not to tell.
        let evidence = Evidence::baselines(Health::Ok, Health::Ok).about(app(
            HealthCounts {
                ok: 3,
                carrying_traffic: 1,
                ..HealthCounts::default()
            },
            Some(pool(8, 0)),
        ));
        let diagnosis = diagnose(&evidence);

        assert_eq!(diagnosis.verdict, Verdict::Clear);
        assert_eq!(diagnosis.endpoints_affected, 0);
    }

    #[test]
    fn an_endpoint_whose_probes_were_filtered_is_not_counted_as_a_failure_either() {
        let evidence = Evidence::baselines(Health::Ok, Health::Ok).about(app(
            HealthCounts {
                ok: 2,
                blocked: 2,
                ..HealthCounts::default()
            },
            None,
        ));
        let diagnosis = diagnose(&evidence);

        assert_eq!(diagnosis.verdict, Verdict::Clear);
        assert_eq!(diagnosis.endpoints_affected, 0);
        assert_eq!(diagnosis.endpoints_total, 4);
    }

    #[test]
    fn an_application_whose_every_endpoint_was_filtered_reports_that_it_cannot_see() {
        // Nothing failed and nothing succeeded. Calling it clear would present an absence of
        // measurement as good news.
        let evidence = Evidence::baselines(Health::Ok, Health::Ok).about(app(
            HealthCounts {
                blocked: 3,
                ..HealthCounts::default()
            },
            None,
        ));
        assert_eq!(diagnose(&evidence).verdict, Verdict::NothingMeasurable);
    }

    #[test]
    fn an_application_with_no_endpoints_yet_says_so() {
        let evidence =
            Evidence::baselines(Health::Ok, Health::Ok).about(app(HealthCounts::default(), None));
        assert_eq!(diagnose(&evidence).verdict, Verdict::NotEnoughEvidence);
    }

    #[test]
    fn an_application_whose_endpoints_are_all_unknown_says_so() {
        let evidence = Evidence::baselines(Health::Ok, Health::Ok).about(app(
            HealthCounts {
                unknown: 4,
                ..HealthCounts::default()
            },
            None,
        ));
        assert_eq!(diagnose(&evidence).verdict, Verdict::NotEnoughEvidence);
    }

    #[test]
    fn a_broken_network_is_never_blamed_on_the_application() {
        // The ordering rule in one test: an application's endpoints failing while the whole
        // network is failing says nothing whatever about that application, and a verdict
        // sending the user to a game accelerator would waste their time and their money.
        let evidence = Evidence::baselines(Health::Unreachable, Health::Unreachable)
            .about(app(counts(0, 0, 7), Some(pool(0, 8))));
        assert_eq!(diagnose(&evidence).verdict, Verdict::LocalNetworkOrProvider);
    }

    #[test]
    fn a_border_problem_is_never_blamed_on_the_game() {
        let evidence = Evidence::baselines(Health::Ok, Health::Unreachable)
            .about(app(counts(0, 0, 7), Some(pool(0, 8))));
        assert_eq!(diagnose(&evidence).verdict, Verdict::CrossBorderPath);
    }

    #[test]
    fn only_problems_are_actionable() {
        for verdict in [
            Verdict::NotEnoughEvidence,
            Verdict::NothingMeasurable,
            Verdict::Clear,
        ] {
            assert!(!verdict.is_actionable(), "{verdict:?}");
        }
        for verdict in [
            Verdict::LocalNetworkOrProvider,
            Verdict::CrossBorderPath,
            Verdict::RouteToThisApplication,
            Verdict::GameServersUnreachable,
            Verdict::GameServersPartlyUnreachable,
        ] {
            assert!(verdict.is_actionable(), "{verdict:?}");
        }
    }
}
