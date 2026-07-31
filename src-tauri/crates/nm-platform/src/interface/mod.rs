//! Which adapter an address leaves the machine by.
//!
//! Everything else in this product measures a *route*; this names the first step of it. The
//! app already binds each probe to the same local address the application's own flow uses,
//! so the two follow the same interface — and an address is not something a user can check
//! that against. A name — Wi-Fi, Ethernet, the accelerator's own adapter, a tunnel — is.
//!
//! That matters most in the case the product exists for. A user turning a VPN or a game
//! accelerator on wants to compare before and after, and the only way to know which they are
//! looking at is to be told which adapter the traffic is leaving by. And where a probe
//! cannot follow an application — two applications reaching one endpoint by different routes,
//! or an address a baseline was already probing — naming both adapters turns a warning that
//! something might be wrong into a statement of what is.
//!
//! **Read-only and cheap.** One call returns every adapter with its addresses; nothing is
//! opened, changed or configured. Linux reads the same from netlink (`RTM_GETLINK` /
//! `RTM_GETADDR`) or `/sys/class/net`, macOS from `getifaddrs`.

use std::collections::HashMap;
use std::net::IpAddr;

use crate::Error;

#[cfg(windows)]
pub mod windows;

/// One network adapter, as far as naming a route's first step needs it.
///
/// Carries no MAC address, no configuration and no statistics: none of that is needed to
/// say which adapter an address belongs to, and all of it describes a real person's network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkInterface {
    /// The name the operating system shows its own user for this adapter.
    ///
    /// Not the device description and not an identifier: the point is that the user can
    /// recognise it, and what they have seen is whatever they renamed it to.
    pub name: String,
    /// Local addresses assigned to it.
    pub addresses: Vec<IpAddr>,
}

/// Lists the machine's network adapters.
pub trait InterfaceTable: Send + Sync {
    /// Takes a snapshot of the adapters and the addresses assigned to them.
    ///
    /// A snapshot in the strict sense: a tunnel can appear or vanish between two calls,
    /// which is exactly what happens when the user toggles a VPN, and is the reason this is
    /// re-read rather than cached for the life of the session.
    ///
    /// # Errors
    ///
    /// Returns an error when the OS refuses the enumeration.
    fn interfaces(&self) -> Result<Vec<NetworkInterface>, Error>;
}

impl<T: InterfaceTable + ?Sized> InterfaceTable for Box<T> {
    fn interfaces(&self) -> Result<Vec<NetworkInterface>, Error> {
        (**self).interfaces()
    }
}

/// The host's interface table, if this build has one.
///
/// # Errors
///
/// Returns [`Error::UnsupportedPlatform`] where no backend exists yet.
pub fn system_table() -> Result<Box<dyn InterfaceTable>, Error> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsInterfaceTable))
    }
    #[cfg(not(windows))]
    {
        Err(Error::UnsupportedPlatform)
    }
}

/// Adapter names, looked up by the local address that leaves through them.
///
/// The shape the rest of the app actually asks for: it holds a local address — the one a
/// probe is bound to — and wants the adapter's name. Built once per refresh and then read
/// once per endpoint, because the question is asked far more often than the answer changes.
///
/// Platform-free, so the mapping is tested on any operating system.
#[derive(Debug, Clone, Default)]
pub struct InterfaceNames {
    by_address: HashMap<IpAddr, String>,
}

impl InterfaceNames {
    /// Indexes a snapshot by address.
    ///
    /// Where two adapters somehow claim one address the first wins, and the choice does not
    /// matter: the name is a label beside a number, never something a measurement depends on.
    #[must_use]
    pub fn of(interfaces: &[NetworkInterface]) -> Self {
        let mut by_address = HashMap::new();
        for interface in interfaces {
            for address in &interface.addresses {
                by_address
                    .entry(*address)
                    .or_insert_with(|| interface.name.clone());
            }
        }
        Self { by_address }
    }

    /// The adapter an address belongs to, if the snapshot knows it.
    ///
    /// [`None`] is a normal answer, not a failure: an address can be released between the
    /// snapshot and the question — again, a VPN going down — and a label that guessed would
    /// be worse than one that is absent.
    #[must_use]
    pub fn name_of(&self, address: IpAddr) -> Option<&str> {
        self.by_address.get(&address).map(String::as_str)
    }

    /// Whether nothing is known — no backend, or an enumeration that failed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_address.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    fn v4(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
    }

    fn interface(name: &str, addresses: &[IpAddr]) -> NetworkInterface {
        NetworkInterface {
            name: name.to_owned(),
            addresses: addresses.to_vec(),
        }
    }

    #[test]
    fn an_address_is_named_after_the_adapter_that_carries_it() {
        let names = InterfaceNames::of(&[
            interface("Ethernet", &[v4(10)]),
            interface("Tunnel", &[v4(20), IpAddr::V6(Ipv6Addr::LOCALHOST)]),
        ]);

        assert_eq!(names.name_of(v4(10)), Some("Ethernet"));
        assert_eq!(names.name_of(v4(20)), Some("Tunnel"));
        assert_eq!(
            names.name_of(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            Some("Tunnel")
        );
    }

    #[test]
    fn an_address_no_adapter_claims_is_unnamed_rather_than_guessed() {
        // A tunnel that went down between the snapshot and the question. A label that
        // guessed would be worse than one that is absent.
        let names = InterfaceNames::of(&[interface("Ethernet", &[v4(10)])]);
        assert_eq!(names.name_of(v4(99)), None);
    }

    #[test]
    fn an_adapter_with_several_addresses_names_all_of_them() {
        let names = InterfaceNames::of(&[interface("Wi-Fi", &[v4(1), v4(2), v4(3)])]);
        for last in [1, 2, 3] {
            assert_eq!(names.name_of(v4(last)), Some("Wi-Fi"));
        }
    }

    #[test]
    fn an_empty_snapshot_names_nothing_and_says_so() {
        let names = InterfaceNames::of(&[]);
        assert!(names.is_empty());
        assert_eq!(names.name_of(v4(10)), None);

        assert!(InterfaceNames::default().is_empty());
    }

    #[test]
    fn an_adapter_with_no_addresses_contributes_nothing() {
        let names = InterfaceNames::of(&[interface("Disconnected", &[])]);
        assert!(names.is_empty());
    }

    #[test]
    fn a_contested_address_takes_the_first_adapter_that_claims_it() {
        // It cannot normally happen, and if it does the name is a label beside a number
        // rather than something a measurement depends on. What matters is that it is
        // decided rather than left to hash order.
        let names = InterfaceNames::of(&[
            interface("First", &[v4(10)]),
            interface("Second", &[v4(10)]),
        ]);
        assert_eq!(names.name_of(v4(10)), Some("First"));
    }
}
