# Known applications: grouping and names

This file does two jobs, and they are deliberately separate lists.

- **`applications` groups.** It joins several executables into one application.
- **`labels` names.** It gives one executable a proper noun and joins nothing to it.

The distinction is what keeps the safety rule below workable. "Never list an executable
several applications share" is a rule about *grouping*: putting `steam.exe` in a grouping
entry would silently merge two games the user chose separately. Putting it in `labels` only
means the picker says "Steam" instead of `steam.exe`, which is safe precisely because it
joins nothing — so the rule keeps applying, unchanged, to `applications` alone.

A game that runs as one process needs no grouping and still deserves its own name; without
a label it appears as `WorldOfTanks.exe` and the picker reads like a task manager.

## Grouping

An application is not a process. Discord is a main process and a handful of helpers that
share its executable name; a launcher starts the title as a child of itself; an anti-cheat
shim re-launches the game, so the process the user picked can be gone before the first
packet of the match. Nobody wants to know which of them opened the socket — they asked to
watch *Discord* and *Apex*.

The app therefore groups processes into applications. Most of that grouping is derived from
what the operating system already reports, and needs no data at all:

1. **The process the user picked**, always.
2. **Every process with the same executable name.** This is what catches an Electron
   application's helpers, which really are the same program.
3. **Every descendant of a member.** This is what catches launcher → title, and the
   anti-cheat re-launch, and it is the only relation that does.

This file exists for the cases those three rules get wrong — a title whose launcher has a
different name and is *not* its parent, or a game that spreads itself over two unrelated
executables. Listing them here joins them into one application without a code change.

## Schema

```jsonc
{
  "schemaVersion": 1,
  "applications": [
    {
      "id": "apex-legends", // stable, unique in this file; never shown to the user
      "label": "Apex Legends", // a proper noun: shown as written, never translated
      "executables": [
        // every executable name that belongs to this application, case-insensitive.
        // The first is the one the application is named after when several are running.
        "r5apex.exe",
        "r5apex_dx12.exe"
      ]
    }
  ],
  "labels": [
    {
      "id": "steam", // stable, unique across BOTH lists; never shown to the user
      "label": "Steam", // a proper noun: shown as written, never translated
      "executable": "steam.exe" // exactly one, case-insensitive; joins nothing
    }
  ]
}
```

The list is compiled into the binary. It is never fetched, and it changes only with a new
release of the app or by editing this file and rebuilding.

## Rules for adding a grouping entry

- **Never list an executable that several applications share.** `EasyAntiCheat_EOS.exe`,
  `EpicGamesLauncher.exe`, `RiotClientServices.exe` and `steam.exe` all serve more than one
  game: listing one would silently merge two applications the user chose separately, and
  the merged endpoint list would be wrong in a way they cannot see. A shared launcher is
  still handled — by rule 3 above, if the user picks the launcher itself. It may still be
  *named*, in `labels`.
- **A grouping entry is not a detector.** Nothing here decides what to monitor; the user
  does. An entry only says "if you picked one of these, these others belong with it".
- Executable names are compared case-insensitively, which is how Windows compares them.
- Grouping entries are for applications whose grouping is genuinely awkward. A game that is
  one process needs no entry — give it a label instead.

## Rules for adding a label

- **Verify the executable name against a real installation.** A recalled name is a label
  that never fires: nothing breaks, nothing warns, and the name simply never appears. Every
  entry shipped here was read off an installed copy.
- **One executable per entry, and never one a grouping entry already claims.** A grouping
  entry already carries a name; two sources for one name would depend on which list was read
  first. Parsing refuses it.
- A label is a proper noun. It is shown as written and never translated, and the picker
  shows the executable name beside it — a name the user cannot check is one they cannot
  correct.
- Labelling a shared launcher is fine and is the point of the separate list. Labelling an
  executable whose *identity* is ambiguous — a bare `launcher.exe` several vendors ship — is
  not: a confident wrong name is worse than a file name.

## `labels.json` — the generated catalogue

`presets.json` is written by hand and every entry in it was checked against a real
installation. That does not scale to a games library, so a second file, `labels.json`, holds
~9 300 generated names and is consulted **after** both curated lists. A curated entry always
wins, which is what makes correcting a wrong name one line rather than an argument with a
table of nine thousand entries.

- **It is generated by `node scripts/build-app-labels.mjs` and committed.** The application
  never runs that script and never fetches anything: the catalogue changes when a release
  changes it, exactly like the baseline target lists.
- **Where the data comes from.** Discord publishes the index it uses to recognise a running
  game (`/api/v9/applications/detectable`) — tens of thousands of titles, each with the
  executable names it runs as, per operating system. The mapping is factual: a file name
  beside the name of the program it belongs to. **No licence is stated for that endpoint.**
  That is written down here rather than assumed away; nothing in the app depends on it
  staying available, and dropping the file costs names and nothing else.
- **The filter is harsh on purpose**, because a confident wrong name is worse than a file
  name. Dropped: anything claimed by more than one title (408 of them), generic names and
  runtimes (`game.exe`, `launcher.exe`, `java.exe`), anything with fewer than four characters
  before the extension — the index really does claim `at.exe`, which is a Windows system
  binary — and anything the curated file already speaks for.
- It only ever supplies a **name**. Nothing in it can group one process with another, so a
  wrong entry costs a wrong label and can never put another program's traffic into a game's
  endpoint list.

## Why there are no port ranges here

`PLAN.md` originally asked for "process names + expected port ranges". The names earn their
place; the port ranges do not, and inventing a use for them would be worse than leaving
them out.

The one thing a port range could be used for is guessing which endpoint carries the match
traffic. The app already knows that, and it knows it by measurement rather than by
assumption: the flow counters say which endpoint the bytes are actually crossing, and the
ranking that picks the endpoint worth a path edge reads exactly those counters. A bundled
port range would be a weaker duplicate of a fact we hold — and one that quietly goes stale
every time a title changes its ports, in the direction of pointing at the wrong endpoint.
So it is left out, deliberately, rather than shipped as data nothing reads.
