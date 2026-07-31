//! Probe engine for Network Monitor.
//!
//! Sits between the pure core and the OS layer: it turns scheduling decisions from
//! `nm-core` into actual probes issued through `nm-platform` traits (ICMP, TCP connect,
//! TLS handshake, HTTP(S) `HEAD`, TTL-limited path probes), and enforces the timeout,
//! backoff and rate policy that keeps the app inside its traffic budget.
//!
//! The crate holds no OS-specific code of its own — every syscall goes through a
//! `nm-platform` trait, which is what makes the engine testable with mocks.
//!
//! Phase 2 in progress: the [`probe`] seam, the ICMP and TCP probers. The TLS prober, the
//! fallback chain and the async runner are still to come (see `PLAN.md`).

#![warn(missing_docs)]
// Every OS call goes through a `nm-platform` trait, so this crate needs no `unsafe`.
#![forbid(unsafe_code)]

mod error;

mod socket;

pub mod chain;
pub mod icmp;
pub mod path;
pub mod probe;
pub mod runner;
pub mod tcp;
pub mod tls;

pub use error::Error;

/// Hard ceiling on probes issued per second across everything the app monitors:
/// per-app endpoints, both baselines and the service status page combined.
///
/// This is a product promise, not a tuning knob — see the resource budget in
/// `CLAUDE.md`.
pub const GLOBAL_PROBE_RATE_CAP_PER_SEC: u32 = 32;

// The per-application caps that used to sit here now live in
// `nm_core::endpoint`, beside the code that enforces them: `nm-core` is below this crate
// in the dependency order and could not have referred to them here. They remain what they
// were — five applications, sixteen actively probed endpoints each — and their product
// still deliberately exceeds [`GLOBAL_PROBE_RATE_CAP_PER_SEC`] at a one-second interval,
// because the scheduler is meant to be oversubscribed and to answer by stretching
// intervals rather than by abandoning targets.
