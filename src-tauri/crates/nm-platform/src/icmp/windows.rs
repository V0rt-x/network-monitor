//! ICMP echo on Windows, via the IP Helper API.
//!
//! `IcmpSendEcho2Ex` is the reason this product needs no administrator rights and no
//! packet-capture driver: it sends real ICMP echo requests through a documented,
//! read-only OS service, so nothing here is visible to an anti-cheat system as anything
//! other than an ordinary application.
//!
//! Only IPv4 is implemented. IPv6 needs `Icmp6SendEcho2`, which takes `sockaddr_in6`
//! structures for both endpoints and returns a differently shaped reply; until that
//! exists, an IPv6 target fails with [`Error::Ipv6Unsupported`] rather than being
//! silently reported as unreachable.

use std::ffi::c_void;
use std::net::{IpAddr, Ipv4Addr};
use std::ptr;
use std::time::Instant;

use windows_sys::Win32::Foundation::{GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho2Ex, ICMP_ECHO_REPLY, IP_OPTION_INFORMATION,
};

use super::{
    classify_status, from_in_addr, to_in_addr, EchoOutcome, EchoRequest, IcmpProber, StatusKind,
};
use crate::Error;

/// Largest echo payload this implementation will send.
///
/// Well under any path MTU, so a probe never becomes the thing that gets fragmented or
/// dropped. Buffers are sized for this at compile time, which keeps every probe free of
/// heap allocation.
const MAX_PAYLOAD_LEN: usize = 512;

/// Filler byte for the payload. `ping` uses ASCII letters; the contents are irrelevant
/// beyond being non-zero and compressible-looking like ordinary traffic.
const PAYLOAD_FILL: u8 = b'a';

/// The API requires room for the reply structure, the echoed payload, and eight bytes
/// for an ICMP error message.
const REPLY_CAPACITY: usize = size_of::<ICMP_ECHO_REPLY>() + MAX_PAYLOAD_LEN + 8;

/// Reply buffer with pointer alignment.
///
/// The API writes an `ICMP_ECHO_REPLY` here, which contains a pointer field; giving the
/// buffer the alignment that structure expects avoids relying on an unaligned write
/// being tolerated. Reads still go through `read_unaligned` so correctness does not
/// depend on the assumption.
#[repr(C, align(8))]
struct ReplyBuffer([u8; REPLY_CAPACITY]);

/// An open IP Helper ICMP handle, closed when dropped.
struct IcmpHandle(HANDLE);

impl IcmpHandle {
    fn open() -> Result<Self, Error> {
        // SAFETY: `IcmpCreateFile` takes no arguments and has no preconditions. It
        // returns either a handle we own or INVALID_HANDLE_VALUE, which is checked
        // before the handle is used or stored.
        let handle = unsafe { IcmpCreateFile() };
        if handle == INVALID_HANDLE_VALUE {
            // SAFETY: `GetLastError` only reads the calling thread's last-error slot.
            return Err(Error::Icmp {
                code: unsafe { GetLastError() },
            });
        }
        Ok(Self(handle))
    }
}

impl Drop for IcmpHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from a successful `IcmpCreateFile` and is closed exactly
        // once, because `IcmpHandle` is neither `Copy` nor `Clone` and owns the handle
        // for its whole lifetime.
        unsafe {
            IcmpCloseHandle(self.0);
        }
    }
}

/// Sends ICMP echo requests through the Windows IP Helper API.
///
/// Each request opens and closes its own handle. That costs two cheap calls per probe —
/// negligible against a 32 probes/second budget — and in exchange the handle is never
/// shared between threads, so the API's undocumented concurrency behaviour never has to
/// be relied upon.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsIcmpProber;

impl IcmpProber for WindowsIcmpProber {
    fn echo(&self, request: &EchoRequest) -> Result<EchoOutcome, Error> {
        let IpAddr::V4(target) = request.target else {
            return Err(Error::Ipv6Unsupported);
        };
        let source = match request.source {
            None => Ipv4Addr::UNSPECIFIED,
            Some(IpAddr::V4(address)) => address,
            Some(IpAddr::V6(_)) => return Err(Error::Ipv6Unsupported),
        };

        let payload_len = usize::from(request.payload_len).min(MAX_PAYLOAD_LEN);
        let payload = [PAYLOAD_FILL; MAX_PAYLOAD_LEN];
        let mut reply = ReplyBuffer([0; REPLY_CAPACITY]);

        let options = IP_OPTION_INFORMATION {
            Ttl: request.ttl.unwrap_or_default(),
            Tos: 0,
            Flags: 0,
            OptionsSize: 0,
            OptionsData: ptr::null_mut(),
        };
        // A null options pointer means "system defaults", which is what we want unless a
        // path probe asked for a specific TTL.
        let options_ptr: *const IP_OPTION_INFORMATION = if request.ttl.is_some() {
            &raw const options
        } else {
            ptr::null()
        };

        let timeout_ms = u32::try_from(request.timeout.as_millis()).unwrap_or(u32::MAX);
        let payload_len_u16 = u16::try_from(payload_len).unwrap_or(u16::MAX);
        let reply_capacity = u32::try_from(REPLY_CAPACITY).unwrap_or(u32::MAX);

        let handle = IcmpHandle::open()?;

        // Timed here rather than read from the reply's `RoundTripTime`, which the API
        // reports in whole milliseconds. That resolution would quantise away exactly the
        // sub-millisecond variation that jitter is made of, and would show a 0.4 ms LAN
        // hop as 0 ms.
        let started = Instant::now();

        // SAFETY: every pointer below is valid for the duration of the call and for the
        // length passed alongside it:
        //  * `handle.0` is a live handle owned by `handle`, which outlives this call.
        //  * The event and APC parameters are null/None, selecting the synchronous form,
        //    so the API does not retain any pointer past its return.
        //  * `payload` is a live stack array of MAX_PAYLOAD_LEN bytes and
        //    `payload_len_u16 <= MAX_PAYLOAD_LEN`.
        //  * `options_ptr` is either null or points at `options`, live for this scope.
        //  * `reply.0` is a live stack array of exactly `reply_capacity` bytes, which
        //    meets the API's documented minimum of reply + payload + 8.
        let replies = unsafe {
            IcmpSendEcho2Ex(
                handle.0,
                ptr::null_mut(),
                None,
                ptr::null(),
                to_in_addr(source),
                to_in_addr(target),
                payload.as_ptr().cast::<c_void>(),
                payload_len_u16,
                options_ptr,
                (&raw mut reply.0).cast::<c_void>(),
                reply_capacity,
                timeout_ms,
            )
        };
        let elapsed = started.elapsed();

        if replies == 0 {
            // With no reply structure the API reports the outcome through the last-error
            // slot, and the responding address is simply not available.
            // SAFETY: `GetLastError` only reads the calling thread's last-error slot.
            let code = unsafe { GetLastError() };
            return match classify_status(code) {
                StatusKind::TimedOut => Ok(EchoOutcome::TimedOut),
                StatusKind::TtlExpired => Ok(EchoOutcome::TtlExpired {
                    from: None,
                    rtt: elapsed,
                }),
                StatusKind::Unreachable => Ok(EchoOutcome::Unreachable { from: None }),
                // "Success with zero replies" is a contradiction, so it is treated as
                // the local failure it must be rather than invented into a measurement.
                StatusKind::Replied | StatusKind::Unusable => Err(Error::Icmp { code }),
            };
        }

        // SAFETY: `replies` is non-zero, so the API wrote at least one `ICMP_ECHO_REPLY`
        // at the start of the buffer, and `REPLY_CAPACITY` exceeds that structure's size.
        // `read_unaligned` copies the bytes out without assuming any alignment.
        let reply: ICMP_ECHO_REPLY = unsafe { ptr::read_unaligned((&raw const reply.0).cast()) };
        let from = IpAddr::V4(from_in_addr(reply.Address));

        match classify_status(reply.Status) {
            StatusKind::Replied => Ok(EchoOutcome::Replied { from, rtt: elapsed }),
            StatusKind::TtlExpired => Ok(EchoOutcome::TtlExpired {
                from: Some(from),
                rtt: elapsed,
            }),
            StatusKind::Unreachable => Ok(EchoOutcome::Unreachable { from: Some(from) }),
            StatusKind::TimedOut => Ok(EchoOutcome::TimedOut),
            StatusKind::Unusable => Err(Error::Icmp { code: reply.Status }),
        }
    }
}
