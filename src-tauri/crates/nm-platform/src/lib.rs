//! Operating-system abstraction layer for Network Monitor.
//!
//! This is the **only** crate allowed to contain `#[cfg(windows)]`/`#[cfg(unix)]`
//! branches or `unsafe` blocks. Every OS capability the product needs (process
//! enumeration, connection tables, flow events, ICMP probing) is expressed here as a
//! trait so that `nm-probes` and `nm-app` stay platform-free and testable with
//! `mockall` doubles on any development machine.
//!
//! Windows is implemented first; the Linux (`sock_diag` netlink, `/proc`) and macOS
//! (`libproc`) paths must remain plausible for every trait added here.

#![warn(missing_docs)]

mod error;

pub use error::Error;

/// The host platforms Network Monitor targets.
///
/// Deliberately *not* `#[non_exhaustive]`: this workspace is the only consumer, and
/// adding a platform must break every `match` that has to learn about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostPlatform {
    /// Windows — the first-class target.
    Windows,
    /// Linux — planned; see the crate documentation for the intended API path.
    Linux,
    /// macOS — planned; see the crate documentation for the intended API path.
    MacOs,
}

impl HostPlatform {
    /// Stable machine-readable identifier for logs and persisted data.
    ///
    /// Renaming one of these breaks previously written files, so treat them as a
    /// contract rather than a display label.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOs => "macos",
        }
    }

    /// The platform this binary was compiled for.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedPlatform`] when built for a target the crate has no
    /// implementation path for.
    pub const fn current() -> Result<Self, Error> {
        #[cfg(windows)]
        {
            Ok(Self::Windows)
        }
        #[cfg(target_os = "linux")]
        {
            Ok(Self::Linux)
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self::MacOs)
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_is_supported() {
        let platform = HostPlatform::current().expect("dev/CI hosts are windows, linux or macos");
        assert!(matches!(
            platform,
            HostPlatform::Windows | HostPlatform::Linux | HostPlatform::MacOs
        ));
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        // These strings end up in logs and persisted data; a rename is a breaking change.
        assert_eq!(HostPlatform::Windows.id(), "windows");
        assert_eq!(HostPlatform::Linux.id(), "linux");
        assert_eq!(HostPlatform::MacOs.id(), "macos");
    }

    #[test]
    fn current_matches_compile_target() {
        let expected = if cfg!(windows) {
            HostPlatform::Windows
        } else if cfg!(target_os = "linux") {
            HostPlatform::Linux
        } else {
            HostPlatform::MacOs
        };
        assert_eq!(HostPlatform::current().unwrap(), expected);
    }
}
