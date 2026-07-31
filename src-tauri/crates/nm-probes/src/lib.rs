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

pub mod icmp;
pub mod path;
pub mod probe;
pub mod tcp;
pub mod tls;

pub use error::Error;

/// Hard ceiling on probes issued per second across everything the app monitors:
/// per-app endpoints, both baselines and the service status page combined.
///
/// This is a product promise, not a tuning knob — see the resource budget in
/// `CLAUDE.md`.
pub const GLOBAL_PROBE_RATE_CAP_PER_SEC: u32 = 32;

/// Maximum number of applications the user may monitor at the same time.
pub const MAX_MONITORED_APPS: u32 = 5;

/// Maximum number of endpoints actively probed per monitored application.
///
/// Endpoints beyond this count are not dropped: they demote to infrequent probing,
/// prioritized by recent traffic.
///
/// Note that `MAX_MONITORED_APPS * MAX_ACTIVE_ENDPOINTS_PER_APP` deliberately exceeds
/// [`GLOBAL_PROBE_RATE_CAP_PER_SEC`] at the default one-probe-per-second interval: the
/// scheduler is expected to be oversubscribed and must respond by stretching intervals
/// for low-priority targets, never by silently dropping them.
pub const MAX_ACTIVE_ENDPOINTS_PER_APP: u32 = 16;
