//! Adapter enumeration on Windows, via `GetAdaptersAddresses`.
//!
//! One call returns every adapter with the addresses assigned to it, and it needs no
//! handle, no privilege and no administrator rights — it is the same information the
//! network settings page shows its own user.
//!
//! Two details make the difference between this working and appearing to:
//!
//! **The friendly name, not the description.** `Description` is the driver's word for the
//! device ("Intel(R) Wi-Fi 6E AX211 160MHz"); `FriendlyName` is what the user has seen and
//! possibly renamed to — Wi-Fi, or the name of the accelerator they installed. The point of
//! naming an adapter at all is that the person reading recognises it.
//!
//! **The buffer must be aligned.** The API writes a linked list of structures into whatever
//! is handed to it, so a `Vec<u8>` — aligned to one byte — would be undefined behaviour to
//! read back as a structure needing eight. The allocation is therefore made of `u64`.

use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, ERROR_SUCCESS};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

use super::{InterfaceTable, NetworkInterface};
use crate::flow::decode_sockaddr;
use crate::Error;

/// Starting buffer, in bytes.
///
/// The documented recommendation is 15 KB, which covers a normal machine in one call; the
/// loop below grows for one with many adapters, which is exactly what a machine running
/// several tunnels is.
const INITIAL_BUFFER_BYTES: usize = 16 * 1024;

/// Ceiling for the buffer, in bytes.
///
/// Reaching it would mean the API wants more than a megabyte to describe this machine's
/// adapters, which is not a machine — it is a bug. The loop stops rather than growing
/// without bound.
const MAX_BUFFER_BYTES: usize = 1024 * 1024;

/// Longest adapter name read, in UTF-16 code units.
///
/// A guard on a NUL-terminated string whose length the API does not report. Windows caps a
/// connection name far below this; the limit exists so a corrupt pointer cannot become an
/// unbounded read.
const MAX_NAME_UNITS: usize = 512;

/// Lists adapters through the Windows IP Helper API.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsInterfaceTable;

impl InterfaceTable for WindowsInterfaceTable {
    fn interfaces(&self) -> Result<Vec<NetworkInterface>, Error> {
        // `u64` rather than `u8`: the API writes structures into this and they must be
        // eight-byte aligned to be read back at all.
        let mut buffer: Vec<u64> = vec![0; INITIAL_BUFFER_BYTES / size_of::<u64>()];
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;

        loop {
            let mut size = u32::try_from(buffer.len() * size_of::<u64>()).unwrap_or(u32::MAX);

            // SAFETY: `buffer` is a live, eight-byte-aligned allocation of `size` bytes,
            // which is what `size` reports on input. The API either fills it and returns
            // success, or writes the size it needs into `size` and returns an overflow. No
            // pointer is retained past this call.
            let status = unsafe {
                GetAdaptersAddresses(
                    u32::from(AF_UNSPEC),
                    flags,
                    std::ptr::null(),
                    buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                    &raw mut size,
                )
            };

            match status {
                // SAFETY: the call succeeded, so `buffer` holds a well-formed linked list of
                // adapter structures whose pointers all point inside it, and it outlives the
                // walk below.
                ERROR_SUCCESS => return Ok(unsafe { collect(buffer.as_ptr().cast()) }),
                // A machine with no adapters at all. Empty, not an error.
                ERROR_NO_DATA => return Ok(Vec::new()),
                ERROR_BUFFER_OVERFLOW => {
                    let wanted = usize::try_from(size).unwrap_or(MAX_BUFFER_BYTES);
                    if wanted > MAX_BUFFER_BYTES {
                        return Err(Error::Os {
                            api: "GetAdaptersAddresses",
                            code: ERROR_BUFFER_OVERFLOW,
                        });
                    }
                    // Rounded up, plus a little slack: an adapter can appear between the
                    // call that reported the size and the call that uses it, and a second
                    // overflow would otherwise be the normal outcome.
                    let units = wanted.div_ceil(size_of::<u64>()) + 64;
                    buffer.clear();
                    buffer.resize(units, 0);
                }
                code => {
                    return Err(Error::Os {
                        api: "GetAdaptersAddresses",
                        code,
                    })
                }
            }
        }
    }
}

/// Walks the adapter list the API wrote.
///
/// # Safety
///
/// `head` must be the start of a linked list of `IP_ADAPTER_ADDRESSES_LH` as
/// `GetAdaptersAddresses` writes it, in an allocation that outlives this call.
unsafe fn collect(head: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<NetworkInterface> {
    let mut interfaces = Vec::new();
    let mut adapter = head;

    while !adapter.is_null() {
        // SAFETY: the pointer is non-null and, by this function's contract, points at a
        // live adapter structure inside the caller's allocation.
        let current = unsafe { &*adapter };

        // SAFETY: `FriendlyName` is a NUL-terminated wide string the API wrote into the
        // same allocation, and the read is bounded regardless.
        let name = unsafe { read_wide(current.FriendlyName, MAX_NAME_UNITS) };

        let mut addresses = Vec::new();
        let mut unicast = current.FirstUnicastAddress;
        while !unicast.is_null() {
            // SAFETY: same contract — a live entry of this adapter's unicast list.
            let entry = unsafe { &*unicast };
            let length = usize::try_from(entry.Address.iSockaddrLength).unwrap_or(0);
            if !entry.Address.lpSockaddr.is_null() && length > 0 {
                // SAFETY: the API reports the length of the socket address it wrote, and
                // the bytes live in the caller's allocation for the length of this walk.
                let bytes = unsafe {
                    std::slice::from_raw_parts(entry.Address.lpSockaddr.cast::<u8>(), length)
                };
                if let Some(socket) = decode_sockaddr(bytes) {
                    addresses.push(socket.ip());
                }
            }
            unicast = entry.Next;
        }

        // An adapter with no address cannot name anything, and a nameless one names nothing
        // a user would recognise. Neither is an error; both are simply not useful here.
        if !name.is_empty() && !addresses.is_empty() {
            interfaces.push(NetworkInterface { name, addresses });
        }
        adapter = current.Next;
    }

    interfaces
}

/// Reads a NUL-terminated wide string, up to `limit` code units.
///
/// # Safety
///
/// `text` must be null, or point at a NUL-terminated wide string that stays valid for the
/// length of this call.
unsafe fn read_wide(text: *const u16, limit: usize) -> String {
    if text.is_null() {
        return String::new();
    }
    let mut units = Vec::new();
    for offset in 0..limit {
        // SAFETY: by contract the string is NUL-terminated within `limit` units, and the
        // loop stops at the terminator before reading past it.
        let unit = unsafe { *text.add(offset) };
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    // Lossy on purpose: an adapter the user renamed to something unrepresentable is still
    // an adapter, and dropping it would leave an endpoint with no name at all.
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn the_machine_has_at_least_one_named_adapter_with_an_address() {
        let interfaces = WindowsInterfaceTable
            .interfaces()
            .expect("enumerating adapters needs no privilege");

        assert!(!interfaces.is_empty(), "a running machine has adapters");
        assert!(interfaces
            .iter()
            .all(|interface| !interface.name.is_empty() && !interface.addresses.is_empty()));
    }

    #[test]
    fn loopback_is_present_and_named() {
        // The one address every Windows machine has, on an adapter it always describes —
        // so this asserts the walk really reached the unicast list rather than stopping at
        // the first adapter.
        let interfaces = WindowsInterfaceTable.interfaces().unwrap();
        let names = super::super::InterfaceNames::of(&interfaces);

        assert!(
            names.name_of(IpAddr::V4(Ipv4Addr::LOCALHOST)).is_some(),
            "127.0.0.1 belongs to an adapter this machine describes"
        );
    }

    #[test]
    fn an_enumeration_is_cheap_enough_to_repeat_while_monitoring() {
        // Re-read while the app runs, because a VPN coming up is exactly the event the
        // egress labels exist to make visible. Measured rather than assumed; the ceiling is
        // loose enough for a busy machine and tight enough to catch a call that became
        // tens of milliseconds.
        let rounds = 20;
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            WindowsInterfaceTable.interfaces().unwrap();
        }
        let each = started.elapsed() / rounds;
        eprintln!("adapter enumeration: {each:?}");
        assert!(
            each < std::time::Duration::from_millis(20),
            "enumerating adapters took {each:?}"
        );
    }
}
