# Known-application presets

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
  ]
}
```

The list is compiled into the binary. It is never fetched, and it changes only with a new
release of the app or by editing this file and rebuilding.

## Rules for adding an entry

- **Never list an executable that several applications share.** `EasyAntiCheat_EOS.exe`,
  `EpicGamesLauncher.exe`, `RiotClientServices.exe` and `steam.exe` all serve more than one
  game: listing one would silently merge two applications the user chose separately, and
  the merged endpoint list would be wrong in a way they cannot see. A shared launcher is
  still handled — by rule 3 above, if the user picks the launcher itself.
- **A preset is a grouping, not a detector.** Nothing here decides what to monitor; the
  user does. An entry only says "if you picked one of these, these others belong with it".
- Executable names are compared case-insensitively, which is how Windows compares them.
- Presets are for applications whose grouping is genuinely awkward. A game that is one
  process needs no entry.

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
