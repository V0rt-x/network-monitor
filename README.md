# Network Monitor

A lightweight desktop network monitor for gamers on heavily filtered connections.
It tells you *where* a problem is — your ISP, the border, the route to a game, or the
game's own servers — using only read-only OS APIs and probes you can see.

Windows first; Linux and macOS are planned. See `CLAUDE.md` for the engineering contract
and `PLAN.md` for the roadmap.

## What it does not do

- No telemetry, no crash reporting, no phone-home. The only network traffic is the
  probes you asked for, to targets listed in human-readable files under `assets/targets/`.
- No packet capture, no kernel drivers, no code injection, no process hooking. The app is
  invisible to anti-cheat systems and needs no administrator rights.
- No circumvention features. This is a measurement tool.

## Requirements

| Tool                      | Version                                                          |
| ------------------------- | ---------------------------------------------------------------- |
| Rust                      | stable (1.83+), `x86_64-pc-windows-msvc`                          |
| Visual Studio Build Tools | 2022, "Desktop development with C++" workload + Windows SDK       |
| Node.js                   | 22+                                                              |
| WebView2 runtime          | preinstalled on Windows 11                                       |

## Running the app

Once, after cloning:

```sh
npm install
```

Then, to run it:

```sh
npm run tauri dev      # or: just dev
```

That compiles the Rust workspace, starts Vite on `localhost:1420` and opens the desktop
window against it; the frontend hot-reloads on save, and a change under `src-tauri/`
rebuilds the core and restarts the window. The first build is a cold compile of the whole
dependency tree and takes a few minutes — subsequent ones are seconds. No administrator
rights are needed, in development or in a release build.

For a real, optimised build:

```sh
npm run tauri build
```

The executable and the Windows installers land under `src-tauri/target/release/` and
`src-tauri/target/release/bundle/`.

## Quality gates

Every commit must pass the whole suite:

```sh
npm run check          # or: just check
```

That runs, in order: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, a
check that the generated IPC bindings are committed and current, `tsc --noEmit`,
`eslint --max-warnings 0`, `prettier --check` and `vitest run`. The same command is the
whole of CI, on a Windows runner.

The suite stays offline and deterministic. Tests that send real packets are opt-in and
run deliberately:

```sh
cargo test -p nm-platform --features network-tests -- --nocapture
```

They need a working internet connection and report what the network actually did, which
is why they are also the instrument for the measurement-model reality check in `PLAN.md`.

## Layout

```
assets/targets/            baseline target lists (JSON) — see the README beside them
src/                       React UI (feature folders) + generated IPC bindings + locales
src-tauri/crates/
  nm-core/                 pure domain logic — no OS, no tokio, no Tauri
  nm-probes/               probe engine, scheduling and rate policy
  nm-platform/             OS traits; the only crate with #[cfg(...)] or unsafe
  nm-app/                  Tauri layer: commands, events, state, tray
```

Dependency direction is strict: `nm-core` <- `nm-probes` <- `nm-app` -> `nm-platform`.

`src/bindings.ts` is generated from the Rust IPC surface by `tauri-specta`; it is
rewritten by `cargo test` and must never be edited by hand.

The Tauri configuration lives at `src-tauri/crates/nm-app/tauri.conf.json` rather than in
`src-tauri/`, because the application is one crate in a workspace rather than the whole
of it. The CLI discovers it automatically.

## What it probes, and where that is written down

Everything the app contacts comes from `assets/targets/`: one list per country of services
expected to work inside it, and one list of services typically degraded or blocked at a
border. They are plain JSON, versioned here, and compiled into the binary — nothing is
fetched at runtime and no list updates itself. `assets/targets/README.md` documents the
schema and the rules for adding an entry.

Settings live in `settings.json` in the per-user application configuration directory and
nowhere else. There is no account, no sync, and no geo-detection: the country is something
you choose, because working out where someone is means asking a remote service.
