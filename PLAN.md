# PLAN.md — Network Monitor roadmap

Status legend: `[ ]` todo · `[x]` done · `[~]` in progress.
Each phase ends with all quality gates green (see CLAUDE.md). Phases are ordered so that
the riskiest platform work (ETW) is de-risked early, but a usable app exists from Phase 3.

## Phase 0 — Scaffolding & quality gates

Goal: empty-but-real app that builds, tests, lints on Windows; CI-ready.

- [x] `git init`, `.gitignore`, README stub
- [x] Tauri 2 workspace: `nm-core`, `nm-probes`, `nm-platform`, `nm-app` crates; Vite + React + TS frontend
- [x] Tooling: rustfmt/clippy configs, ESLint (strict, no-`any`), Prettier, Vitest, tsc
- [x] tauri-specta wired: one demo command + event with generated TS types
- [x] i18next wired: `locales/en/common.json`, no hardcoded strings from day one
- [x] `just check` (or npm script) running all gates from CLAUDE.md; sample test in every crate + one UI test
- [x] GitHub Actions CI: Windows runner, full gate suite
- **Accept**: fresh clone → `just check` green; `tauri dev` opens a window showing a translated string fed from Rust.
  *Verified: window shows core version, platform backend and live heartbeat uptime, all rendered
  from i18next keys with values supplied by Rust.*

Phase 0 notes (deviations worth remembering):

- `tauri.conf.json`, the app manifest and icons live in `src-tauri/crates/nm-app/`, not in
  `src-tauri/`, because the Tauri app is one crate of a workspace rather than the whole of it.
  The CLI discovers the config automatically.
- `nm-app`'s tests are integration tests under `tests/`, and `[lib]`/`[[bin]]` set `test = false`.
  Linking Tauri pulls in comctl32 v6 exports; in-crate test harnesses do not receive the
  side-by-side manifest `tauri-build` embeds into the binary, so they died at load with
  `STATUS_ENTRYPOINT_NOT_FOUND`. `build.rs` supplies `tests.manifest` to test targets instead.
- IPC payloads stay within `u32`/`f64`: `specta` rejects 64-bit integers because JavaScript
  numbers cannot carry them without precision loss.

## Phase 1 — Metrics core (pure Rust, fully tested)

Goal: all measurement math exists and is bulletproof before any OS integration.

- [x] Metric types: RTT sample, probe outcome (success/timeout/unreachable/icmp-blocked), target identity
- [x] Fixed-capacity ring buffers for sample history (no unbounded growth)
- [x] Sliding-window stats: avg/min/max RTT, jitter (RFC 3550 mean deviation + stddev), loss % from timeouts, percentiles (p50/p95/p99)
- [x] Probe scheduler model: per-target interval, global rate cap, monotonic-clock based, testable with a fake clock
- [x] Target registry: add/remove/tag targets (per-app endpoint, baseline, status service)
- **Accept**: exhaustive unit tests incl. edge cases (empty window, all-timeouts, clock jumps); `nm-core` has zero platform/tokio deps.
  *Verified: 77 unit tests; `cargo tree -p nm-core` is `thiserror` and its proc-macro crates, nothing else.*

Phase 1 decisions worth remembering:

- **Absent knowledge is `None`, never zero.** A window with no delivery test reports no loss
  percentage rather than `0 %`. `Blocked` (the probe kind is filtered) is excluded from the
  loss denominator entirely — counting it as loss would invent 100 % packet loss on a healthy
  link, which is exactly the fake-zero failure CLAUDE.md forbids. `Unreachable` is likewise a
  definitive answer, not a lost packet.
- **RTT is stored as `u32` microseconds**, not a float: exact, totally ordered (so percentile
  sorting can never meet a `NaN`), 4 bytes per sample. Statistics are presented in `f64` ms.
- **Percentiles use nearest rank** with integer ceiling division, so boundaries do not depend
  on floating-point rounding. Jitter is RFC 3550's `J += (|D| - J)/16` over consecutive
  *replies* — round-trip differences stand in for the one-way transit differences the RFC
  defines, since a round-trip probe cannot observe transit; failures are skipped rather than
  treated as gaps.
- **Scheduling priority is interval length, not a separate rank.** Ordering due targets by how
  overdue they are makes oversubscription stretch every effective interval fairly and rules out
  starvation by construction; a strict priority order could freeze out low-priority targets
  forever. Deadlines are set forward from *now*, never from the missed deadline, so a backlog
  can never become the burst the rate cap exists to prevent. The token bucket's burst allowance
  is an eighth of a second's budget — 4 probes at the product's 32/s cap.
- **No type in `nm-core` reads a clock.** Callers pass `now` in, which is what lets the tests
  simulate ten seconds of scheduling across a hundred targets in microseconds. All arithmetic
  on `Instant`/`Duration` is checked or saturating, so a clock that appears to move backwards
  yields no panic and no windfall of probe budget.

## Phase 2 — Probe engine

Goal: real measurements against real hosts, still no per-app discovery.

- [~] `Prober` trait + implementations: ICMP (`IcmpSendEcho2Ex` on Windows via `nm-platform`), TCP-connect, TLS-handshake, HTTP(S) HEAD
      — trait, `select_kind` gate, ICMP and TCP-connect probers done; TLS/HTTP remain.
      **TLS/HTTP are no longer optional fallbacks**: they are the only probe kind that measures
      a FakeIP-tunnelled endpoint at all (see the spike). TCP-connect must never be used as an
      RTT source for such endpoints — it completes locally and reports a fake-*good* ~0.7 ms.
      Enforced by `select_kind`, which refuses ICMP and TCP outright for a tunnel sentinel.
- [~] TTL-limited path probing: RTT to last responding hop when the target itself is silent; classify where the path dies (ISP / border / destination)
      — the TTL mechanism works and is verified; the ISP/border/destination classification remains
- [x] Source-address binding on all probers (probe egresses via a chosen local address/interface)
      — implemented and verified live for ICMP; carried into TCP-connect (bind before connect,
      with a mismatched address family refused rather than silently handed to the OS)
- [ ] Async probe runner on tokio: timeouts, cancellation, per-target backoff on repeated failure.
      **Windows needs ~2 s to report a refused TCP connection** — measured on loopback, where the
      reset is instant; the stack retries the attempt before believing it. A TCP probe deadline
      under that turns every closed port into fabricated packet loss, so TCP-connect needs its
      own generous deadline and the runner must treat a slow TCP probe as normal.
- [ ] ICMP-blocked detection → automatic fallback chain per target (ICMP → TCP/TLS where ports exist → path probe)
- [x] **Reality-check spike**: measure actual ICMP/path-probe responsiveness of real server pools (Valve SDR, Discord voice, Apex/AWS, Riot); record results in a doc — validates the whole measurement model early
      — see `docs/measurement-reality-check.md`
- [ ] **FakeIP / synthetic-address handling** (added by the spike — the target audience's
      routers really do this, via podkop/sing-box). An endpoint inside `198.18.0.0/15` or
      `fc00::/18` is a sentinel a local tunnel will remap, so ICMP measures nothing and
      TCP-connect lies. Detect it, mark the endpoint as tunnelled, route it to a TLS/HTTP
      probe, and label the measurement honestly as end-to-end-through-a-tunnel rather than
      as an RTT to the server. The range must be configurable — sing-box's default is not
      mandatory. Note that a real setup is a **mix**: some endpoints are tunnelled and some
      are direct, in the same session.
- [ ] **Endpoint labelling from the OS DNS cache** (candidate): Windows' resolver cache maps
      the sentinel address back to the domain that produced it, so a tunnelled endpoint can
      be shown by name rather than as a meaningless synthetic address — read-only, no capture,
      no router access. Needs a stable API (`DnsGetCacheDataTable`) verified first; applications that
      resolve over their own DoH will not appear in it.
- [ ] Rate/budget enforcement (≤ 1 probe/s/target default, global cap) with tests via mocked prober + fake clock.
      **A TLS handshake is far more expensive than an ICMP echo** — in traffic, in CPU, and in
      load on someone else's server — so tunnelled endpoints need a much longer interval, with
      passive flow statistics covering the gaps. Worth designing: one long-lived connection
      with periodic light requests instead of a full handshake per probe, and what TLS session
      resumption does to the measured value.
- **Accept**: integration test (feature-gated, run manually/CI-opt-in) probing localhost + a public anycast IP; unit suite runs offline via mocks.
  *Partly met: `nm-platform`'s `network-tests` feature probes loopback, a public anycast IP and
  walks the path outward. The offline suite covers the ICMP prober through a `mockall` platform
  mock and the TCP prober against a loopback listener; the runner and the chain still to come.*

What the spike settled (full report in `docs/measurement-reality-check.md`):

- **8 of 12 pools answered ICMP directly**, so the model holds — but the fallback chain is a
  routine path, not an edge case. Epic's AWS-hosted address ignored every echo while the path
  probe still placed the failure at the destination's edge, 15 hops out and past the border.
- **A FakeIP router turns some endpoints synthetic.** podkop/sing-box on the router answers DNS
  with `198.18.0.0/15` sentinels and remaps them at connection time. Measured consequences:
  ICMP leaks past the tunnel and dies inside the ISP; **TCP-connect returns ~0.7 ms because the
  handshake completes locally** — a fake-*good* number, which is worse than a fake-bad one
  because it would tell the user their network is fine; TLS handshake does traverse the real
  path (170–180 ms vs 30 ms direct, genuine certificate) and is therefore the only usable probe
  for these endpoints. The tunnel terminates on the router, so the PC cannot split the latency
  into legs unless the user supplies the proxy's address.
- **Windows returns TTL expiries with the hop's address**, so path probing gets the identity it
  needs; walks must step over silent hops rather than stop at the first.
- **The Valve SDR hostnames were guesswork and do not resolve.** Phase 5 must take them from the
  SDR configuration Steam publishes, not from a guessed naming scheme.

## Phase 3 — App shell & general network health (first usable build)

Goal: tray app a user can run during a game.

- [ ] Tauri shell: system tray, minimize-to-tray, single instance, autostart toggle (off by default)
- [ ] UI stops rendering when window hidden; Rust core continues; event stream batched ≤ 4 Hz
- [ ] Baseline target lists in `assets/targets/`: `domestic/<country>.json` (start: ru, ir) + `foreign.json`; country selected in settings (no geo-detection phoning home)
- [ ] Dashboard page: domestic vs foreign health side by side — RTT/jitter/loss sparklines (uPlot), simple verdict per group (OK / degraded / blocked)
- [ ] Settings page: language (en now), country, probe intervals; persisted locally (debounced writes)
- **Accept**: manual scenario — run alongside a game, task manager shows core <1 % CPU, <150 MB total; dashboard clearly distinguishes "ISP down" vs "foreign degraded" (simulated via mocks in tests).

## Phase 4 — Per-application monitoring (Windows)

Goal: the headline feature. Riskiest OS work — budget extra care and testing.

- [ ] `nm-platform` Windows: process enumeration (name, pid, icon), TCP table snapshot w/ PID (`GetExtendedTcpTable`), UDP table
- [ ] ETW session (`Microsoft-Windows-Kernel-Network` via ferrisetw): per-process UDP/TCP flow events → remote endpoint discovery + per-flow byte counters; graceful degradation to table-polling-only if ETW unavailable
- [ ] Endpoint lifecycle: appear/idle/gone; dedup; enforced caps (≤ 5 monitored apps,
      ≤ 16 probed endpoints/app prioritized by recent traffic, 32 probes/s global) —
      scheduler stretches intervals under pressure, never silently drops; unit-tested
- [ ] Auto-probing of discovered endpoints (ICMP → fallbacks → path probe), tagged per app; probes source-bound to the same local address as the app's flow (VPN/accelerator route parity)
- [ ] Egress awareness in UI: show which interface each app flow and its probe use; mismatch warning (per-process interceptor case)
- [ ] App-monitor page: process picker with multi-select (search, icons), per-app endpoint
      lists with live RTT/jitter/loss + throughput, per-endpoint sparkline; "probe blocked"
      honest state
- [ ] Known-app presets: Discord, Dota 2, CS2, Apex Legends, Valorant, Fortnite (process names + expected port ranges as data, not code)
- **Accept**: monitor Discord + a game simultaneously in a real session → voice server and game endpoints appear with independent live metrics while staying inside the probe budget; all discovery logic that parses/decides is platform-free and unit-tested; ETW handler tested against recorded event fixtures.

## Phase 5 — Service status, game reference pools & diagnosis verdicts

Goal: at-a-glance "is it them or me", including "the game's servers are down (or partly)".

- [ ] Status check definitions as data: `assets/targets/services.json` — Steam, Epic, Discord, Riot, EA/Origin, Battle.net, Xbox Live, PSN, AWS, GCP, Cloudflare (per-service: endpoints + probe kind)
- [ ] Periodic low-frequency checks (e.g. 30–60 s) reusing the probe engine
- [ ] Status page UI: service cards with state (reachable / slow / unreachable), latency, last-checked; grouping (platforms / infra)
- [ ] Per-service history (session-scoped ring buffer) with mini-timeline
- [ ] Game reference pools — bundled seeds as data: Valve SDR POP ping endpoints (from Valve's published SDR config), AWS GameLift ranges, Riot/Blizzard known targets; refreshed only via app releases or explicit "Update target lists" action
- [ ] Learned endpoint history: persist endpoints the user connected to, tagged per game preset (cap ~32/game, LRU, expire after N days unseen); cold start covered by bundled seeds
- [ ] Reference-pool trickle probing (round-robin, active only while the game is monitored or on explicit "Diagnose"), inside the global probe budget
- [ ] Diagnosis verdict engine in `nm-core`: pure rules combining baselines + app metrics + path-probe death point + reference-pool response ratio + platform-API status into verdicts (ISP / border / routing-to-game / game servers down / partial outage); every matrix row unit-tested; verdicts phrased as network-level facts only
- [ ] Verdict surfacing in UI (dashboard + app-monitor banner)
- **Accept**: page reflects a manually blocked host (hosts-file test) within one check interval; service list extendable by editing JSON only; simulated scenarios (mocked probe outcomes) produce correct verdicts incl. partial game-server outage; stale cache entries expire and never fake an outage.

## Phase 6 — Polish, persistence, packaging

- [ ] Local history persistence (bounded, e.g. rolling 24 h; SQLite or compact custom format) + history view
- [ ] Russian locale (`locales/ru/*`) — validates i18n discipline end-to-end
- [ ] Windows installer (NSIS/MSI via Tauri bundler), code-signing hook point, portable build
- [ ] Perf pass: measure real CPU/RAM against budget, profile hot paths, document results in README
- [ ] Error UX: offline mode, no-targets state, ETW-unavailable banner
- **Accept**: signed-ready installer; budgets verified and documented; switching to ru requires zero code changes.

## Phase 7+ — Later (do not start without explicit go-ahead)

- Linux support (`sock_diag` netlink + `/proc`, unprivileged-ICMP handling)
- macOS support (`libproc`; note: Tauri e2e tooling unavailable on macOS — rely on unit/headless layers)
- Endpoint enrichment: offline ASN/GeoIP database (licensing TBD), reverse DNS (user-toggleable — it generates visible DNS traffic)
- Passive TCP RTT from the OS's own estimates (Windows TCP eStats) as a complement to
  probing — requires admin elevation to enable, hence opt-in and late
- Detection of known accelerator/VPN virtual adapters (ExitLag, WTFast, WireGuard, …) for
  clearer egress labeling and per-process-interceptor warnings
- Alerts (jitter/loss thresholds → tray notification), overlay-friendly compact mode
- Export/share of measurement reports

## Standing risks

- **ETW via ferrisetw** is the least-proven dependency → Phase 4 starts with a spike; if blocked, fall back to raw `windows`-crate ETW consumption behind the same trait.
- **ICMP realism**: many game servers deprioritize/drop ICMP → fallback probes + honest UI states are first-class, not afterthoughts.
- **Dev machine is macOS, target is Windows** → keep platform-free logic maximal; CI on Windows runners is the source of truth for `nm-platform/windows`.
