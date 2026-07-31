//! Process enumeration on Windows, via the Toolhelp snapshot.
//!
//! `CreateToolhelp32Snapshot` reports every process's identifier and executable name
//! without opening a handle to any of them, which is the whole reason it is preferred
//! here over `EnumProcesses`: the sweep touches no process, so there is nothing for an
//! anti-cheat driver to notice and nothing for the OS to deny.
//!
//! The one call that does need a handle, [`WindowsProcessEnumerator::executable_path`],
//! asks for `PROCESS_QUERY_LIMITED_INFORMATION` — the weakest right that exists, which
//! grants the image path and nothing else. It cannot read memory, and it is what Task
//! Manager uses for the same purpose. It still runs only for a process the user picked.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_GEN_FAILURE, ERROR_INSUFFICIENT_BUFFER,
    ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, FALSE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::{name_from_wide, Pid, ProcessEnumerator, ProcessInfo};
use crate::Error;

/// Starting capacity for the process list.
///
/// An idle Windows 11 desktop runs a few hundred processes; sizing for that avoids a
/// handful of reallocations per picker refresh at a cost of a few kilobytes.
const EXPECTED_PROCESS_COUNT: usize = 384;

/// Initial buffer for an image path, in UTF-16 code units.
///
/// `MAX_PATH` covers almost everything; the loop below grows for the long paths that
/// Windows has allowed since the path-length limit was lifted.
const INITIAL_PATH_CAPACITY: usize = 260;

/// Ceiling for the image-path buffer, in UTF-16 code units.
///
/// The documented maximum for an extended-length path. Reaching it means the API is
/// asking for more than Windows can produce, so the loop stops rather than growing
/// without bound.
const MAX_PATH_CAPACITY: usize = 32_768;

/// An open Toolhelp snapshot handle, closed when dropped.
struct SnapshotHandle(HANDLE);

impl SnapshotHandle {
    fn open() -> Result<Self, Error> {
        // SAFETY: the flags and the pid argument are plain values; the call allocates a
        // snapshot and returns either a handle we own or INVALID_HANDLE_VALUE, which is
        // checked before the handle is stored or used.
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if handle == INVALID_HANDLE_VALUE {
            // SAFETY: `GetLastError` only reads the calling thread's last-error slot.
            return Err(Error::Os {
                api: "CreateToolhelp32Snapshot",
                code: unsafe { GetLastError() },
            });
        }
        Ok(Self(handle))
    }
}

impl Drop for SnapshotHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from a successful `CreateToolhelp32Snapshot` and is
        // closed exactly once — the type is neither `Copy` nor `Clone` and owns the
        // handle for its whole lifetime.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// A process handle opened for observation only, closed when dropped.
struct ProcessHandle(HANDLE);

impl ProcessHandle {
    /// Opens `pid` for querying, or reports that it cannot be observed.
    ///
    /// `Ok(None)` covers the two answers that are facts about permission rather than
    /// failures of ours: the process belongs to another user or runs at a higher
    /// integrity level, or it no longer exists.
    fn open_for_query(pid: Pid) -> Result<Option<Self>, Error> {
        // SAFETY: no pointers are involved; the call returns either a handle we own or
        // null, which is checked before the handle is stored.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid.get()) };
        if handle.is_null() {
            // SAFETY: `GetLastError` only reads the calling thread's last-error slot.
            let code = unsafe { GetLastError() };
            return match code {
                ERROR_ACCESS_DENIED | ERROR_INVALID_PARAMETER | ERROR_GEN_FAILURE => Ok(None),
                other => Err(Error::Os {
                    api: "OpenProcess",
                    code: other,
                }),
            };
        }
        Ok(Some(Self(handle)))
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from a successful `OpenProcess` and is closed exactly
        // once, for the same ownership reason as `SnapshotHandle`.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Reads a Toolhelp parent identifier, refusing the ones that name nothing.
///
/// Zero is what Toolhelp writes when it will not name a creator, and pid 0 is the idle
/// process rather than a parent anything could have. Turning it into `Some` would put every
/// such process under one imaginary root, which for a rule that adopts descendants is the
/// difference between "the launcher's children" and "half the machine".
const fn parent_of(raw: u32) -> Option<Pid> {
    if raw == 0 {
        None
    } else {
        Some(Pid::new(raw))
    }
}

/// Lists processes through the Windows Toolhelp API.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsProcessEnumerator;

impl ProcessEnumerator for WindowsProcessEnumerator {
    fn processes(&self) -> Result<Vec<ProcessInfo>, Error> {
        let snapshot = SnapshotHandle::open()?;

        let mut entry = PROCESSENTRY32W {
            // The API rejects the structure outright unless it is told its own size —
            // that is how it version-checks the caller.
            dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(u32::MAX),
            ..PROCESSENTRY32W::default()
        };

        // SAFETY: `snapshot` is a live snapshot handle and `entry` is a live, correctly
        // sized structure with `dwSize` filled in as the API requires.
        if unsafe { Process32FirstW(snapshot.0, &raw mut entry) } == FALSE {
            // SAFETY: `GetLastError` only reads the calling thread's last-error slot.
            let code = unsafe { GetLastError() };
            // An empty snapshot is not a thing that happens — this process is in it —
            // but treating "no more files" as an empty list rather than an error keeps
            // the contract honest if it ever does.
            return if code == ERROR_NO_MORE_FILES {
                Ok(Vec::new())
            } else {
                Err(Error::Os {
                    api: "Process32FirstW",
                    code,
                })
            };
        }

        let mut processes = Vec::with_capacity(EXPECTED_PROCESS_COUNT);
        loop {
            processes.push(ProcessInfo {
                pid: Pid::new(entry.th32ProcessID),
                name: name_from_wide(&entry.szExeFile),
                // Zero is not a process: Toolhelp writes it for a system process whose
                // creator it will not name, and reading it as a parent would make every
                // such process a child of the same fiction.
                parent: parent_of(entry.th32ParentProcessID),
            });

            // SAFETY: same invariants as the `Process32FirstW` call above; the snapshot
            // outlives the loop and `entry` is repopulated in place on each iteration.
            if unsafe { Process32NextW(snapshot.0, &raw mut entry) } == FALSE {
                // SAFETY: `GetLastError` only reads the calling thread's last-error slot.
                let code = unsafe { GetLastError() };
                return if code == ERROR_NO_MORE_FILES {
                    Ok(processes)
                } else {
                    // Anything else means the walk was cut short. Returning the partial
                    // list would present a running game as "not running".
                    Err(Error::Os {
                        api: "Process32NextW",
                        code,
                    })
                };
            }
        }
    }

    fn executable_path(&self, pid: Pid) -> Result<Option<PathBuf>, Error> {
        let Some(process) = ProcessHandle::open_for_query(pid)? else {
            return Ok(None);
        };

        let mut buffer = vec![0u16; INITIAL_PATH_CAPACITY];
        loop {
            let mut length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);

            // SAFETY: `process.0` is a live handle opened with the right this call needs;
            // `buffer` is a live allocation of `length` code units, which is what
            // `length` reports on input. On success the API writes the character count
            // back into `length` without ever exceeding the value passed in.
            let ok = unsafe {
                QueryFullProcessImageNameW(
                    process.0,
                    PROCESS_NAME_WIN32,
                    buffer.as_mut_ptr(),
                    &raw mut length,
                )
            };

            if ok != FALSE {
                let written = usize::try_from(length).unwrap_or(0).min(buffer.len());
                return Ok(Some(PathBuf::from(OsString::from_wide(&buffer[..written]))));
            }

            // SAFETY: `GetLastError` only reads the calling thread's last-error slot.
            let code = unsafe { GetLastError() };
            match code {
                ERROR_INSUFFICIENT_BUFFER if buffer.len() < MAX_PATH_CAPACITY => {
                    buffer.resize((buffer.len() * 2).min(MAX_PATH_CAPACITY), 0);
                }
                // The process exited while we were asking. That is a race, not a fault.
                ERROR_INVALID_PARAMETER | ERROR_GEN_FAILURE => return Ok(None),
                other => {
                    return Err(Error::Os {
                        api: "QueryFullProcessImageNameW",
                        code: other,
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_process_is_in_the_snapshot() {
        let processes = WindowsProcessEnumerator
            .processes()
            .expect("the Toolhelp snapshot is available to any unprivileged process");

        let me = Pid::new(std::process::id());
        let entry = processes
            .iter()
            .find(|process| process.pid == me)
            .expect("this process must appear in its own snapshot");

        assert!(
            entry.name.to_ascii_lowercase().ends_with(".exe"),
            "a process name should be an executable file name"
        );
    }

    #[test]
    fn a_snapshot_holds_the_whole_system() {
        let processes = WindowsProcessEnumerator.processes().unwrap();

        // A running Windows session has dozens of processes; a handful would mean the
        // walk stopped early and quietly returned what it had.
        assert!(processes.len() > 20, "{} processes", processes.len());
        assert!(processes.iter().all(|process| !process.name.is_empty()));
    }

    #[test]
    fn the_current_process_reports_its_own_image_path() {
        let path = WindowsProcessEnumerator
            .executable_path(Pid::new(std::process::id()))
            .expect("querying our own process cannot be denied")
            .expect("our own image path is always readable");

        // Compared by file name, not against `current_exe`: the API returns the path with
        // every junction and symbolic link resolved, so on a machine whose build
        // directory is a junction the two spellings differ while naming the same file.
        let expected = std::env::current_exe().unwrap();
        assert_eq!(path.file_name(), expected.file_name());
        assert!(path.is_absolute(), "an image path is always absolute");
        assert!(path.is_file(), "the image of a running process exists");
    }

    #[test]
    fn a_parent_identifier_is_reported_and_resolves_inside_the_same_snapshot() {
        let processes = WindowsProcessEnumerator.processes().unwrap();

        let me = processes
            .iter()
            .find(|process| process.pid == Pid::new(std::process::id()))
            .unwrap();
        let parent = me
            .parent
            .expect("a test harness is always started by something");
        assert!(
            processes.iter().any(|process| process.pid == parent),
            "the parent of a live process is normally in the same snapshot"
        );
    }

    #[test]
    fn the_idle_process_is_nobodys_parent() {
        // Toolhelp writes zero for a process whose creator it will not name. Read as a
        // parent, that single fiction would adopt half the machine into one application.
        assert_eq!(parent_of(0), None);
        assert_eq!(parent_of(4), Some(Pid::new(4)));

        let processes = WindowsProcessEnumerator.processes().unwrap();
        assert!(processes
            .iter()
            .all(|process| process.parent != Some(Pid::new(0))));
    }

    #[test]
    fn a_sweep_is_cheap_enough_to_repeat_on_the_discovery_beat() {
        // Application membership is recomputed from a fresh sweep every few seconds, so
        // what this costs is a standing charge against the < 1 % CPU budget rather than a
        // one-off. Measured rather than assumed: the ceiling is deliberately loose (a
        // busy machine running the whole test suite is not a quiet desktop), and it is
        // there to catch a sweep that became tens of milliseconds, which would make the
        // periodic refresh untenable and have to move off the beat.
        let rounds = 20;
        let started = std::time::Instant::now();
        let mut counted = 0;
        for _ in 0..rounds {
            counted += WindowsProcessEnumerator.processes().unwrap().len();
        }
        let each = started.elapsed() / rounds;
        eprintln!(
            "process sweep: {each:?} for {} processes",
            counted / rounds as usize
        );
        assert!(
            each < std::time::Duration::from_millis(25),
            "a process sweep took {each:?}"
        );
    }

    #[test]
    fn a_process_that_does_not_exist_is_absent_rather_than_an_error() {
        // Windows pids are multiples of four, so an odd one can never be live. Being
        // told "no such process" must read as absence — a picker that showed an error
        // toast every time a process exited would be unusable.
        let missing = WindowsProcessEnumerator
            .executable_path(Pid::new(0xFFFF_FFFF))
            .expect("an unknown pid is not our failure");
        assert_eq!(missing, None);
    }
}
