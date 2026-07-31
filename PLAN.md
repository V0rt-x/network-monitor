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

- [x] `Prober` trait + implementations: ICMP (`IcmpSendEcho2Ex` on Windows via `nm-platform`), TCP-connect, TLS
      — trait, `select_kind` gate, and all three probers done.
      **TLS is not an optional fallback**: it is the only probe kind that measures
      a FakeIP-tunnelled endpoint at all (see the spike). TCP-connect must never be used as an
      RTT source for such endpoints — it completes locally and reports a fake-*good* ~0.7 ms.
      Enforced by `select_kind`, which refuses ICMP and TCP outright for a tunnel sentinel.
      **The TLS probe sends a `ClientHello` and times the first answering byte rather than
      completing a handshake.** The first flight already contains the round trip, so stopping
      there costs no TLS implementation, no root store and no OpenSSL on Linux, and no key
      exchange at either end. Verified live against three public resolvers (feature
      `network-tests`): all answered, and a non-TLS port yielded no measurement rather than a
      false one. **HTTP(S) HEAD is dropped** — it needs everything the hello probe avoids and
      measures the same quantity plus server-side work.
- [x] TTL-limited path probing: RTT to last responding hop when the target itself is silent; classify where the path dies
      — the walk lives in `nm_probes::path` (steps over silent hops, stops on reply/unreachable/
      a run of silence), the classification in `nm_core::path`.
      **Deliberate correction to the wording**: the classifier reports *position* — the user's
      own network, the provider's carrier NAT, public infrastructure before any long-haul link,
      or past one — and does **not** claim to have found a national border. Naming a border
      needs the domestic/foreign baselines to corroborate, which is Phase 6's job. A confident
      "the border is blocking you" built on one traceroute would be wrong often enough to make
      the product untrustworthy for the people who most need it to be right.
      A long-haul link is a round-trip jump of ≥ 30 ms that *stays* high for every hop beyond:
      distance does not come back, whereas a router with a busy control plane spikes once and
      recovers.
- [x] Source-address binding on all probers (probe egresses via a chosen local address/interface)
      — implemented and verified live for ICMP; carried into TCP-connect (bind before connect,
      with a mismatched address family refused rather than silently handed to the OS)
- [x] Async probe runner on tokio: timeouts, cancellation, per-target backoff on repeated failure.
      **Windows needs ~2 s to report a refused TCP connection** — measured on loopback, where the
      reset is instant; the stack retries the attempt before believing it. A TCP probe deadline
      under that turns every closed port into fabricated packet loss, so the connecting kinds get
      six seconds (`timeout_for`) while an echo gets one.
      — `nm_probes::runner` splits decisions from execution: `ProbeRunner` reads no clock and
      opens no socket, so a day of scheduling, degradation and recovery is testable in
      milliseconds on any OS; `drive` is the thin tokio loop. A dispatched target is
      *unscheduled* until its answer arrives, so a probe outliving its own interval is never
      issued twice and the rate cap is spent on probes that are actually happening.
      Backoff (`nm_core::backoff`) stretches only on *unbroken* failure — an endpoint losing
      every other packet keeps full resolution, because that loss is the measurement the user
      came for. Switching probe kind resets it: the old kind's failures say nothing about its
      replacement. Our own failures reach the caller but never touch the chain or the backoff.
- [x] ICMP-blocked detection → automatic fallback chain per target (ICMP → TCP/TLS where ports exist → path probe)
      — `nm_probes::chain::FallbackChain`, a pure state machine: no clock, no probes, so a
      whole session of degradation is testable without a network. An explicit "filtered"
      outcome sets a kind aside at once; silence needs an unbroken run, because falling back
      on the first timeout would abandon ICMP over ordinary packet loss. An `Unreachable` never
      costs a kind its place — the endpoint is answering, with a "no".
      **Filtering is only *claimed* once a later kind succeeds**: silence alone is equally
      consistent with a dead host, and the UI must not say "ICMP blocked" without the proof.
      A tunnelled endpoint that exhausts TLS gets `Nothing`, not a path walk — a TTL walk from
      this machine would map the route to the tunnel rather than to the destination.
- [x] **Reality-check spike**: measure actual ICMP/path-probe responsiveness of real server pools (Valve SDR, Discord voice, Apex/AWS, Riot); record results in a doc — validates the whole measurement model early
      — see `docs/measurement-reality-check.md`
- [x] **FakeIP / synthetic-address handling** (added by the spike — the target audience's
      routers really do this, via podkop/sing-box). An endpoint inside `198.18.0.0/15` or
      `fc00::/18` is a sentinel a local tunnel will remap, so ICMP measures nothing and
      TCP-connect lies. Detect it, mark the endpoint as tunnelled, route it to a TLS
      probe, and label the measurement honestly as end-to-end-through-a-tunnel rather than
      as an RTT to the server. The range must be configurable — sing-box's default is not
      mandatory. Note that a real setup is a **mix**: some endpoints are tunnelled and some
      are direct, in the same session.
      — detection (`nm_core::address`, configurable ranges) and routing to the TLS probe
      (`select_kind`) done in Phase 2; the honest **labelling closed in Phase 3**, where a
      tunnelled baseline target carries a "through a tunnel" badge beside the probe kind that
      produced its figure. Confirmed on the dev machine's real podkop/sing-box router: a
      foreign service resolved into the sentinel range, was refused ICMP and TCP by
      `select_kind`, was measured by the TLS hello, and read several times the round trip of
      its directly-routed siblings — labelled, not silently presented as an RTT to the server.
- [ ] **Endpoint labelling from the OS DNS cache** (candidate) — **deferred to Phase 4**, where
      an endpoint list exists to label. Windows' resolver cache maps the sentinel address back to
      the domain that produced it, so a tunnelled endpoint could be shown by name rather than as
      a meaningless synthetic address — read-only, no capture, no router access.
      *The precondition set here is not met*: `DnsGetCacheDataTable` does not appear anywhere in
      `windows-sys` 0.61, i.e. it is absent from the Win32 metadata and is an undocumented
      `dnsapi.dll` export whose structures have changed between Windows versions. Committing to
      it would put an unsupported API in a tool that must not break on a Windows update. Decide
      in Phase 4 between shipping without names, shelling out to a supported command, or
      accepting the undocumented export behind a clearly optional feature. Applications that
      resolve over their own DoH will not appear in the cache either way.
- [x] Rate/budget enforcement (≤ 1 probe/s/target default, global cap) with tests via mocked prober + fake clock.
      — the cap is wired in `ProbeRunner::new` from `GLOBAL_PROBE_RATE_CAP_PER_SEC` and enforced
      by `nm_core::scheduler`'s token bucket; tested with an injected clock, including that
      targets over budget stay due rather than being dropped.
      **A TLS probe is still more expensive than an ICMP echo** — a connection setup and a few
      hundred bytes against a 32-byte echo — so tunnelled endpoints need a longer interval,
      with passive flow statistics covering the gaps. Much less expensive than first assumed,
      though: stopping at the hello removes the key exchange on both sides, which was the part
      that made a full handshake unrepeatable. The long-lived-connection idea and TLS session
      resumption are no longer relevant — neither applies to a handshake we never finish.
- **Accept**: integration test (feature-gated, run manually/CI-opt-in) probing localhost + a public anycast IP; unit suite runs offline via mocks.
  *Met: `nm-platform`'s `network-tests` feature probes loopback, a public anycast IP and walks
  the path outward; `nm-probes`' sends a real `ClientHello` to three public resolvers. The
  offline suite covers the ICMP prober through a `mockall` platform mock, the TCP and TLS
  probers against loopback listeners, and the chain, backoff and runner as pure state machines
  with an injected clock.*

**Phase 2 status: the probe engine is done. One item remains open and cannot be closed here.**

- Endpoint labelling from the DNS cache is a *candidate* whose stated precondition failed — see
  above. Deferred to Phase 4 as a decision, not as work.
- FakeIP handling was `[~]` because its remaining half was UI. Phase 3 built that half and the
  item is now closed.

One change was made to Phase 2's code while building Phase 3, and it belongs here: `drive` now
emits a `Completed` — the report **plus** `TargetProgress`, the runner's belief about the target
after folding that result in (which kind is in use, whether filtering has been *proven*, whether
anything measurable is left). The runner is moved into its own loop and cannot be queried from
outside, so without this the app layer would have had to reimplement `FallbackChain`'s inference
to caption a number. The belief travels with the measurement instead.

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

- [x] Tauri shell: system tray, minimize-to-tray, single instance, autostart toggle (off by default)
      — `nm_app::shell`. Two official Tauri plugins were added, and the reason is layering as much
      as convenience: `tauri-plugin-single-instance` so a second launch reaches the running
      instance instead of starting a second set of probes, and `tauri-plugin-autostart` so
      "start with Windows" does not mean writing to the registry from a crate that forbids
      `unsafe` and platform `cfg`s.
      **The tray menu's words come from the UI, not from Rust.** Every user-visible string goes
      through an i18next key and those live in the frontend, so the tray starts with an icon and
      no menu and the UI hands it translated labels on mount and on any language change. Russian
      stays what CLAUDE.md promises: new JSON, no code. The consequence is deliberate — until
      those labels arrive there is no way back from the tray, so closing the window *quits*
      rather than hides until the menu exists.
- [x] UI stops rendering when window hidden; Rust core continues; event stream batched ≤ 4 Hz
      — one `AtomicBool` is the single source of truth, read by both the health task and the
      heartbeat before either emits. Hidden means **nothing is sent at all**: the core keeps
      probing and the history keeps filling, but the `WebView` is never woken to lay out a chart
      nobody can see. Showing the window emits immediately rather than waiting out the period, so
      it is never blank on return. Emission is 1 Hz — the ≤ 4 Hz cap is a ceiling, not a target,
      and baselines are probed every few seconds anyway.
- [x] Baseline target lists in `assets/targets/`: `domestic/<country>.json` (start: ru, ir) + `foreign.json`; country selected in settings (no geo-detection phoning home)
      — compiled in with `include_str!`, so the app never fetches them and cannot be made to.
      Schema, rationale and the rules for adding an entry are in `assets/targets/README.md`; a
      test asserts every bundled list validates, stays inside its probe budget and carries a port
      for the fallback chain to use.
      **An entry may be a host name**, resolved once through the system resolver when monitoring
      starts. Not a convenience: public resolvers are anycast, so `1.1.1.1` measured from inside a
      censored country usually terminates inside it and says almost nothing about the border —
      confirmed on the dev machine, where the anycast entries answered in single-digit
      milliseconds while a named foreign service took an order of magnitude longer. A name that
      does not resolve is shown as unresolved rather than dropped: a foreign baseline that quietly
      shrank to its working members would read as good news.
- [x] Dashboard page: domestic vs foreign health side by side — RTT/jitter/loss sparklines (uPlot), simple verdict per group (OK / degraded / blocked)
      — verdict logic is `nm_core::health`, pure and exhaustively tested; the UI renders it.
      **A group always shows its distribution beside the headline**, per the rule Phase 4 states
      for applications: "3 clean, 1 unreachable" is actionable, one amber dot is not, and a group
      rolled up to its worst member reads as an outage that is not happening. Each target carries
      why its number means what it does — the probe kind, whether a tunnel is in the path, whether
      filtering has been *proven* rather than guessed, whether it can be measured at all.
      Sparkline gaps stay gaps: uPlot does not span a `null`, so an outage is a break in the line,
      and the x axis is real elapsed time rather than sample indices because backoff stretches
      intervals.
- [x] Settings page: language (en now), country, probe intervals; persisted locally (debounced writes)
      — the reply from Rust becomes the UI's state, never the value that was sent: intervals are
      clamped, unknown countries fall back, and the autostart flag comes back as the *platform*
      reports it, so a toggle that failed springs back instead of claiming something untrue about
      the machine. A malformed settings file is reported and **left untouched** — silently
      resetting someone's configuration and destroying the evidence is how a parsing bug of ours
      becomes their lost afternoon.
- **Accept**: manual scenario — run alongside a game, task manager shows core <1 % CPU, <150 MB total; dashboard clearly distinguishes "ISP down" vs "foreign degraded" (simulated via mocks in tests).
  *Partly met, and the gap is named rather than papered over.* The app was run on the dev machine
  (Windows, real router): both baselines populated within seconds, every domestic target answered
  ICMP, and the foreign group reported a mixed distribution — most members clean, one degraded —
  with the tunnelled member measured by TLS and labelled as such. The mocked scenarios are covered
  by tests: `tests/monitor.rs` asserts domestic-clean-with-foreign-dead comes out as exactly that
  and that one failing member never turns its healthy siblings red.
  **Not yet verified: the CPU and RAM budget under a running game.** That needs a real gaming
  session and a release build, and the numbers belong in the Phase 6 perf pass where they are
  measured properly and written down. Nothing here should be read as a measured budget claim.

Phase 3 decisions worth remembering:

- **The health window scales with the probe interval** (`nm_app::monitor::health_window`), twelve
  intervals clamped to 1–10 minutes. A fixed sixty-second window would hold a single sample at the
  slowest interval the settings allow, leaving every verdict permanently "not measured yet".
- **`Health` is one vocabulary for a target and for a group.** The five states mean the same thing
  at both levels, so the UI needs one set of strings rather than two that have to be kept in step.
  A group is `Ok` only when every judged member is; anything mixed is `Degraded` *with the counts
  shown*; a group where nothing answers is `Unreachable`, or `Blocked` when every member's probes
  were filtered — because filtering is an absence of knowledge and being told "no" is knowledge.
- **Group loss is weighted by probes, not averaged over members.** Averaging percentages lets a
  target probed twice report the same weight as one probed a hundred times, which turns two lost
  packets into a double-digit loss figure. The group's round-trip time is a *median*, so one member
  on a bad path does not drag the headline away from what everyone else sees.
- **The settings file is tolerant; the IPC type is strict.** `Stored` has all-optional fields so a
  file from an older build still loads, and `Settings` has none, so the generated TypeScript cannot
  let the UI send half a configuration.
- **`serde(deny_unknown_fields)` on target lists, but not on the settings file.** Opposite choices
  for opposite reasons: a misspelled key in a target list is a target that will never be probed and
  must be loud, while an unknown key in a settings file is usually a downgrade and must not lock
  the user out of their own configuration.

## Phase 4 — Per-application monitoring (Windows)

Goal: the headline feature. Riskiest OS work — budget extra care and testing.

- [~] `nm-platform` Windows: process enumeration (name, pid, icon), TCP table snapshot w/ PID (`GetExtendedTcpTable`), UDP table
      — `nm_platform::process` and `nm_platform::connection`: the traits, the platform-free
      parsing, and the Windows backends. Both address families for both protocols, because a
      build that read only IPv4 would silently lose every endpoint of a game on a v6 connection.
      **A UDP row names only the local socket.** UDP is connectionless, so the kernel has no
      peer to report — not even for a socket the application has `connect`ed — and `remote` is
      `None` rather than a guess. That absence is the exact size of the hole ETW exists to
      fill: polling alone can never discover the endpoints a game actually plays over. A test
      asserts it, so nobody later "fixes" the `None` into a listening port.
      **The sweep opens no process handle.** Toolhelp reports pid and executable name without
      touching a process, so an anti-cheat driver has nothing to see. The one call that does
      need a handle (`executable_path`) asks for `PROCESS_QUERY_LIMITED_INFORMATION` — the
      weakest right there is, unable to read memory — and runs only for a process the user
      selected, not for the few hundred a desktop has running. Being denied is `None`, not an
      error: a process at a higher integrity level is a fact about permission, and a picker
      that raised an error every time a process exited would be unusable.
      Ports are the subtle part. The tables carry them in network byte order inside a `DWORD`,
      so reading the low 16 bits yields a byte-swapped port that fails *silently* — every
      probe goes somewhere plausible and nothing ever answers. Pinned by a unit test and by a
      live one that reads back a port this process just bound on loopback.
      **Icons are not done**, deliberately: they belong with the process picker below, which
      is where the decision about extracting and encoding them for the `WebView` has to be
      made anyway.
- [ ] ETW session (**`Microsoft-Windows-TCPIP`** via ferrisetw): per-process UDP/TCP flow events → remote endpoint discovery + per-flow byte counters; graceful degradation to table-polling-only if ETW unavailable
      — **the spike ran first, as the standing risk demanded, and it corrected this item
      twice over; full report in `docs/etw-privileges-spike.md`.**
      `Microsoft-Windows-Kernel-Network`, the provider this plan named, is **unusable**: it
      is refused even to an account that may create sessions, because it is a kernel
      provider. `Microsoft-Windows-TCPIP` is not, and it carries exactly what this item
      needs — a UDP send/receive event with the process, both socket addresses and a byte
      count, which is the one thing the connection tables cannot report.
      **A trace session cannot be created by a standard user at all** — any provider, any
      output mode. The one-time fix is membership in the local group `S-1-5-32-559`
      («Пользователи журналов производительности»), after which the app runs unelevated
      forever. Verified end to end on this machine, unelevated, before and after.
      So *degraded* is the **default** state, not an edge case: until the user performs a
      one-time elevated action, the app sees TCP endpoints only. The UI must say that
      plainly and must never present an absent UDP endpoint as an absent flow. The app
      performs no elevation itself; it explains and leaves the action to the user.
      **Keyword, level and an event-ID filter are what make this fit the budget**, and the
      figures are measured rather than reasoned: `ut:SendPath | ut:ReceivePath` at
      `Informational` (full level costs sixteen times the volume for the same information),
      plus an `EVENT_FILTER_TYPE_EVENT_ID` filter on the two UDP event numbers, which the
      kernel applies before anything reaches this process. Over identical 20-second runs
      that filter took the delivered stream from 32 859 events to 94 — **99.7 % dropped** —
      and the spike's whole CPU cost, session setup included, was 16 ms. The marginal cost
      works out at ~0.5 µs per delivered event, so even 2 000 events/s would be 0.1–0.2 %
      of one core against a 1 % budget. **ETW consumption is not a threat to the budget.**
      PID filtering is *not* the lever — `ferrisetw` documents it as ineffective on a
      user-mode session, and it could not work for this provider anyway, since the kernel
      writes these events rather than the process they describe. Selecting the monitored
      processes is a `u32` comparison in our callback, which after the event-ID filter
      costs nothing worth measuring.
      Event numbers come from the provider's manifest and may move between Windows
      versions; the consumer resolves by provider and number and degrades when a number is
      absent rather than failing.
- [ ] Endpoint lifecycle: appear/idle/gone; dedup; enforced caps (≤ 5 monitored apps,
      ≤ 16 probed endpoints/app prioritized by recent traffic, 32 probes/s global) —
      scheduler stretches intervals under pressure, never silently drops; unit-tested
- [ ] Auto-probing of discovered endpoints (ICMP → fallbacks → path probe), tagged per app; probes source-bound to the same local address as the app's flow (VPN/accelerator route parity)
- [ ] Egress awareness in UI: show which interface each app flow and its probe use; mismatch warning (per-process interceptor case)
- [ ] App-monitor page: process picker with multi-select (search, icons), per-app endpoint
      lists with live RTT/jitter/loss + throughput, per-endpoint sparkline; "probe blocked"
      honest state
- [ ] **Per-endpoint state, never a single per-app verdict.** Filtering rarely hits everything
      an application talks to: within one app some endpoints stay clean, some lose packets, and
      some are unreachable outright — commonly at the same moment, because they sit in different
      networks (a login service on a CDN, voice on one provider, the game server on another),
      and because a FakeIP router tunnels some and routes others directly. The UI must therefore
      show state **per endpoint** and summarise an app as a distribution ("4 clean, 2 degraded,
      1 unreachable"), never collapse it to one colour: an app rolled up to its worst endpoint
      reads as "the game is broken" when the game is fine, and rolled up to its best hides the
      failure the user came to find. The per-endpoint state must also carry *why* — measured,
      degraded, probe filtered (which kind), unreachable, or not measurable — since
      `nm_probes::chain` already distinguishes these and flattening them would throw away the
      one thing that makes the verdict actionable. Sorting/grouping by severity, so the broken
      few are visible without hunting through a long list.
- [ ] Known-app presets: Discord, Dota 2, CS2, Apex Legends, Valorant, Fortnite (process names + expected port ranges as data, not code)
- **Accept**: monitor Discord + a game simultaneously in a real session → voice server and game endpoints appear with independent live metrics while staying inside the probe budget; **one app's endpoints can hold different states at once and the UI shows all of them** (verify by blocking a single endpoint via the hosts file or a firewall rule: that endpoint turns unreachable while its siblings stay clean, and the app is not reported as broken); all discovery logic that parses/decides is platform-free and unit-tested; ETW handler tested against recorded event fixtures.

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
- [ ] Verdict surfacing in UI (dashboard + app-monitor banner). A verdict covers *the endpoints
      it actually explains* and says which — an app whose voice endpoint is blocked while its
      game server is clean gets a verdict about that endpoint, not about the app. See the
      per-endpoint state requirement in Phase 4: partial failure inside one application is the
      normal case under filtering, not an edge case.
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
- Passive TCP RTT from the OS's own estimates as a complement to probing — **no longer
  believed to need elevation**: the Phase 4 spike found `Microsoft-Windows-TCPIP` emitting
  a per-connection summary carrying `RttUs`/`MinRttUs`/`MaxRttUs` to the same unelevated
  session Phase 4 already opens. Still late and still opt-in, but for cost rather than for
  privilege. (`GetPerTcpConnectionEStats` — the API this item originally meant — does
  require elevation; the ETW route sidesteps it.)
- Detection of known accelerator/VPN virtual adapters (ExitLag, WTFast, WireGuard, …) for
  clearer egress labeling and per-process-interceptor warnings
- Alerts (jitter/loss thresholds → tray notification), overlay-friendly compact mode
- Export/share of measurement reports

## Standing risks

- **ETW via ferrisetw** is the least-proven dependency → Phase 4 starts with a spike; if blocked, fall back to raw `windows`-crate ETW consumption behind the same trait.
  *Both halves are now measured (`docs/etw-privileges-spike.md`) and the risk is closed as
  a feasibility question. A session needs a one-time group membership and the planned
  provider had to be replaced; ferrisetw 1.2 then opened that provider unelevated, applied
  the event-ID filter that makes the budget comfortable, and parsed the process, both
  socket addresses and the byte counts off live events. What remains is ordinary
  implementation risk, not "can this be done".*
- **ICMP realism**: many game servers deprioritize/drop ICMP → fallback probes + honest UI states are first-class, not afterthoughts.
- **Dev machine is macOS, target is Windows** → keep platform-free logic maximal; CI on Windows runners is the source of truth for `nm-platform/windows`.
