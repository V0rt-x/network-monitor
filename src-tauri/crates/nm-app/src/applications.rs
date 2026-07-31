//! What the user actually chose: an application, not a process.
//!
//! A desktop application is several processes. Discord is a main process and a handful of
//! helpers sharing its executable; a launcher spawns the title as a child of itself; an
//! anti-cheat shim re-launches the game, so the process identifier the user clicked can be
//! dead before the first packet of the match. Nobody wants to know which of them opened the
//! socket — they asked to watch *Discord* and *Apex*.
//!
//! This module owns that grouping and nothing else. It reads a process snapshot and decides
//! which processes belong to which monitored application; everything downstream — the
//! endpoint tracker, the probe targets, the caps, the event the UI renders — is keyed by
//! [`AppId`] and never sees a process identifier again.
//!
//! # The rule, and why it is these three parts
//!
//! 1. **The process the user picked.** Always, whatever else follows.
//! 2. **Every process running under one of the application's names.** For an application
//!    with a [preset](crate::presets) those are the preset's executables; for anything else
//!    it is the picked process's own executable name. This is what catches an Electron
//!    application's helpers, which really are the same program.
//! 3. **Every descendant of a member that has *just appeared*.** The only relation that joins
//!    a launcher to the title it starts, and the only one that survives an anti-cheat
//!    re-launch.
//!
//! # Why a descendant must be new
//!
//! Found by running the build against a live game: an Apple device service turned up inside
//! Apex Legends. Windows does not clear a process's recorded parent identifier when the
//! parent exits, and it reissues identifiers freely — so a service started hours ago can
//! name a dead parent whose number the game later received, and the tree rule believes it.
//! The game's endpoint list then holds another program's traffic, which is a wrong
//! measurement rather than an untidy one.
//!
//! A process is therefore adopted through the tree only if it was **absent from the previous
//! snapshot**. A title its launcher really did start appears after the launcher is already a
//! member, so it is caught on the next discovery beat; a service that was running all along
//! never is. What this costs is the case where the child was already running when the user
//! picked its parent — there the user can pick the child itself, and a preset covers the
//! titles worth covering. Comparing process creation times would be exact, but it means
//! opening a handle to every process claiming to be a child, including the ones the answer
//! will reject, and this app's first promise is that it touches no process it was not asked
//! about.
//!
//! The executable *path* would be a stronger version of rule 2 — it separates two unrelated
//! programs that happen to share a file name — but reading it means opening a handle to
//! every candidate process, and this app's first promise is that it touches no process it
//! was not asked about. The name is free, comes out of the same snapshot as the tree, and
//! where it groups wrongly the fix is a preset entry rather than a build.
//!
//! # Membership is live, and it is sticky
//!
//! Recomputed on the discovery beat: a process that exits stops being a member while its
//! siblings carry on, and a process spawned a moment ago is adopted without the user
//! touching anything. A member that is still running keeps its place even if the rule that
//! first found it no longer applies — the parent it was adopted through may have exited —
//! but only under the same executable name. Windows recycles process identifiers, and a
//! recycled one running something else is not the process we adopted.
//!
//! **An application with no running process is kept, not dropped.** That is the point of
//! arming the monitor before a match: the user picks the launcher, the game appears minutes
//! later, and the choice they made is still there to catch it. It costs one of the five
//! application slots and the page says plainly that nothing is running under it.
//!
//! # Nothing here reads a clock or the operating system
//!
//! The snapshot is passed in, so the whole rule — adoption, expiry, the tree walk, the caps
//! and the conflicts between two applications wanting one process — is replayed in tests on
//! any operating system without a process ever being enumerated.

use std::collections::{BTreeSet, HashMap, HashSet};

use nm_core::endpoint::{AppId, MAX_MONITORED_APPS};
use nm_platform::process::{Pid, ProcessInfo};

use crate::presets::PresetList;

/// How many processes one application may hold.
///
/// Not a product promise like the five-application cap, but a bound on what rule 3 can do:
/// a user who picks a process with hundreds of descendants — a shell, a service host —
/// would otherwise turn one click into a membership set the size of the machine, and every
/// flow event would be tested against it. Sixty-four is far beyond any real application
/// (a heavily-helpered Electron app runs a dozen) and far below the point where the
/// per-event test costs anything.
pub const MAX_PROCESSES_PER_APP: usize = 64;

/// One process currently belonging to an application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Its identifier.
    pub pid: Pid,
    /// Its executable name, as the snapshot that adopted it reported.
    ///
    /// Kept so that a recycled identifier can be told from the process we adopted: the
    /// name has to still match on every later beat or the member is dropped.
    pub name: String,
}

/// One application the user chose to monitor.
#[derive(Debug, Clone)]
pub struct Application {
    id: AppId,
    label: String,
    preset: Option<String>,
    /// Executable names that make a process a member on sight.
    names: Vec<String>,
    members: Vec<Member>,
}

impl Application {
    /// The identity everything downstream keys on.
    #[must_use]
    pub const fn id(&self) -> AppId {
        self.id
    }

    /// What to call it: the preset's name for it, or the chosen executable's file name.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The preset that grouped it, if one did.
    #[must_use]
    pub fn preset(&self) -> Option<&str> {
        self.preset.as_deref()
    }

    /// The processes it currently consists of, by identifier.
    ///
    /// Shown to the user, deliberately: a grouping nobody can inspect is one nobody can
    /// correct.
    #[must_use]
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Whether a process currently belongs to it.
    #[must_use]
    pub fn contains(&self, pid: Pid) -> bool {
        self.members.iter().any(|member| member.pid == pid)
    }

    /// Recomputes membership against a snapshot, claiming the processes it takes.
    ///
    /// `claimed` carries the processes earlier applications already hold: one process can
    /// only belong to one application, and the tie is broken by the order the user chose
    /// them, which is the only order they can see.
    ///
    /// `seen` is every process that existed at the previous snapshot. The tree rule adopts
    /// only what is missing from it — see the module documentation for the live failure that
    /// bought this rule.
    fn regroup(&mut self, index: &Index<'_>, claimed: &mut HashSet<Pid>, seen: &HashSet<Pid>) {
        let mut chosen: BTreeSet<Pid> = BTreeSet::new();
        let mut room = MAX_PROCESSES_PER_APP;

        let mut take = |pid: Pid, chosen: &mut BTreeSet<Pid>| {
            if room == 0 || claimed.contains(&pid) || chosen.contains(&pid) {
                return false;
            }
            chosen.insert(pid);
            room -= 1;
            true
        };

        // Members that are still running the same program. First, so that the cap never
        // evicts a process already being measured in favour of one just discovered.
        for member in &self.members {
            if index.still_running(member.pid, &member.name) {
                take(member.pid, &mut chosen);
            }
        }

        // Anything running under one of the application's own names.
        for process in index.processes() {
            if self.answers_to(&process.name) {
                take(process.pid, &mut chosen);
            }
        }

        // Everything *newly* descended from what we have, breadth first. A visited set is
        // not needed beyond `chosen` itself, which is what stops a recycled identifier that
        // happens to close a parent loop from spinning here.
        let mut frontier: Vec<Pid> = chosen.iter().copied().collect();
        while let Some(pid) = frontier.pop() {
            for child in index.children_of(pid) {
                // A process that was already running cannot have been started by this
                // application while we were watching, whatever its recorded parent says.
                if seen.contains(&child) {
                    continue;
                }
                if take(child, &mut chosen) {
                    frontier.push(child);
                }
            }
        }

        claimed.extend(chosen.iter().copied());
        self.members = chosen
            .into_iter()
            .filter_map(|pid| {
                Some(Member {
                    pid,
                    name: index.name_of(pid)?.to_owned(),
                })
            })
            .collect();
    }

    /// Whether an executable name makes a process a member on sight.
    ///
    /// Case-insensitive over ASCII, which is how Windows compares file names and covers
    /// every executable name in practice. A non-ASCII name — games really do ship with
    /// localized ones — must match exactly, which is stricter rather than wrong.
    fn answers_to(&self, executable: &str) -> bool {
        self.names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(executable))
    }
}

/// A process snapshot, indexed for the two questions the rule asks of it.
struct Index<'a> {
    by_pid: HashMap<Pid, &'a ProcessInfo>,
    children: HashMap<Pid, Vec<Pid>>,
    order: &'a [ProcessInfo],
}

impl<'a> Index<'a> {
    fn build(snapshot: &'a [ProcessInfo]) -> Self {
        let mut by_pid = HashMap::with_capacity(snapshot.len());
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for process in snapshot {
            by_pid.insert(process.pid, process);
            if let Some(parent) = process.parent {
                // A parent identifier is a claim about a process that may have exited long
                // ago and may since have been reissued. Only ever resolved inside this same
                // snapshot, which is what keeps the answer self-consistent.
                if parent != process.pid {
                    children.entry(parent).or_default().push(process.pid);
                }
            }
        }
        Self {
            by_pid,
            children,
            order: snapshot,
        }
    }

    /// The processes, in the order the operating system reported them.
    fn processes(&self) -> impl Iterator<Item = &'a ProcessInfo> + '_ {
        self.order.iter()
    }

    /// Whether a process is still running the program that was adopted under it.
    fn still_running(&self, pid: Pid, name: &str) -> bool {
        self.by_pid
            .get(&pid)
            .is_some_and(|process| process.name.eq_ignore_ascii_case(name))
    }

    fn name_of(&self, pid: Pid) -> Option<&'a str> {
        self.by_pid.get(&pid).map(|process| process.name.as_str())
    }

    fn children_of(&self, pid: Pid) -> impl Iterator<Item = Pid> + '_ {
        self.children.get(&pid).into_iter().flatten().copied()
    }
}

/// One application the user could choose, as the picker offers it.
///
/// **Not a process.** A picker listing six identical `Discord.exe` rows asks the user to
/// pick one arbitrarily and gets the question backwards: they want to watch Discord, and
/// which of its processes opened the socket is exactly what this app exists to stop them
/// having to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Stable key for the offer — the preset's identifier, or the lowercased executable
    /// name. Unique in one listing, and the same across refreshes, so a list the user is
    /// clicking in does not reshuffle.
    pub key: String,
    /// What to call it.
    pub label: String,
    /// The process to form the application around, if the user chooses it.
    pub seed: Pid,
    /// The processes it is offered as, in identifier order.
    ///
    /// What the *picker's* rules found: an executable name, or a preset. The tree rule is
    /// not applied here — a candidate list that adopted descendants would put half the
    /// machine under whichever process happened to be listed first. It is applied when the
    /// user actually chooses, which is why an adopted application can hold more processes
    /// than the offer showed.
    pub processes: Vec<Pid>,
}

/// Groups every running process into the applications a user could choose.
///
/// Two rules, and deliberately not the third. A preset joins its executables; otherwise
/// processes sharing an executable name are one offer. **Descendants are not followed
/// here**: the tree rule exists to catch a launcher starting a title *after* the user has
/// committed to watching it, and applying it to an unfiltered process list would collapse
/// the machine into whatever the shell started.
#[must_use]
pub fn candidates(presets: &PresetList, snapshot: &[ProcessInfo]) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for process in snapshot {
        if process.name.is_empty() {
            continue;
        }
        let (key, label) = match presets.matching(&process.name) {
            Some(preset) => (preset.id.clone(), preset.label.clone()),
            None => (process.name.to_lowercase(), process.name.clone()),
        };

        if let Some(candidate) = index.get(&key).and_then(|at| found.get_mut(*at)) {
            candidate.processes.push(process.pid);
            continue;
        }
        index.insert(key.clone(), found.len());
        found.push(Candidate {
            key,
            label,
            seed: process.pid,
            processes: vec![process.pid],
        });
    }

    for candidate in &mut found {
        candidate.processes.sort_unstable();
        candidate.seed = root_of(candidate, snapshot);
    }
    // By label, then by key, so the list is the same on every refresh and a row does not
    // move out from under the cursor.
    found.sort_by(|left, right| {
        left.label
            .to_lowercase()
            .cmp(&right.label.to_lowercase())
            .then_with(|| left.key.cmp(&right.key))
    });
    found
}

/// The member to seed an application from: one whose parent is not also a member.
///
/// The main process of an application that spawns helpers, which is the one whose children
/// are worth adopting. Ties, and a group with no such member, fall back to the lowest
/// identifier so the choice is decided rather than left to enumeration order.
fn root_of(candidate: &Candidate, snapshot: &[ProcessInfo]) -> Pid {
    let members: HashSet<Pid> = candidate.processes.iter().copied().collect();
    candidate
        .processes
        .iter()
        .copied()
        .find(|pid| {
            snapshot
                .iter()
                .find(|process| process.pid == *pid)
                .and_then(|process| process.parent)
                .is_none_or(|parent| !members.contains(&parent))
        })
        .or_else(|| candidate.processes.first().copied())
        .unwrap_or(candidate.seed)
}

/// Every application the user is monitoring, and which process belongs to which.
#[derive(Debug)]
pub struct Applications {
    presets: PresetList,
    apps: Vec<Application>,
    /// Which application each member belongs to, rebuilt after every change.
    ///
    /// Exists because it is asked once per discovery observation — a flow event per send
    /// call on a busy game — while it changes once a second at most.
    index: HashMap<Pid, AppId>,
    /// Every process that existed at the previous snapshot.
    ///
    /// What tells a child an application really started from one that was running all
    /// along and merely names a reissued parent identifier. See the module documentation.
    seen: HashSet<Pid>,
    next: u32,
}

impl Applications {
    /// Builds a registry that groups by the given presets.
    #[must_use]
    pub fn new(presets: PresetList) -> Self {
        Self {
            presets,
            apps: Vec::new(),
            index: HashMap::new(),
            seen: HashSet::new(),
            // Identifiers start at one so that zero never names an application; a raw
            // identifier crosses the IPC boundary, and a default-constructed zero arriving
            // back from the UI must not be a valid target.
            next: 1,
        }
    }

    /// Starts monitoring the application `seed` belongs to.
    ///
    /// Returns its identity, or [`None`] when the process is not running, is already part
    /// of a monitored application, or would be the sixth — the cap is a refusal rather
    /// than an eviction, because which application to stop watching is the user's choice.
    pub fn adopt(&mut self, seed: Pid, snapshot: &[ProcessInfo]) -> Option<AppId> {
        if self.apps.len() >= usize::try_from(MAX_MONITORED_APPS).unwrap_or(usize::MAX) {
            return None;
        }
        // Against this snapshot rather than the last one: the answer decides whether the
        // user's click does anything at all, so it is worth being current.
        self.refresh(snapshot);
        if self.app_of(seed).is_some() {
            return None;
        }
        let process = snapshot.iter().find(|process| process.pid == seed)?;

        let id = AppId::new(self.next);
        self.next = self.next.saturating_add(1);
        let application = match self.presets.matching(&process.name) {
            Some(preset) => Application {
                id,
                label: preset.label.clone(),
                preset: Some(preset.id.clone()),
                names: preset.executables.clone(),
                members: Vec::new(),
            },
            None => Application {
                id,
                label: process.name.clone(),
                preset: None,
                names: vec![process.name.clone()],
                members: Vec::new(),
            },
        };
        self.apps.push(application);
        self.refresh(snapshot);
        Some(id)
    }

    /// Recomputes every application's membership against a fresh snapshot.
    pub fn refresh(&mut self, snapshot: &[ProcessInfo]) {
        let index = Index::build(snapshot);
        let mut claimed: HashSet<Pid> = HashSet::new();
        for app in &mut self.apps {
            app.regroup(&index, &mut claimed, &self.seen);
        }
        // Taken after the regrouping, so the *next* snapshot can tell which processes are
        // new. Every process, not only the members: a child of a member was not a member
        // itself a moment ago, and it is its own novelty that qualifies it.
        self.seen.clear();
        self.seen.extend(snapshot.iter().map(|process| process.pid));
        self.reindex();
    }

    /// Stops monitoring one application. Returns whether it was being monitored.
    pub fn forget(&mut self, id: AppId) -> bool {
        let before = self.apps.len();
        self.apps.retain(|app| app.id != id);
        if self.apps.len() == before {
            return false;
        }
        self.reindex();
        true
    }

    /// The application a process belongs to, if any.
    #[must_use]
    pub fn app_of(&self, pid: Pid) -> Option<AppId> {
        self.index.get(&pid).copied()
    }

    /// Every process the discovery sources must report on, in a stable order.
    #[must_use]
    pub fn watched_pids(&self) -> Vec<Pid> {
        let mut pids: Vec<Pid> = self.index.keys().copied().collect();
        pids.sort_unstable();
        pids
    }

    /// The monitored applications, in the order the user chose them.
    pub fn iter(&self) -> impl Iterator<Item = &Application> + '_ {
        self.apps.iter()
    }

    /// How many applications are monitored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.apps.len()
    }

    /// Whether nothing is monitored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    fn reindex(&mut self) {
        self.index.clear();
        for app in &self.apps {
            for member in &app.members {
                self.index.insert(member.pid, app.id);
            }
        }
    }
}
