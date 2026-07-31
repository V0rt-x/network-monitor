# CLAUDE.md — Network Monitor

## What this project is

A lightweight desktop network monitor for gamers in regions with heavy state-level
internet censorship (Iran, Russia, etc.). Windows-first; Linux and macOS are planned
targets and must never be designed out.

Core capabilities:

1. **Per-application monitoring** — the user picks one or more running processes
   (e.g. Discord + Apex Legends simultaneously); the app discovers each one's remote
   endpoints and continuously shows ping (RTT), jitter, and packet loss per endpoint.
2. **General network health** — two separate baselines: *domestic* (services known to be
   reachable in the user's country) and *foreign* (services typically degraded or blocked),
   so the user can tell "my ISP is down" apart from "the border is throttled".
3. **Service status page** — live reachability of popular gaming platforms and
   infrastructure: Steam, Epic Games, Discord, Riot, EA, AWS, Google Cloud, Cloudflare, etc.
4. **Diagnosis verdicts** — combine all signals (baselines, per-app metrics, path probes,
   game reference-server pools) into an actionable network-level verdict: "your ISP",
   "border throttling/blocking → try VPN", "routing to game degraded → try accelerator",
   "game's servers down / partially down". Verdicts claim network-level facts only —
   never "the game is broken".

The user does not write code ("nocode" workflow): Claude implements everything. That makes
the quality gates below non-negotiable — they are the only safety net.

## Tech stack (decided, do not re-litigate)

| Layer | Choice |
|---|---|
| Shell | Tauri 2 |
| Backend / core | Rust (stable toolchain, edition 2021+), tokio |
| Frontend | React 18+, TypeScript (strict), Vite |
| Charts | uPlot (dense realtime series); no heavyweight chart libs |
| i18n | i18next + react-i18next; English default, Russian planned |
| IPC typing | tauri-specta — TS types are **generated** from Rust, never hand-written |
| State (UI) | Zustand (or equally minimal); no Redux |
| Rust tests | cargo test / cargo-nextest, mockall for trait mocks |
| Frontend tests | Vitest + React Testing Library |
| Lint / format | clippy, rustfmt, ESLint, Prettier, tsc --noEmit |

## Hard constraints — never violate

### Resource budget (the product's core promise)

The app must run *alongside* a competitive game without affecting FPS or ping.

- **CPU**: < 1 % average of one core while actively monitoring; short peaks < 5 %.
- **RAM**: Rust core < 50 MB; whole app (incl. WebView) < 150 MB.
- **When minimized to tray**: UI rendering must stop entirely (window hidden/unloaded);
  only the Rust core keeps working. Chart redraws only while the window is visible.
- **Probe traffic** is negligible by design (≤ ~1 probe/s/target, small payloads) and must
  never be bursty enough to add jitter to a game connection.
- **Scaling limits (enforced, not advisory)**: max **5 simultaneously monitored apps**;
  max **16 actively probed endpoints per app** (prioritized by recent traffic — idle
  flows demote to infrequent probing); global cap **32 probes/s** across everything
  (per-app + baselines + status page). On budget pressure the scheduler stretches
  intervals for low-priority targets; it never silently drops endpoints. Discovery cost
  is inherently ~constant (one ETW session, one table-poll loop, PID-filtered) — only
  probing scales, which is why the caps live there.
- No busy-polling. Event-driven (ETW) where possible; polling loops (e.g. TCP table) at
  ≥ 1 s intervals with monotonic-clock scheduling.
- Zero disk I/O on hot paths; settings/persistence writes are debounced.

### Monitoring techniques — userland only

- **NO packet-capture drivers** (Npcap/WinPcap/pcap), **no kernel drivers, no code
  injection, no process hooking, no reading other processes' memory.** The app must be
  invisible to anti-cheat systems (Vanguard, EAC, BattlEye) — read-only OS APIs only.
- Windows connection discovery: IP Helper API (`GetExtendedTcpTable`,
  `GetExtendedUdpTable`) + ETW (`Microsoft-Windows-Kernel-Network`) for per-process
  UDP flows and traffic counters.
- **Measurement model (universal, protocol-agnostic)**: passive *discovery* + active
  *measurement*. We passively learn *which endpoints* a process talks to (flow metadata:
  addresses, byte counters — never packet contents), then send **our own probes** (ICMP
  via `IcmpSendEcho2Ex` — no admin rights needed on Windows; TCP-connect/TLS/HTTPS as
  fallbacks) to those endpoints and derive RTT/jitter/loss from the probes. We do not
  parse application traffic: extracting RTT/loss from a game's own UDP packets requires
  per-protocol knowledge and packet capture — both banned. One pipeline serves per-app
  monitoring, baselines, and the status page; per-service differences are **data**
  (target lists, preferred probe kind), never separate code paths.
- **Silent servers → probe the path**: many game servers (esp. AWS/GCP-hosted) drop ICMP
  and expose no TCP ports. The fallback of last resort is a **TTL-limited path probe**:
  measure RTT to the last responding hop before the target (typically the datacenter edge,
  covering ~99 % of the path) and report *where* the path dies — inside the ISP, at the
  border, or at the destination. A fully blind "probe blocked" state must be rare and
  still shows passive flow liveness/throughput.
- **Game reference pools** (detect server-side outages, incl. partial): per-game pool of
  reference targets = bundled seeds (data in `assets/targets/`: Valve's published SDR POP
  ping endpoints, AWS GameLift CIDRs, etc.) + locally learned history of endpoints the
  user actually connected to (capped per game, LRU, entries expire after days unseen —
  game server IPs rotate and stale entries fake outages). Pools are probed as a
  low-frequency round-robin trickle within the global probe cap. Bundled lists update
  only with app releases or an explicit user action — never auto-fetch.
- **Routing-aware probes** (core use case: user compares before/after enabling a VPN or
  game accelerator like ExitLag): probes to an app's endpoint must **source-bind to the
  same local address the app's flow uses** (`IcmpSendEcho2Ex` source parameter), so they
  follow the same egress interface/tunnel. Show the egress interface per endpoint and
  warn on app-flow vs probe mismatch (per-process WFP interceptors can't be joined —
  disclose honestly, never silently mismeasure).
- Some game servers drop ICMP: detect this and degrade honestly (show flow/throughput
  stats and "ICMP blocked" state) instead of showing fake zeros.
- The app must run without administrator privileges. Any feature that would require
  elevation must be optional and clearly justified — ask the user (the human) first.

### Privacy & trust (audience lives under surveillance)

- **Zero telemetry. Zero crash reporting. Zero phone-home.** The app makes only the
  network requests the user can see and expects: probes to monitored endpoints and the
  bundled/user-edited status-check lists.
- All measurements and history stay on the local machine.
- Probe target lists (domestic baselines per country, foreign baselines, service status
  checks) live in versioned, human-readable config assets — auditable and user-editable.
- No third-party analytics SDKs of any kind. Dependencies are chosen conservatively;
  every new crate/npm package needs a reason.
- Never implement anything that could be construed as circumvention tooling (proxying,
  tunneling, traffic obfuscation). This is a *measurement* tool. That line keeps users safe.

### Cross-platform discipline

- All OS-specific code lives behind traits in the `platform` crate (e.g.
  `ProcessEnumerator`, `ConnectionTable`, `FlowEventSource`, `IcmpProber`).
  `#[cfg(windows)]` etc. appears **only** inside `platform`; `core`, `probes`, and the
  Tauri app layer must compile on all three OSes.
- Windows is implemented first, but every trait must have a documented, plausible
  Linux (netlink `sock_diag`, `/proc`) and macOS (`libproc`) implementation path.
- Time handling: monotonic clocks (`Instant`) for all measurements; wall clock only for
  display and persistence.

### i18n

- UI language is English. **Every user-visible string goes through i18next keys** — no
  hardcoded literals in components, ever, including units, tooltips, and error toasts.
- Keys are namespaced (`dashboard.ping.label`), stored in `locales/en/*.json`. Adding
  Russian later must be purely additive (new JSON files, no code changes).
- Numbers/dates formatted via `Intl` with the active locale.

## Architecture

```
network-monitor/
├── CLAUDE.md, PLAN.md, README.md
├── src-tauri/
│   ├── crates/
│   │   ├── nm-core/        # domain: metric types, ring buffers, jitter/loss math,
│   │   │                   # aggregation, target registry. Pure, platform-free, no tokio I/O.
│   │   ├── nm-probes/      # probe engine: ICMP/TCP/TLS/HTTP probers, scheduler,
│   │   │                   # timeout & rate policy. Depends on core + platform traits.
│   │   ├── nm-platform/    # traits + windows/ (later linux/, macos/) implementations
│   │   └── nm-app/         # Tauri entry: commands, events, state, persistence, tray
│   └── tauri.conf.json
├── src/                    # React UI (feature-folder layout: dashboard/, app-monitor/,
│   │                       # status-page/, settings/, shared/)
│   └── locales/en/*.json
└── assets/targets/         # baseline & status-check target lists (JSON, per-country)
```

Dependency direction is strict: `nm-core` ← `nm-probes` ← `nm-app` → `nm-platform`.
`nm-core` knows nothing about Tauri, tokio runtimes, or any OS.

### Design rules (senior-level expectations)

- **Rust**: `#![deny(warnings)]` in CI via `clippy -D warnings` (pedantic group enabled,
  targeted `allow`s with a comment). `unwrap()`/`expect()`/`panic!` are forbidden in
  library code — only in tests and at the very top of `main`. Errors: `thiserror` per
  crate, no `anyhow` in library crates. Public items documented (`#[warn(missing_docs)]`
  in nm-core/nm-probes). No `unsafe` outside `nm-platform`; every `unsafe` block carries
  a `// SAFETY:` justification.
- **Concurrency**: one tokio runtime owned by `nm-app`; core exposes sync, pure logic.
  Channels (`tokio::sync`) over shared mutable state; no lock held across `.await`.
- **IPC**: Rust → UI is a stream of typed events (batched at ≤ 4 Hz — the UI never
  drives sampling); UI → Rust is explicit commands. All types cross the boundary via
  tauri-specta generation. The frontend contains **zero business logic** — it renders
  state and sends intents; every calculation lives in Rust where it is unit-tested.
- **TypeScript**: `strict: true`, no `any` (ESLint-enforced), no default exports,
  components are function components, side effects isolated in hooks/stores.
- **Extensibility seams** (keep them clean, they are the roadmap): new probe kinds
  implement `Prober`; new platforms implement the `nm-platform` traits; new status-page
  services are data (JSON), not code; new locales are data, not code.
- Prefer boring, explicit code over cleverness. Small modules, small functions,
  names that spell out units (`rtt_ms`, `loss_pct`, `window_secs`).

## Quality gates — the commit contract

**Never commit unless ALL of the following pass locally:**

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # or: cargo nextest run
npm run typecheck               # tsc --noEmit
npm run lint                    # eslint --max-warnings 0
npm run test                    # vitest run
```

(These will be wrapped in a single `just check` / npm script during scaffolding — after
that, run the wrapper. If any gate fails, fix it; never bypass with `--no-verify`,
`#[ignore]`, `eslint-disable`, or loosened configs.)

- Testing is not optional. Every feature lands with tests: metric math (jitter, loss,
  percentiles) gets exhaustive unit tests incl. edge cases (empty windows, monotonic
  time going weird, timeouts); platform traits are mocked (`mockall`) so `nm-probes` and
  `nm-app` logic is testable on any dev machine; UI components with logic get Vitest tests.
  Bug fixes start with a failing regression test.
- Windows-only code paths that cannot run on the dev machine (macOS) must still compile
  (`cargo check --target x86_64-pc-windows-msvc` where toolchain allows) and their logic
  must be factored so the parseable/decidable parts are platform-free and tested.
- Conventional Commits (`feat:`, `fix:`, `test:`, `refactor:`, `chore:`). Small, focused
  commits. Do not commit generated artifacts except committed tauri-specta bindings.
- Keep `PLAN.md` current: check off completed items in the same commit as the work.

## Working agreements for Claude

- The user communicates in Russian; code, comments, identifiers, commit messages, and UI
  strings are English.
- When a requirement is ambiguous, prefer the interpretation that protects the resource
  budget and user privacy; ask only when the choice is genuinely product-shaping.
- Do not add features, dependencies, or configuration beyond what the current PLAN.md
  phase needs. Propose plan changes explicitly instead of drive-by scope growth.
- Verify claims about OS APIs against real behavior (docs/tests), not memory — ETW and
  IP Helper details are easy to get subtly wrong.
- If tests or analysis fail, report it plainly and fix it; never present unverified work
  as done.
