//! Pure domain logic for Network Monitor.
//!
//! `nm-core` owns every calculation the product makes: metric types, sample history,
//! sliding-window statistics, probe scheduling models and diagnosis rules. It is the
//! bottom of the dependency chain
//! (`nm-core` <- `nm-probes` <- `nm-app` -> `nm-platform`) and therefore must never
//! depend on an operating system, an async runtime, or Tauri. Everything here is
//! synchronous, allocation-frugal and unit-testable on any machine.
//!
//! Phase 0 establishes the crate; the metric types and statistics arrive in Phase 1
//! (see `PLAN.md`), together with the first fallible operations and this crate's
//! `thiserror` enum.

#![warn(missing_docs)]

pub mod time;

/// Semantic version of the monitoring core, surfaced to the UI on startup.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
