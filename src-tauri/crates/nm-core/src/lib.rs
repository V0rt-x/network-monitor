//! Pure domain logic for Network Monitor.
//!
//! `nm-core` owns every calculation the product makes: metric types, sample history,
//! sliding-window statistics, probe scheduling models and diagnosis rules. It is the
//! bottom of the dependency chain
//! (`nm-core` <- `nm-probes` <- `nm-app` -> `nm-platform`) and therefore never depends on
//! an operating system, an async runtime, or Tauri. Everything here is synchronous,
//! allocation-frugal and unit-testable on any machine.
//!
//! # How the pieces fit
//!
//! [`target::TargetRegistry`] says *what* may be probed and why. [`scheduler`] decides
//! *when*, within a global rate cap. The probe engine carries out the work and hands each
//! result back as a [`sample::ProbeSample`], which [`history::SampleHistory`] retains in a
//! fixed-capacity [`ring::RingBuffer`]. [`stats`] turns a window of that history into the
//! numbers the UI shows.
//!
//! # Two rules the whole crate obeys
//!
//! **Time is monotonic and injected.** Measurements are stamped with [`std::time::Instant`];
//! wall-clock time is only ever used for display and persistence. No type here reads a
//! clock of its own — callers pass `now` in, which is what makes an entire day of
//! scheduling reproducible in a millisecond of test time.
//!
//! **Missing knowledge is [`None`], never zero.** A window with no delivery test reports
//! no loss percentage rather than `0 %`; a filtered probe is not counted as a lost packet.
//! A confident-looking number the data does not support is worse than an admitted gap.

#![warn(missing_docs)]
// CLAUDE.md confines `unsafe` to `nm-platform`; here that rule is compiler-enforced
// rather than a convention someone has to remember.
#![forbid(unsafe_code)]

mod error;

pub mod address;
pub mod cidr;
pub mod history;
pub mod path;
pub mod ring;
pub mod sample;
pub mod scheduler;
pub mod stats;
pub mod target;
pub mod time;

pub use error::Error;

/// Semantic version of the monitoring core, surfaced to the UI on startup.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
