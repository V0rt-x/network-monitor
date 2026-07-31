//! Enumerating the processes a user can choose to monitor.
//!
//! This is the first half of per-application monitoring: the user picks a running
//! process, and everything downstream — connection-table rows, flow events, probe tags —
//! is keyed by its [`Pid`].
//!
//! **Read-only, and deliberately minimal.** Enumeration goes through the Toolhelp
//! snapshot, which reports a process's identity without opening a handle to it. Nothing
//! here reads another process's memory, loads code into it, or asks for a right beyond
//! observation — the app has to be indistinguishable from a task manager to an anti-cheat
//! system, and the cheapest way to guarantee that is to never take a capability we do not
//! need.
//!
//! [`ProcessEnumerator::executable_path`] is separate from the sweep for that same
//! reason, plus a budget one: it is the one call that must open a process handle, so it
//! runs only for a process the user actually selected rather than for the few hundred a
//! desktop has running.
//!
//! Linux implements this over `/proc/<pid>/comm` and `/proc/<pid>/exe`, macOS over
//! `proc_listpids` and `proc_pidpath`; both are read-only in the same way.

use std::fmt;
use std::path::PathBuf;

use crate::Error;

#[cfg(windows)]
pub mod windows;

/// Operating-system process identifier.
///
/// A newtype because a bare `u32` is interchangeable with a port, an interface index or a
/// byte count, and mixing those up is the kind of bug that produces a plausible-looking
/// wrong answer instead of a compile error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pid(u32);

impl Pid {
    /// Wraps a raw OS process identifier.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw identifier, for handing back to an OS call.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A running process, as far as an unprivileged observer can see it.
///
/// Carries no window title, command line or owning user: none of that is needed to
/// measure a network path, and all of it is sensitive on a machine whose owner is under
/// surveillance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Identifier to key everything else by.
    pub pid: Pid,
    /// Executable file name, e.g. `Discord.exe`.
    ///
    /// Not a path and not unique — several copies of one game can run at once, which is
    /// exactly why the [`Pid`] is the identity and this is only a label.
    pub name: String,
}

/// Lists the processes running on this machine.
pub trait ProcessEnumerator: Send + Sync {
    /// Takes a snapshot of the currently running processes.
    ///
    /// The result is a snapshot in the strict sense: a process may exit before the caller
    /// reads the list, and one that started during the call may be missing. Callers must
    /// treat a [`Pid`] as a claim that has to be re-checked, never as a handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the OS refuses the enumeration entirely. A process that
    /// cannot be described individually is skipped rather than failing the sweep.
    fn processes(&self) -> Result<Vec<ProcessInfo>, Error>;

    /// Full path of the executable backing `pid`, when it can be read.
    ///
    /// [`None`] is a legitimate answer, not a failure: a process running as another user
    /// or at a higher integrity level will not open, and a process that exited between
    /// the sweep and this call cannot. The caller falls back to
    /// [`ProcessInfo::name`] — which is always available — rather than showing an error.
    ///
    /// # Errors
    ///
    /// Returns an error only when the call fails for a reason that is *our* problem
    /// rather than a limit on what we are allowed to see.
    fn executable_path(&self, pid: Pid) -> Result<Option<PathBuf>, Error>;
}

impl<E: ProcessEnumerator + ?Sized> ProcessEnumerator for Box<E> {
    fn processes(&self) -> Result<Vec<ProcessInfo>, Error> {
        (**self).processes()
    }

    fn executable_path(&self, pid: Pid) -> Result<Option<PathBuf>, Error> {
        (**self).executable_path(pid)
    }
}

/// The host's process enumerator, if this build has one.
///
/// # Errors
///
/// Returns [`Error::UnsupportedPlatform`] where no backend exists yet.
pub fn system_enumerator() -> Result<Box<dyn ProcessEnumerator>, Error> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsProcessEnumerator))
    }
    #[cfg(not(windows))]
    {
        Err(Error::UnsupportedPlatform)
    }
}

/// Reads a NUL-terminated wide string out of a fixed-size OS buffer.
///
/// Two hazards, both handled: the buffer is padded past its terminator with whatever the
/// OS left there, and Windows file names are not guaranteed to be well-formed UTF-16.
/// The result is lossy on purpose — this string is a label in a process picker, so a
/// replacement character is a better outcome than dropping a process the user is looking
/// for.
pub(crate) fn name_from_wide(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str, capacity: usize) -> Vec<u16> {
        let mut buffer: Vec<u16> = text.encode_utf16().collect();
        buffer.resize(capacity, 0);
        buffer
    }

    #[test]
    fn a_pid_round_trips_through_the_newtype() {
        assert_eq!(Pid::new(4).get(), 4);
        assert_eq!(Pid::new(u32::MAX).get(), u32::MAX);
        assert_eq!(Pid::new(1234).to_string(), "1234");
    }

    #[test]
    fn reads_a_name_up_to_its_terminator() {
        assert_eq!(name_from_wide(&wide("Discord.exe", 260)), "Discord.exe");
    }

    #[test]
    fn ignores_whatever_follows_the_terminator() {
        // Toolhelp does not zero the tail of the buffer, so trailing units are real.
        let mut buffer = wide("cs2.exe", 32);
        buffer[10] = u16::from(b'X');
        buffer[11] = u16::from(b'Y');
        assert_eq!(name_from_wide(&buffer), "cs2.exe");
    }

    #[test]
    fn handles_an_unterminated_buffer() {
        let buffer: Vec<u16> = "abc".encode_utf16().collect();
        assert_eq!(name_from_wide(&buffer), "abc");
    }

    #[test]
    fn an_empty_buffer_is_an_empty_name() {
        assert_eq!(name_from_wide(&[]), "");
        assert_eq!(name_from_wide(&[0; 16]), "");
    }

    #[test]
    fn keeps_non_ascii_names_intact() {
        // Games and launchers really do ship with localized file names.
        assert_eq!(name_from_wide(&wide("Игра.exe", 260)), "Игра.exe");
        assert_eq!(name_from_wide(&wide("原神.exe", 260)), "原神.exe");
    }

    #[test]
    fn a_malformed_name_is_replaced_rather_than_dropped() {
        // A lone surrogate cannot be encoded; losing the whole process entry over it
        // would hide the very application the user is trying to select.
        let buffer = [0xD800, u16::from(b'a'), 0];
        assert_eq!(name_from_wide(&buffer), "\u{FFFD}a");
    }
}
