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
- [x] **Endpoint labelling from the OS DNS cache** (candidate) — **deferred to Phase 4**, where
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
      — **Decided in Phase 4: not shipped, and not deferred either.** All three options were
      weighed against what they buy, which is a label:
      *The undocumented export* is refused outright. An unsupported structure layout in a
      tool a censored user depends on is a crash or a garbage name one Windows update from
      now, and there is no version of "optional feature" that makes reading an undocumented
      layout safe — only one that makes the breakage rarer and harder to diagnose.
      *Shelling out* is refused for a different reason. `ipconfig /displaydns` prints
      localized field names, so parsing it means parsing a translation; `Get-DnsClientCache`
      means launching PowerShell. Both mean this application spawning a child process while a
      game with an anti-cheat driver is running, repeatedly, to obtain decoration. That is a
      worse trade than any label is worth.
      *So: no names from the cache.* What is lost is smaller than it first looked — the case
      that motivated it, a FakeIP sentinel, is already labelled as tunnelled, and an
      application resolving over its own DoH was never going to appear in the cache. The
      honest way to put a name beside an endpoint is the Phase 8+ enrichment item: a bundled
      offline provider table, which needs no OS API at all, and reverse DNS, which is
      user-toggleable precisely because it generates traffic of its own.
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

**Phase 2 status: the probe engine is done, and every item is now closed.**

- Endpoint labelling from the DNS cache was a *candidate* whose stated precondition failed. It
  was deferred to Phase 4 as a decision rather than as work, and Phase 4 decided it: not
  shipped, with the reasoning under the item above.
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
- **The Valve SDR hostnames were guesswork and do not resolve.** Phase 6 must take them from the
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
  session and a release build, and the numbers belong in the Phase 7 perf pass where they are
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
- [x] ETW session (**`Microsoft-Windows-TCPIP`** via ferrisetw): per-process UDP/TCP flow events → remote endpoint discovery + per-flow byte counters; graceful degradation to table-polling-only if ETW unavailable
      — `nm_platform::flow`: the `FlowEventSource` trait, the platform-free `SOCKADDR`
      decoding, and the ETW backend. A test opens a real session, sends on loopback and
      asserts that this process's own UDP peer is discovered with the right process, byte
      count and direction — the one thing the connection tables cannot do.
      **That test passes two ways on purpose.** Where the account may not trace it asserts
      exactly `TracingNotPermitted` and says so on stderr; where a session opens it asserts
      the discovery. A machine that quietly lost the ability to trace would otherwise be
      indistinguishable from one where the feature still works.
      Degradation is a distinct error variant rather than a generic failure, because the
      UI has to explain what is missing and what it costs, not report a fault the user can
      do nothing about.
      **Process selection happens in the callback, before any address is decoded** — flows
      belonging to applications the user did not pick never enter the process's memory.
      That is data minimisation first and cost second: this audience's machine should hold
      as little of its network's shape as the job allows.
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
- [x] Endpoint lifecycle: appear/idle/gone; dedup; enforced caps (≤ 5 monitored apps,
      ≤ 16 probed endpoints/app prioritized by recent traffic, 32 probes/s global) —
      scheduler stretches intervals under pressure, never silently drops; unit-tested
      — `nm_core::endpoint`, pure and clock-free: callers pass `now`, so the tests play out
      hours of lifecycle in microseconds. The two per-application caps moved here from
      `nm-probes`, beside the code that now enforces them; the global probe cap stays where
      the scheduler that enforces *it* lives.
      **The cap limits probing, not knowledge.** Past sixteen an endpoint demotes to the
      long interval and stays listed — an endpoint that disappeared from the UI for ranking
      seventeenth would look exactly like one that stopped working. A test asserts twenty
      endpoints stay twenty, split 16/4.
      **Ranking degrades to recency on its own.** Priority is recent bytes, then last seen;
      where no source counts bytes — a Windows machine without the tracing setup — every
      endpoint scores alike and the ordering falls through to recency with no special case.
      `recent_bytes` stays `None` there rather than `Some(0)`, because "unknown throughput"
      and "measured no throughput" are different answers and the UI must not merge them.
      **Idle is not gone.** Ten seconds of silence demotes; two minutes forgets. The gap is
      deliberately wide: a game whose endpoints were forgotten during a loading screen
      would rediscover and re-measure everything the moment play resumed.
      A new endpoint is demoted until its first ranking, so discovery can never surprise
      the budget, and an unmonitored application is refused rather than registered
      implicitly — otherwise discovery would walk straight past the five-app cap.
- [x] Auto-probing of discovered endpoints (ICMP → fallbacks → path probe), tagged per app; probes source-bound to the same local address as the app's flow (VPN/accelerator route parity)
      — decisions in `nm_app::apps::AppMonitor`, the operating-system side in
      `nm_app::discovery`, and the loop that joins them to the probe engine in
      `nm_app::runtime`.
      `AppMonitor` returns `TargetChange`s instead of calling the runner, because the runner
      lives inside its own async loop and is reachable only by message. The payoff is that
      the tests assert *what would be asked of the probe engine* without one existing —
      which is the only practical way to pin down that a steady endpoint produces no
      commands at all, since re-registering it every second would otherwise be invisible.
      Probes carry the local address the application's flow egresses from, and a flow that
      moves takes its probes with it — the VPN-toggled-mid-session case.
      **Two applications reaching one endpoint share one probe**, reference-counted so one
      letting go does not stop the other's measurement, at the shortest interval any current
      user wants. Where they reach it by *different* routes one probe cannot represent both,
      so that is recorded as a conflict — the disclosure CLAUDE.md requires, feeding the
      egress-mismatch item below.
      `nm-probes` gained `set_interval` for this: the cap demotes rather than drops, which
      needs a target's cadence to change while it stays registered. Failure history survives
      the change, so a rank shift is not an amnesty for a long-dead endpoint.
      **One probe engine, therefore one target registry.** The 32 probes/s cap is global, so
      a second `ProbeRunner` for applications would have a second token bucket and quietly
      double the traffic the product promises not to send. The registry had to be shared
      with it — two would hand the same handle to two different addresses and land a
      baseline's measurement on a game's endpoint — so `AppMonitor` borrows the session's
      registry instead of owning one. That also buys what the registry was built for: an
      address that is *both* a baseline and a game server is probed once and answers both,
      and an application letting go of it does not stop the dashboard's measurement.
      **The endpoints discovery declines to track** are the ones no probe kind would accept:
      loopback, LAN, link-local and reserved space, and port zero. That is not a measurement
      withheld — such an address would be registered, refused by the runner and listed
      forever as unmeasurable, spending one of the sixteen slots the application is allowed
      on a game's conversation with its own launcher.
      Nothing is polled until a process is chosen: a table sweep enumerates every socket on
      the machine, and the thread waits on its instruction channel instead. Choosing a
      process wakes it at once rather than at the end of its period.
      The IPC surface is `monitor_app`/`forget_app`, taking a pid. **No UI calls them yet** —
      the process picker below is what will, and until it exists the feature is reachable
      only from the bindings. That is the honest state of it: the pipeline is built and
      tested end to end against fake platform sources, and it has not been exercised against
      a real game.
- [x] Egress awareness in UI: show which interface each app flow and its probe use; mismatch warning (per-process interceptor case)
      — `nm_platform::interface`: the `InterfaceTable` trait, the platform-free
      address→adapter mapping, and a Windows backend over `GetAdaptersAddresses`. Read-only,
      unprivileged, and measured at ~2.5 ms, so it is re-read on the same five-second beat as
      application membership rather than cached for the session — a VPN or an accelerator
      coming up is precisely the event these labels exist to make visible.
      **The friendly name, not the device description.** `Description` is the driver's word
      for the hardware; `FriendlyName` is what the user has seen and possibly renamed, and
      the entire point of naming an adapter is that the person reading recognises it. An
      address no adapter claims shows as an address, never as a guessed name.
      **The warning now names both routes.** The endpoint carries the address the
      application's own flow leaves from *and*, when they differ, the one the probe is
      actually bound to — the per-process interceptor case, and the case of an address a
      baseline was already probing whose binding is not ours to move. "This may be wrong"
      leaves the user nothing to act on; "your traffic leaves via the accelerator, the probe
      leaves via Ethernet" is the finding. The second line is shown only when it differs, so
      the disclosure is not buried under a route restated on every row.
- [x] App-monitor page: process picker with multi-select (search, icons), per-app endpoint
      lists with live RTT/jitter/loss + throughput, per-endpoint sparkline; "probe blocked"
      honest state
      — `src/features/app-monitor/`, fed by `list_processes` and the `AppEndpoints` event.
      The page states the flow-source situation (`FlowStatus`) whenever it is not `Active`:
      on a machine without the one-time tracing setup there are **no UDP endpoints and no
      byte counters at all**, and the banner says so and how to fix it, because an empty
      list must never read as an application that has gone quiet.
      **Icons are not done.** They need `SHGetFileInfo` plus HICON→bitmap conversion in
      `unsafe` Windows code, and — because a picker holds a few hundred processes — a lazy
      per-process fetch to avoid megabytes of base64 crossing the IPC boundary for a list
      the user scrolls past. That is a feature's worth of platform code for decoration, so
      it is deferred to Phase 7's polish pass; the picker searches by name instead. The
      `[~]` on the process-enumeration item above stays for the same reason.
      The picker offers **every** running process, not only those already holding a socket:
      a game the user wants to watch *before* it connects is exactly the case where the
      first endpoints are worth catching. It is read when the page opens and when the user
      asks again — never on a timer, since a process list is stale the instant it is taken.
      **Throughput is a byte count over a stated window, not a rate.** The counter covers
      between one and two traffic windows, so dividing would invent a precision the
      measurement does not have.
      Not done here: the *interface* an endpoint egresses from is shown as the local
      address the probes bind to, not as an adapter name — see the item above.
- [x] **Per-endpoint state, never a single per-app verdict.** Filtering rarely hits everything
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
      — an application carries `HealthCountsView` and nothing else; there is no field a UI
      could render as a single colour for a game, which is the point. Endpoints are ordered
      worst first **in Rust** (`nm_app::view`), because the ordering is a judgement about
      severity and the frontend holds no business logic. Unreachable outranks degraded
      outranks *filtered*: filtering is an absence of knowledge, and being told "no" is
      knowledge.
- [x] **A silent endpoint that is demonstrably carrying traffic must not be called
      "unreachable".** Found by running the app against a live game: its UDP match server
      answered neither ICMP, nor TCP-connect, nor a TLS hello — correctly, since nothing
      listens on a game port but the game — while the flow events showed hundreds of
      kilobytes crossing it. The page reported *Unreachable*, which reads as "your game
      server is down" about a server that is working perfectly. That is the exact lie this
      product exists not to tell, and it is the normal case for the headline feature rather
      than an edge one: **every** UDP game server looks like this.
      — `Health::CarryingTraffic`, folded in by `nm_core::health::with_passive_evidence`.
      **A state of its own rather than a flavour of `Blocked`**: `Blocked` promises that
      filtering was *proven*, and that promise is what lets the UI state it as a fact the
      user can act on. Silence is not proof, so merging the two would have cost the stronger
      claim its meaning.
      The rule only ever softens a verdict of *absence*, never a measured one — traffic must
      not paper over a degraded path, which is the finding the user came for. It covers an
      explicit refusal as well as silence: a TCP probe to a game port is normally refused,
      and that is a fact about the port our probe chose, not about the path the game plays
      over. And it claims nothing further: round-trip time, jitter and loss stay absent,
      because knowing an endpoint is alive is not knowing how well it is doing.
      Its evidence is passive and cannot be faked — bytes the operating system counted — so
      it exists only where flow events do. Without the one-time tracing setup a game server
      still reads as unreachable, which is one more thing the flow-status banner has to
      explain.
- [x] **A tracing session that stops must be noticed and restarted.** Also found by running
      the app, and the harder lesson of the two. An ETW session is a *named system object*,
      not something a process owns: whoever opens the name next takes it over, and the
      previous consumer is left running with nothing arriving. The app read its flow status
      once at start-up, so it went on reporting "active" while discovering no UDP endpoint
      of a live game at all — which on screen is indistinguishable from a game that has
      none. `FlowStatus` is now asked of the source every time (`is_running`, cleared by the
      pump thread when `ProcessTrace` returns, whoever ended the session), a stopped session
      is restarted on the discovery beat and resumes watching the same processes, and
      `FlowStatus::Stopped` says so meanwhile.
      **What took the session over was our own test suite**, which opened the product's
      session name. Every `cargo test` on this machine silently stopped the tracing of the
      app the developer had running. The live test now uses a session name of its own; the
      fixed product name stays, because reclaiming a session orphaned by a crash is worth
      more than the collision it costs, but reclaiming must never be indistinguishable from
      *being* reclaimed.
- [x] Known-app presets: Discord, Dota 2, CS2, Apex Legends, Valorant, Fortnite (process names + expected port ranges as data, not code)
      — `assets/apps/presets.json`, compiled in like the target lists, with the schema and
      the rules for adding an entry in `assets/apps/README.md`. They are the *data half* of
      amendment 1 below: most grouping needs no data at all, and a preset exists only for a
      title the executable name and the process tree cannot join.
      **The one rule that protects the user is enforced by a test**: an executable several
      applications share — `steam.exe`, `EpicGamesLauncher.exe`, `RiotClientServices.exe`,
      an anti-cheat service — must never appear in a preset, because listing it would
      silently merge two applications the user chose separately into one wrong endpoint
      list. Parsing refuses an executable claimed by two presets for the same reason: which
      application a process joined would otherwise depend on file order.
      **The port ranges are deliberately not shipped.** The only thing they could be used
      for is guessing which endpoint carries the match traffic, and the app already knows
      that by measurement — the flow counters say where the bytes are actually going, and
      the ranking that picks the endpoint worth a path edge reads exactly those counters. A
      bundled range would be a weaker duplicate of a fact we hold, going stale in the
      direction of pointing at the wrong endpoint. The reasoning is written down in the
      asset README so it is not silently re-litigated.

### Amendments from use (recorded 2026-07-31) — in descending order of importance

Stated by the user after Phase 5A landed, and ordered as they stated them. Both change what
Phase 4 shipped and come before Phase 6.

A third was stated with them — naming an endpoint's destination by ASN or provider — and the
user placed it explicitly last. It was last, and it has since landed: see **Endpoint
enrichment** below, between Phase 6.5 and Phase 7. It was never part of Phase 4 and nothing
here waited on it.

- [x] **1. An application is a set of processes, not a pid.** The picker, the caps, the endpoint
      lists and the IPC surface are all keyed on one process id today, and that is not the thing
      the user chose. A desktop application is several processes: Discord is a main process plus
      helpers, a launcher spawns the title as a child (Riot Client → Valorant, EA app → the game),
      an anti-cheat shim may re-exec the game, so the pid the user picked can be dead before the
      first packet of the match. Nobody wants to know which of them opened the socket; they asked
      to watch *Discord* and *Apex*. What this changes:
      – **The grouping key is a decision, not a guess.** Candidates: the executable path (all
        instances of one binary, catches helpers that share it), the install directory (catches
        launcher + title under one vendor tree, at the risk of over-grouping), the file-version
        `ProductName`, and the process tree (a child joins its parent's application — the only
        rule that catches launcher → game). Likely a documented combination, with the
        awkward titles expressed as *data* so a wrong grouping is fixable without a build; that
        is the same file as the known-app presets item above.
      – **Membership is live.** A process dying must not end monitoring while its siblings live,
        and a newly spawned child must be adopted on the discovery beat. That is exactly the
        launcher-starts-the-game case, and it is what lets a user arm the monitor *before* a match
        rather than scramble for the picker once the game is already connected.
      – **The caps move up a level**: five monitored *applications*, sixteen probed endpoints per
        *application*, deduplicated across its processes, with the group's byte counters merged for
        ranking. Under the current pid keying five helper processes of one Electron app would eat
        the whole allowance the user meant for five games.
      – **The flow filter takes a changing set of pids.** `nm_app::discovery` already selects on
        process id inside the ETW callback, so this is a set-membership test that has to be
        refreshed as membership moves — not new platform work.
      – **`monitor_app`/`forget_app` and the `AppEndpoints` event take an application identity**
        instead of a pid. The UI must show which processes an application currently consists of:
        a grouping the user cannot inspect is one they cannot correct.
        *Amended by the user on 2026-08-03, after reading the running build: the card counts
        them at level one and enumerates them in an expander.* The requirement is unchanged —
        the list is one click away, not gone — but a browser contributes seventeen processes,
        and seventeen lines of `name · PID` above the figures is a wall the reader has to get
        past to reach what they came for. The count is the part worth a glance: it says how
        large a group the rule caught, which is what would look wrong if the grouping were.
        An application with **no** processes keeps its sentence at level one, because that is
        a finding — the monitor is armed and the game has not started — and "0 processes"
        would read as a bug.
      – **The picker offers applications too.** Found by running the build: the monitoring
        side was grouped and the picker was still a raw process list, so choosing Discord
        meant picking one of six identical `Discord.exe` rows at random — the exact question
        this item exists to remove. `list_applications` now groups every running process the
        same way and offers "Discord · 6 processes".
      — `nm_app::applications`, pure and clock-free: the process snapshot is passed in, so
      adoption, expiry, the tree walk, the caps and the conflicts between two applications
      wanting one process replay in tests on any operating system without a process ever
      being enumerated. `nm_platform::process::ProcessInfo` gained `parent`, which Toolhelp
      reports in the same structure as the name and therefore costs nothing — no handle, no
      second call.
      **The grouping key is three rules, not one**, and each earns its place: the chosen
      process; every process running under the same executable *name* (which catches an
      Electron application's helpers); every *descendant* of a member (the only relation
      that joins a launcher to the title it starts, and the only one that survives an
      anti-cheat re-launch). Presets are the fourth, for what those cannot see. The
      executable **path** was considered and rejected as the everyday rule: it is stronger,
      and reading it means opening a handle to every candidate process, which contradicts
      the promise that this app touches no process it was not asked about. Where the name
      groups wrongly the fix is a preset entry rather than a build.
      **Membership is sticky, but only under the same executable name.** A member that is
      still running keeps its place even after the parent it was adopted through exits;
      Windows reissues process identifiers, so a pid that came back running something else
      is dropped rather than kept.
      **A descendant must be newly seen** — found by running the build against a live game,
      where an unrelated device service turned up inside Apex Legends. Windows does not clear
      a recorded parent identifier when the parent exits and reissues identifiers freely, so
      a service started hours ago can name a dead parent whose number the game later
      received. Believing it puts another program's traffic in the game's endpoint list,
      which is a wrong measurement rather than an untidy one. The tree rule therefore adopts
      only a process absent from the previous snapshot: a title its launcher really did start
      appears after the launcher is already a member, a service running all along never does.
      Comparing process creation times would be exact and was rejected — it means opening a
      handle to every process claiming to be a child, including the ones the answer will
      reject, against the promise to touch no process we were not asked about.
      **An application with no running process is kept, not dropped** — that is the whole
      point of arming the monitor before a match, and the page says plainly that nothing is
      running under it. It costs one of the five slots, which is the user's to spend.
      **The refresh is not on the discovery beat, and the reason is measured**: a Toolhelp
      sweep of a real desktop (284 processes) takes ~8 ms, so once a second would spend most
      of the whole 1 % CPU budget on bookkeeping. Membership is recomputed every five
      seconds, on the blocking pool, and not at all while nothing is monitored; a user
      action takes its own snapshot rather than waiting for the beat. A test in `nm-platform`
      pins the 8 ms so it cannot quietly grow.
      A per-application cap of 64 processes bounds what the descendant rule can do — a user
      who picks a shell would otherwise turn one click into a membership set the size of the
      machine, tested against on every flow event.
      **The picker groups by two of the three rules, not all three.** A preset, then the
      executable name; the descendant rule is deliberately left out of the *offer*, because
      applied to an unfiltered process list it would collapse the machine into whatever the
      shell started. It applies once the user has committed to watching something, which is
      why an adopted application can hold more processes than the offer showed — and why the
      picker marks an offer as taken if *any* of its processes is monitored rather than
      requiring the two groupings to agree exactly.
- [x] **2. Show every endpoint of a selected application at once, on one chart.** The page today
      is a list of endpoints each with its own sparkline, which answers "how is this endpoint"
      and not the question the user actually has — "which of these is the odd one out". One
      multi-series time chart per application (uPlot, one line per endpoint) with the list beside
      it; hovering a line names its endpoint and raises its detail — probe kind, jitter, loss,
      throughput, egress, path panel — dimming the others rather than hiding them, and a click
      pins that selection. Constraints it must respect:
      – **An endpoint with no round trip must not vanish from the picture.** The silent match
        server has no RTT series to draw and it is the endpoint the whole product exists to
        watch; it is represented by its *path* figure, drawn distinctly and labelled as what it
        is. A path figure sharing an axis with round trips must never read as one — the Phase 5
        rule, applied to a chart.
      – Gaps stay gaps: no spanning of nulls, as on the dashboard, so an outage is a break.
      – Colour is assigned stably per endpoint and legible in both themes, and it never carries
        state on its own: the worst-first ordered list remains the authority on health, the chart
        is additive.
      – Sixteen series at ≤ 4 Hz, redrawn only while the window is visible. The chart must not
        become the thing that breaks the render budget.
      – The hover detail must be reachable without a mouse.
      — the alignment is **Rust's**, in `nm_core::series::Grid`: sixteen endpoints probed a
      second apart do not share sample times, and uPlot needs one x array, so the placement
      of a sample into a slot is a decision about what the numbers mean and belongs where
      every other calculation lives. A slot with no sample stays `None` — never zero, never
      interpolated — and where two samples fall in one the later is shown rather than an
      average, because smoothing would remove the spike the chart was opened to find. The
      window's statistics are still computed over every sample, so the list beside the chart
      remains the authority on what happened.
      **The per-endpoint sparkline is gone**, replaced by the shared chart: it answered "how
      is this endpoint" and the question during a match is "which of these is the odd one
      out". The payload shrank with it — one axis for the application instead of one per
      endpoint.
      **The silent match server appears as its route**, on a dashed line labelled "route to
      …", sharing the axis and never the meaning; `nm_core::edge::PathEdge` gained a way to
      read the reported hop's samples for it. An endpoint with a round trip *and* a route
      draws both. A test asserts the silent endpoint's own line is empty while its route's is
      not — the two must never merge into one number called "ping".
      **Colour identifies and never states.** The health palette is deliberately not reused:
      the worst-first list is the authority, and a second quieter verdict in the chart would
      contradict it the moment two endpoints shared a state. Assignment is held per endpoint
      (`endpointColours.ts`) rather than derived from list position, which re-sorts by
      severity on every emission and would otherwise repaint every line whenever one endpoint
      got worse.
      Hovering a line raises its row and dims — never hides — the others; a click pins it,
      and so does activating the row's own button, which is the keyboard path to the same
      thing.
      **The time axis is coarser than the probe interval, and that is the point.** Found by
      running the build: a clean endpoint's line was full of holes. A slot finer than the
      sampling is empty whenever no probe happened to land in it, and an empty slot is drawn
      as a break — so an endpoint losing nothing at all appeared to be dropping packets,
      purely because the scheduler had stretched an interval under the global rate cap. Slots
      are three seconds, which holds two or three probes even under budget pressure, so a
      break now means packets that did not come back. It also makes the axis advance a third
      as fast, which is what stops the lines sliding out from under a pointer trying to hover
      one. The cost is resolution, paid where it hurts least: **a slot shows its slowest
      sample**, so a spike inside it survives, and the note under the chart says so — a line
      of per-slot maxima sits above the mean in the row beside it, on purpose.
      **The round-trip axis is logarithmic**, decided after seeing the live build: an
      application's endpoints span two orders of magnitude — a hop inside the provider at a
      few milliseconds beside a server across an ocean — and one spike flattened every other
      line against the floor, which is both unreadable and impossible to put a pointer on. A
      log axis gives every line the same room to move in and clips nothing; the alternative
      considered and rejected was capping the range at a percentile, which would have hidden
      the size of the very spike the user is looking for. A value a log axis cannot place —
      zero, which needs a round trip faster than a microsecond — is drawn at the floor, and
      that is a drawing decision stated as one: the row beside the chart carries the exact
      figure.
      **Verified against a real render** on 2026-08-01, with Apex Legends monitored during a
      session: nine endpoints on one axis, the silent match server drawn as a dashed route
      line while its own round trip, jitter and loss stayed dashes in the row beneath, and
      the route panel naming the hop it belongs to. jsdom still has no canvas, so the
      component's own tests go through a stand-in — what the chart *contains* is tested, how
      it looks was checked by eye.
      Getting there cost two failures worth recording. The chart drew nothing at all and
      **said nothing about it**: the tick formatter treated a blanked minor tick as a number
      and threw in the middle of a draw, leaving an empty canvas, no error anywhere, and a
      note underneath still describing a chart that was not there. The formatter is now a
      tested function of its own, and a chart that cannot be drawn says so where the chart
      would have been. The second was mechanical and wasted more time than the first: an
      orphaned Vite server kept port 1420, so a restarted app was served a stale frontend and
      every fix appeared to do nothing.
- **Accept**: monitor Discord + a game simultaneously in a real session → voice server and game endpoints appear with independent live metrics while staying inside the probe budget; **one app's endpoints can hold different states at once and the UI shows all of them** (verify by blocking a single endpoint via the hosts file or a firewall rule: that endpoint turns unreachable while its siblings stay clean, and the app is not reported as broken); all discovery logic that parses/decides is platform-free and unit-tested; ETW handler tested against recorded event fixtures.

**Phase 4 status: every item is built, and the acceptance criterion is partly met. The gap
is named rather than papered over.**

Met, and covered by tests: every part of discovery that parses or decides is platform-free
and unit-tested — the connection-table and flow decoding, the address filter, the grouping
of processes into applications, the endpoint lifecycle and its caps, the per-endpoint
health rules, and the ordering the page renders. One application really does hold several
endpoint states at once; `tests/app_view.rs` asserts a distribution and a worst-first order
rather than a single verdict, and there is no field an application view could be collapsed
to one colour through.

Two things are **not** verified, and neither should be read as done:

- **No real session has been run since the amendments landed.** The app was run against a
  live match before them — that is where `Health::CarryingTraffic` and the stopped-tracing
  bug came from — but the grouping of processes into applications, the multi-series chart
  and the adapter names have only been exercised against fakes and unit tests. Discord plus
  a game, side by side, with a single endpoint blocked by a firewall rule, is the check this
  criterion actually asks for and it has not been performed.
- **The chart has not been seen rendering.** jsdom has no canvas, so uPlot draws nothing in
  the suite and the component's tests go through a stand-in.

One item stays `[~]` on purpose: process icons, deferred to Phase 7's polish pass with the
reasoning recorded above. "ETW handler tested against recorded event fixtures" is met in
substance rather than in form — the handler's decidable half (the `SOCKADDR` decoding, the
process filter, the endpoint filter) is unit-tested platform-free, and the handler itself is
covered by a live-session test that asserts this process's own loopback traffic is
discovered. No fixture file is replayed, because what a fixture would exercise is exactly
the decoding that is already tested directly.

## Phase 5 — Measuring the endpoint that answers nothing

Goal: real figures for a game's **match server** — the endpoint the whole product exists to
watch, and the one nothing we can send will ever answer.

This phase was inserted after Phase 4 shipped and the app was run against a live match. The
match server appeared, was proven alive by its own traffic, and carried **no round-trip
time, no jitter and no loss** — three dashes where the product's headline number belongs.
Everything Phase 4 built is worth little until this is closed.

**What is off the table, and why.** Tools that show "ping to the match server" do one of
three things: speak the game's protocol, capture packets with a driver, or read the game's
own overlay out of its memory. The second and third are banned by `CLAUDE.md` and visible
to anti-cheat; the first is per-game, fragile, and would have to be re-done for every title.
So the product must not promise an in-game ping. It promises two honest quantities instead,
and **never merges them into one number called "ping"** — see the rule added to `CLAUDE.md`.

### A — the path, measured continuously

- [x] **Sustained path-edge probing**: find the deepest hop toward the match server that
      answers consistently, then probe *that hop* at the normal cadence. `nm_probes::path`
      already walks the route and `nm_core::path` already classifies where it dies; what is
      missing is turning a one-off trace into a stream of samples with a history, so the
      figure has dynamics rather than being a snapshot.
      **A single hop is not evidence.** Routers rate-limit ICMP addressed *to themselves*
      while forwarding perfectly, so a spike or loss at the last hop is as likely to be a
      busy control plane as a bad path. Probe the last **two or three** responding hops and
      believe a degradation only when it shows at all of them; a figure that moves on the
      deepest hop alone is reported as the router's, not the path's. Without this rule the
      metric lies, which is worse than having none.
      — `nm_core::edge`, pure and clock-free: it chooses the hops, keeps a history per hop,
      and decides what they say together. A figure confined to the deepest hop is
      `PathQuality::Uncorroborated` — stated, with its ambiguity, never attributed to the
      path. Hops the address policy will not probe (the home router, carrier NAT) are passed
      over rather than spending a slot on silence, and one router answering at two distances
      takes one slot, since it could otherwise "corroborate" its own rate limiting.
- [x] **Honest labelling**: the figure is "to the last answering hop, N hops short of the
      server", never "RTT to the server". The UI states the hop count and where that hop
      sits (own network / ISP / carrier NAT / past a long-haul link — `nm_core::path`
      already answers this).
      — **with one deliberate correction: "N hops short of the server" cannot be stated,**
      because the server answered at no time-to-live at all, so nothing bounds its distance
      beyond the edge. What the panel says instead is the hop's *own* distance, how many hops
      are being watched, where the route stops, and — in as many words — that how much
      further the server sits is unknown. Claiming a hop count we never measured would be the
      same class of lie as the merged "ping" this phase exists to avoid.
- [x] Re-walk the route periodically and whenever the chosen hop stops answering, so a route
      change is followed rather than reported as loss.
      — every five minutes, or twenty seconds after the deepest hop goes quiet, with a
      one-minute floor under both: a hop that permanently rate-limits echoes would otherwise
      ask for thirty probes every few seconds forever. A re-walk that finds the same route
      keeps every sample and asks the probe engine for nothing at all.
- [x] Budget: 2–3 probes/s for the *active* match endpoint only, inside the global 32/s cap.
      Endpoints ranked below it keep the ordinary single probe.
      — **one edge per application**, on the busiest endpoint that has run out of probe
      kinds, which in a live match is the match server. It gives its hops up before another
      endpoint takes over, so the per-application cost is never briefly doubled.
      A defect found while building this and fixed in the same phase: **a walk is up to
      thirty probes and the scheduler counted it as one**, because one is what it dispatched.
      Once an endpoint's kinds were exhausted it walked at the endpoint's own cadence — a
      single silent game server could quietly spend the budget the baselines and every other
      endpoint share. Walks now run on their own five-minute cadence, with `Command::WalkNow`
      for the early re-walk, refused for any endpoint that can still be probed directly so it
      can never become a way past the rate cap.
- [x] **A refused port must retire the probe kind that chose it** (added here; without it
      section A never engages in a common case). A game's match server answers a TCP
      handshake on the port it plays UDP over with a reset — normal, since nothing listens
      there but the game — and `nm_probes::chain` treated that as a definitive answer worth
      keeping, which it is *about the port we chose* and not about the endpoint. The chain
      parked there and the route was never reached. A run of refusals now retires a kind that
      needs a port; an ICMP unreachable still does not, because that one comes from a router
      and is about the destination, where every other kind would fail identically. It is
      never evidence of filtering: the endpoint answered, so the path plainly works.

### B — the flow itself, measured passively

The only signal that measures the user's actual game traffic rather than a substitute, and
it costs almost nothing: the events are **already being delivered**.

- [x] **Spike first, and this phase's plan depends on its answer.** `docs/etw-privileges-spike.md`
      established that events `1169`/`1170` fire per *send/receive call* and carry
      `NumMessages`. Whether that gives per-datagram timing for a real game is unverified:
      a title that batches sends would report `NumMessages > 1` and the arrival timing would
      be lost. Measure on a live match: the distribution of `NumMessages`, the distribution
      of inter-event intervals on the receive direction, and the event rate per endpoint.
      Record it in `docs/`, and state plainly which of the metrics below survive.
      — measured against a live Apex Legends session over four progressively longer runs;
      full report in `docs/flow-metrics-spike.md`. **Every metric below survives**, and the
      spike corrected the plan three times over:
      *`NumMessages` is unusable.* It reports **0 on every receive event** — all 4 792 of
      them over four minutes — and 1 on every send. The field the spike was built to
      interrogate cannot count datagrams, so the product does not read it at all, and a
      comment stands where it would be read so nobody restores it.
      *What replaces it is a measurement.* One event really is one datagram: **0 of 4 791
      intervals were under a millisecond**, and 97 % landed within 45–55 ms of each other —
      the server's own 20 Hz tick. Per-datagram timing survives intact.
      *But a server may answer one tick with two packets.* Another phase of the same session
      put **a third of arrivals under a millisecond apart, in pairs**, with the usual cadence
      between the pairs. Timed raw, that stream reports enormous jitter while arriving
      perfectly regularly — so arrivals closer than 5 ms are coalesced into one update, a
      threshold two orders of magnitude clear of both clusters the data showed.
      *And the events arrive late.* Delivery lag is p50 511 ms, p95 964 ms, max 1015 ms —
      the tracing facility's one-second buffer flush, which cannot be set lower. Timing
      intervals on our own clock would therefore have measured the flush rather than the
      traffic. Hence `FlowInstant`, a clock type that cannot be added to an `Instant`, and
      hence the correction to the stall detector below.
- [x] **Arrival jitter**: spread of the intervals between datagrams arriving from the server.
      This is what the player feels as stutter. It is **not** RTT and must never be labelled
      as one — it also folds in the server's own send cadence, which is a feature rather than
      a flaw: combined with a clean path (A) it is what points at a server-side problem.
      — `nm_core::flow::ArrivalStats`, over *updates* rather than raw events, with the worst
      gap in the window shown beside the spread because that is the hitch a player recognises.
- [x] **Rate asymmetry**: our send rate is fully known, so a receive rate that falls while
      sending holds steady is loss or a stall on the far side.
      — reported as a **shortfall against the endpoint's own recent past**, never against an
      assumption of symmetry: no protocol owes one datagram back per datagram sent, so a ratio
      of anything but one means nothing while a *change* in it means something. The window's
      last quarter is compared with the three before it, and the figure is withheld outright
      unless our own send rate held at four fifths of its earlier value — a game closing, a
      match ending or a player alt-tabbing all cut the outgoing rate, and every one of them
      would otherwise be reported as the far end dropping traffic. Deliberately not called
      loss: only the far end knows what it sent.
- [x] **Stall detector**: sending continues, nothing comes back for N hundred milliseconds.
      A one-way outage, visible instantly and without sending a single probe.
      — **with one correction the spike forced: not "instantly".** The stall itself is
      measured exactly, from the kernel's own stamps, but the events carrying that news
      arrive up to a second late. Half a second of silence is the threshold — ten missed
      updates at the cadence a real game showed — and it is never claimed when the
      application's own sending stopped too, because a silence of ours is no evidence about
      theirs.
- [x] All of it pure in `nm-core`, clock-injected, tested against synthetic event streams —
      the same discipline as every other metric.
      — `nm_core::flow`, 25 unit tests, no clock and no I/O.

### C — free real RTT where the OS already has it

- [x] **Passive TCP RTT from event `1477`** (`RttUs`/`MinRttUs`/`MaxRttUs`), which the Phase 4
      spike found on the same unelevated session the app already opens. Not the match server,
      but a *true* round-trip time for the game's login, CDN and voice endpoints, at the cost
      of one more event number in the filter. Moved here from Phase 8+, where it was parked
      as expensive; it is not.
      — built, and it cost rather more than one event number. The spike settled three things
      the plan had assumed:
      *It is not on the same subscription.* The provider's manifest puts 1477 under a
      different keyword and at level 16, where the UDP events sit at 4 — and ETW delivers an
      event only when its level is at or below the session's. Reaching the summary therefore
      raises the session past the `Verbose` per-path telemetry that level 4 was chosen to
      exclude. Safe only because the event-ID filter is applied by the kernel first, which is
      measured rather than assumed: 44 delivered events a second in total during a live game,
      summaries included.
      *It carries no process identifier.* The process filter every other event goes through
      is impossible here, and the naive implementation would decode both addresses of every
      connection closing anywhere on the machine. The gate is the **local port** — an
      integer, tested before any address is decoded, against the set the connection table
      already knows for the monitored applications. The promise that this program holds as
      little of the network's shape as the job allows is kept exactly.
      *It is periodic as well as final.* Summaries arrive when a connection closes **and**
      every few tens of seconds while it lives, so the figure is genuinely live — just slow,
      which is why it is shown with its age rather than as a current reading.
      **A tunnelled endpoint is refused it**, for the same reason `select_kind` refuses a
      TCP-connect probe there: the connection terminates on the router, so the stack would be
      timing the round trip to the user's own hardware. That is a fake-*good* number, and
      telling someone under censorship that their connection is fine is this product's worst
      possible failure.
      One defect worth recording, because it is this codebase's recurring trap: **`LocalPort`
      is in network byte order**, so the filter matched nothing and the whole feature simply
      appeared not to work while the addresses beside the field decoded perfectly. Found by
      the live test, and pinned by a unit test beside the connection table's twin of it.

### The UI rule this phase exists to protect

- [x] The match-server card shows **two columns, never one**: *path* (A, with how far it
      actually reaches) and *flow* (B). Merging them into a single "ping" would make the
      product lie in exactly the way it was built not to.
      — both halves are built and they sit side by side. The path panel names the hop its
      figures belong to, how many hops are watched, where the route stops, and what the
      figure is *not*; the flow panel states that nothing in it times a request against its
      answer, and that the spread it shows includes whatever cadence the server chose. **The
      endpoint's own round trip stays a dash beside them both** — a test asserts exactly
      that, because the temptation to fill three empty fields with the nearest available
      number is precisely how this product would start lying.
      A defect found while writing that test, and fixed here: the match server was showing
      **"100 % loss" beside "carrying traffic"**. Those probes were ours, aimed at a port the
      game never plays over, and the figure read as "this server is dropping everything"
      about a server the user was playing on. `Health` now answers whether the probes
      describe the endpoint at all, and the loss figure is withheld where they do not — the
      same rule that already kept the round trip and the jitter empty.

- **Accept**: on a live match, the match server shows a moving path figure with its hop
  count and an arrival-jitter figure from its own traffic; killing the route to the chosen
  hop makes the app re-walk and pick another rather than report loss; a spike confined to the
  deepest hop alone is *not* reported as path degradation (verify with a rate-limiting hop or
  a simulated trace); no figure anywhere is labelled as a round trip to the server; the whole
  thing stays inside the probe budget with five applications monitored.
  *Met on a live match, on 2026-08-01, with Apex Legends monitored during a session.* The
  match server — silent to every probe kind, as always — showed **both columns at once**: a
  route figure of 111 ms to a hop eleven out, three hops watched, the route stopping past a
  long-haul link; and beside it, from its own traffic, 20.1 updates a second arriving with an
  arrival spread of 1.9 ms and a worst gap of 57 ms. Its own round trip, jitter and loss
  stayed dashes throughout. The arrival figures match the spike's independent measurement of
  the same stream exactly, which is the cross-check that matters: two different code paths,
  the same 20 Hz cadence.
  Covered by tests rather than by that session: `nm-core` replays route changes, rate-limiting
  and recovery against a fake clock; `nm-app`'s integration tests assert what the probe engine
  would be asked for — hops registered on the application's own egress, one edge per
  application, hops released when the route moves or the application is dropped; and
  `nm_core::flow` replays arrival streams, stalls and shortfalls against synthetic event
  streams.
  **Three things are not verified, and none should be read as done:**
  - *Killing the route to the chosen hop* to watch the app re-walk. The logic is replayed
    against a fake clock in `nm_core::edge`, but no route was actually broken on a live match.
  - *Five applications monitored at once* against the probe budget. One was.
  - *The operating system's own round trip appearing in the running UI.* The mechanism is
    proven live at the platform layer — a diagnostic run with the product's exact local-port
    gate passed two of the game's own summaries in 90 seconds, at 135 ms and 193 ms, agreeing
    with what the probes measured for the same two endpoints — and every hop of the wiring
    above it is unit-tested. But the figure had not yet appeared on a row in the running
    application when it was last looked at, and the reason is expected to be simple: at
    roughly one summary per connection per few minutes, the app had not been up long enough.
    Expected, not established.

Phase 5 decisions worth remembering:

- **Two clocks that must never be added together.** The passive metrics run on `FlowInstant`,
  the stamp the kernel puts on an event, and never on `Instant`. Measured: events arrive up to
  a second after the moment they describe, so timing them on arrival would have measured the
  tracing facility's buffer flush and called it network jitter. The separate type is what makes
  that mistake a compile error rather than a plausible-looking number.
- **The reading's "now" is the last event, not the clock.** Which means this history cannot
  tell how stale it is, so the layer above withholds the whole column once the application
  stops using the endpoint — otherwise an idle row would go on showing the arrival pattern of
  a match that ended.
- **Every passive figure is refused rather than guessed when its precondition fails.** No
  stall while our own sending has stopped; no shortfall unless our send rate held; no arrival
  spread under eight updates; no shortfall below five per cent, which is the arithmetic of the
  comparison rather than a finding. Each of those refusals is a test.

## Phase 6 — Service status, game reference pools & diagnosis verdicts

Goal: at-a-glance "is it them or me", including "the game's servers are down (or partly)".

- [x] Status check definitions as data: `assets/targets/services.json` — Steam, Epic, Discord, Riot, EA/Origin, Battle.net, Xbox Live, PSN, AWS, GCP, Cloudflare (per-service: endpoints + probe kind)
      — eleven services, thirteen endpoints, schema and rules in `assets/targets/README.md`.
      **Names, never addresses**, and a test enforces it: a platform's front door lives on a
      content network whose address depends on where the user is, so a bundled literal would
      pin whichever edge the *developer* was nearest and go stale silently.
      **The probe-kind field is a hint, not a permission.** It reorders the kinds
      `preferred_kinds` has already judged honest for the address and can never introduce one
      it refuses — a tunnelled endpoint still gets the end-to-end probe whatever the file
      says, which a test in `nm_probes::chain` pins. It earns its place by arithmetic: the
      ordinary chain opens on the cheapest kind and needs three silent checks — over two
      minutes at this cadence — before reaching the one that works, which on a status page is
      a red card about a service that is up.
- [x] Periodic low-frequency checks (e.g. 30–60 s) reusing the probe engine
      — 45 s per endpoint, on the one probe engine and the one registry everything else
      shares; a second runner would have a second token bucket and quietly double the traffic
      the product promises not to send. A test asserts the whole list costs under a third of a
      probe a second against the cap of thirty-two, because these checks run whether or not
      the user is doing anything.
- [x] Status page UI: service cards with state (reachable / slow / unreachable), latency, last-checked; grouping (platforms / infra)
      — `src/features/status-page/`. **The verdict rule is not the dashboard's**
      (`nm_core::status`), and that is the phase's main design decision. A baseline reports
      what a window has been like; a card asks whether a service answers *now*, and at a check
      every forty-odd seconds a window rule answers that badly at both ends — a service that
      died a minute ago still reads mostly green, one that has just recovered still reads
      mostly red. The rule reads the most recent checks instead and reacts within one interval
      in both directions.
      It still refuses to call one lost check an outage: the card leaves `Ok` at once and the
      strip shows the failed check, but the word *unreachable* waits for the second
      consecutive failure — the same reasoning as `min_delivery_attempts`, since a status page
      that flashed "Steam is down" on every lost packet would be worth nothing on the day it
      was true.
      **A card states what this machine can reach**, never that a company's service is down:
      from inside a filtered network those two are indistinguishable and only one of them is
      observable here. The page says so in as many words.
      A card whose last check is older than two intervals says *that*, rather than showing a
      larger number — a status page whose data quietly stopped arriving looks exactly like one
      reporting calm.
- [x] Per-service history (session-scoped ring buffer) with mini-timeline
      — one cell per check, oldest left, capped at 24 of the 60 retained. **A cell is a fact,
      not a verdict**: it keeps the four ways a check can fail apart, which one colour on the
      card cannot, and it is what distinguishes "a packet went missing twenty minutes ago"
      from "nothing has answered since". Colour is never the only channel — every cell carries
      its translated word.
- [x] Game reference pools — bundled seeds as data: Valve SDR POP ping endpoints (from Valve's published SDR config), AWS GameLift ranges, Riot/Blizzard known targets; refreshed only via app releases or explicit "Update target lists" action
      — `assets/targets/pools/valve-sdr.json`, eight points of presence read out of the SDR
      network configuration Steam publishes, **not from a guessed naming scheme** — the
      correction Phase 2 demanded. One file seeds both Valve titles, because one relay network
      really does serve several and duplicating it would guarantee the copies drift.
      **Two of the three named sources were rejected rather than faked.** AWS GameLift
      publishes CIDR ranges, and a CIDR block is not a reachable address: probing an arbitrary
      member of one measures nothing. Riot and Blizzard publish no reference address that
      answers a probe at all. Inventing entries for them would have reproduced exactly the
      failure the SDR hostnames already caused, with a confident face on it — so those titles
      have **no bundled pool**, their pool is whatever this machine learns, and the page says
      plainly that until it has learned something it cannot tell a game's outage from a path
      the user cannot reach.
      Pools are **addresses**, the opposite of the status page's rule and for the opposite
      reason: a pool asks whether specific machines answer, and a name resolved once at
      start-up would be measured forever against whichever of them the resolver picked.
- [x] Learned endpoint history: persist endpoints the user connected to, tagged per game preset (cap ~32/game, LRU, expire after N days unseen); cold start covered by bundled seeds
      — `nm_core::pool` for the model, `nm_app::pools` for the file. Thirty-two learned
      entries per preset, least-recently-seen evicted, expiring after a fortnight unseen.
      **Wall clock appears here and nowhere else in the product**: the span has to survive the
      app being closed, which is the exception `CLAUDE.md` allows. Nothing here times a probe,
      so a clock that jumps can only expire an entry early or late.
      Expiry is not tidiness — **game server addresses rotate**, so a stale entry is as likely
      to be a stranger's machine as the game's, and a pool full of those would report an
      outage that is not happening.
      **It writes a record of where the user plays**, so it is a setting rather than an
      assumption: on by default because the feature is worthless without it, and turning it
      off deletes what was already written rather than merely stopping new entries. For this
      audience that choice is worth one boolean.
- [x] Reference-pool trickle probing (round-robin, active only while the game is monitored or on explicit "Diagnose"), inside the global probe budget
      — registered with the shared engine at one probe per target every five minutes and left
      there while the game is watched; the engine's own scheduler spreads them, its fallback
      chain gives each a history, and the rate cap covers them like everything else. About a
      tenth of a probe a second per game. A sweep that changes nothing asks the engine for
      nothing, which a test pins, because it runs every five seconds for as long as a game is
      monitored.
      **The explicit "Diagnose" action is deliberately not shipped.** The pool is already
      probed continuously for every monitored game, so a button would only re-ask a question
      being answered the whole time; the case it would actually serve — diagnosing a game that
      is *not* being monitored — needs targets the app has no reason to hold, and a burst of
      probes on a button press is exactly the shape the rate cap exists to prevent.
- [x] Diagnosis verdict engine in `nm-core`: pure rules combining baselines + app metrics + path-probe death point + reference-pool response ratio + platform-API status into verdicts (ISP / border / routing-to-game / game servers down / partial outage); every matrix row unit-tested; verdicts phrased as network-level facts only
      — `nm_core::diagnosis`, pure and exhaustively tested, with the matrix written out in the
      module documentation. **The ordering is the argument**: the general network is settled
      before anything is blamed on an application, because an application's endpoints failing
      while the whole network is failing says nothing about that application — and a user sent
      to a game accelerator over their own broken line has been wasted. A test asserts exactly
      that, and its mirror for a border problem.
      **A border is named only when the domestic baseline corroborates it**, which is the
      claim `nm_core::path` deliberately refuses to make on one traceroute — and even then it
      names a *path*, because throttling, a blocked route and a broken transit link are
      indistinguishable from here.
      **Absence of knowledge is never a finding.** Every probe filtered produces "nothing
      could be measured", never a confident verdict about a border; an endpoint proven alive
      by its own traffic is not counted as a failure, since that is the normal state of every
      UDP match server; and an application whose every endpoint was merely filtered reports
      that it cannot see rather than that everything is clear.
      Two inputs the plan named are **not** read, and neither is a gap: the path-probe death
      point is already the input to Phase 5's own panel and adding it here would let one
      traceroute outvote the baselines, which is the failure mode the border rule exists to
      avoid; and there is no "platform-API status" to combine, because the product fetches
      nothing — what stands in its place is the status page, measured the same way as
      everything else.
- [x] Verdict surfacing in UI (dashboard + app-monitor banner). A verdict covers *the endpoints
      it actually explains* and says which — an app whose voice endpoint is blocked while its
      game server is clean gets a verdict about that endpoint, not about the app. See the
      per-endpoint state requirement in Phase 4: partial failure inside one application is the
      normal case under filtering, not an edge case.
      — `src/shared/VerdictBanner.tsx`, on the dashboard above the two baselines it is drawn
      from and on every application card. **It is always shown, including when it has nothing
      to say**: a banner that appeared only on bad news would make its absence mean "fine", and
      the state before anything has been measured would read as good news.
      It states its scope — "about 2 of 7 endpoints" — and keeps **what to try separate from
      what was observed**, because they are different claims: the app can see that the evidence
      points past the border and cannot see whether a VPN would help. It never suggests one for
      a failure inside the user's own network, which would waste their time and, in some
      places, expose them for nothing. A test pins that.
      Beside it, `PoolPanel` shows the evidence the game-server verdicts rest on, including
      what the pool *cannot* speak for.
- **Accept**: page reflects a manually blocked host (hosts-file test) within one check interval; service list extendable by editing JSON only; simulated scenarios (mocked probe outcomes) produce correct verdicts incl. partial game-server outage; stale cache entries expire and never fake an outage.
  *Met in the parts that tests can establish, and the gap is named rather than papered over.*
  The service list is extendable by editing JSON only — schema, validation and the budget rule
  are all data-driven and covered by `tests/services.rs`. The simulated scenarios are
  `nm_core::diagnosis`'s 24 unit tests, one per row of the matrix including the partial
  game-server outage and every row whose honest answer is "we could not see". Stale entries
  expiring without faking an outage is covered twice over: `nm_core::pool` replays expiry,
  eviction and a clock that steps backwards, and `tests/pools.rs` asserts an expired entry
  stops being probed.
  **Not verified: a manually blocked host on a running build.** The reaction time is a
  property of `StatusThresholds` and is replayed against synthetic checks — a card leaves `Ok`
  on the first failed check and reads *unreachable* on the second — but no host was actually
  blocked with a firewall rule while the app was running, and the numbers on a real status
  page have not been looked at.

**Phase 6 status: every item is built. Two things are not verified and neither should be
read as done.**

- **The build was run, and the dashboard's verdict was seen rendering** on 2026-08-02 — which
  immediately found the defect recorded below. **The status page itself has not been seen**,
  no service has been blocked to watch a card react, and no pool has been probed against a
  live game. Everything else above is unit-tested against synthetic checks, fake clocks and
  fake registries.
*One thing that **is** verified, because it was the riskier of the two unknowns:* every
bundled address was probed from this machine on 2026-08-02, with the probe kind the app will
actually use. **All thirteen service front doors answered a TCP connect**, and **all eight
Valve relays answered an ICMP echo** — so neither list is the kind of guesswork that produced
Phase 2's unusable SDR hostnames, and a pool built on those seeds will have members that can
prove they answer. Timings are not recorded here: they describe a real person's network.

Phase 6 decisions worth remembering:

- **A verdict must read the distribution, never the group's headline.** Found within a minute
  of running the build: the dashboard announced *"services inside your country answer and
  services abroad do not — the path out"* while three of four foreign targets were perfectly
  clean. The fourth was the tunnelled member reading 177 ms, which made the group `Degraded`,
  and `Degraded` read as a headline became a claim about a border. `GroupHealth` calls
  anything less than a clean sweep degraded — correct on a card that shows the counts beside
  it, wrong as the input to a conclusion. A group now fails only when **more than half its
  judged members answer nothing at all**, which is exactly why the lists are built from
  diverse operators: one of them down is that operator's problem, all of them down is a
  network's.
  **Latency alone was dropped as evidence and loss was added in its place.** A slow member is
  an answering member, and a figure high because of a tunnel or an ocean is not something
  being done to the user — but throttling, which is the commoner shape of censorship than
  outright blocking, arrives as *loss*, so a rule that only asked "does it answer" would have
  missed the case the product mostly exists for. A group losing a tenth of every packet across
  four unrelated operators is a path problem by construction.
  This is the second time in this phase a verdict had to be weakened from what the plan
  originally wrote, and both times for the same reason: the strong claim was reached from
  evidence that did not support it.
- **A pool member only counts once it has answered.** Found by reasoning about what a learned
  entry actually is: an endpoint a game connected to, which for a UDP title is a match server
  that answers nothing anyone can send — by design, while the match runs perfectly. Counted as
  unreachable, a pool built from those would report a working game as down on *every* match.
  So silence means nothing until a member has shown it can answer; before that it is an
  address with no baseline, held out of every ratio and reported as such. It is the same lie
  `Health::CarryingTraffic` was introduced to prevent in Phase 4, arrived at from a new
  direction, and it is the single most important line in this phase.
- **`GroupHealth::of_judged` exists so one piece of arithmetic serves three rules.** The
  headline-plus-distribution logic is the same whatever decided each member's state, and a
  second copy of "anything mixed is degraded, and the counts say how much" would drift from
  the first. The baselines, the status page and the pools all roll up through it.
- **The status page and the pools take opposite rules about names and addresses**, and both
  are right. A front door must be a name, because its address depends on where the user is; a
  relay must be an address, because a name would be resolved once and then measured forever
  against whichever machine the resolver picked.

## Phase 6.5 — The page a player can read (amendments from use, 2026-08-01)

Stated by the user after running the build, in their order. Everything here is about what the
page *says* and how it behaves under a reader; **nothing here changes what is measured, and no
figure is deleted** — item 4 moves several of them one level down, and that is the whole of it.

The audience this phase is written for is a player, not an engineer: someone who knows their
game stutters and does not know what jitter is. The depth stays reachable for whoever wants it,
because a measurement tool that cannot explain itself is asking to be trusted on faith, and this
audience has no reason to extend that.

- [x] **1. UDP first: that is where the match is played.** The endpoint list is one flat list
      ordered by severity alone (`nm_app::view::severity`), with transport as a badge on the
      row. During a game the endpoints that matter are the UDP flows, and today they sit
      wherever their health happens to put them, between a launcher's TCP connection, a CDN
      and a telemetry host. What changes:
      – **Two labelled groups**: the match traffic (UDP) first, the supporting connections
        (TCP) below. Grouping is by transport; the ordering *inside* a group stays exactly the
        worst-first severity judgement Rust already makes, and stays in Rust.
      – **TCP is demoted, never hidden.** For this audience a blocked or throttled TCP endpoint
        is a first-class finding — a login service or a CDN with a filter sitting on it is what
        "I cannot get into the game" actually looks like. So the TCP group carries its own
        distribution in its header, and a member in any state worse than `Ok` is surfaced
        there; the group may start collapsed only when every member is `Ok`.
      – **The chart follows the same emphasis**: UDP lines at full weight, TCP lines lighter.
        Colour still identifies and never states health — the list remains the authority.
      – **The honest caveat this creates.** Without the one-time tracing setup there are *no*
        UDP endpoints at all, so on such a machine the match-traffic group is empty. An empty
        group must say why and point at the same explanation the flow-status banner gives; an
        unexplained empty "match traffic" reads as a game that plays over nothing.
      – Probe-budget ranking is untouched: recent bytes then recency, in `nm_core::endpoint`,
        which already finds the match server on its own. This item is presentation only.
      — `AppView.groups` replaces the flat endpoint list: two groups, always both, the match
      traffic first. The grouping is Rust's, like the severity ordering it now applies
      *inside* each group, and each group carries its own `HealthCountsView` plus
      `needsAttention` — whether anything in it is worse than `Ok`. That last one is sent
      rather than derived in TypeScript for the usual reason: what a user must not miss is a
      judgement, and the supporting group may only start folded when it is false.
      **Both groups travel even when empty**, which is the honest-caveat half of the item: an
      absent match-traffic group would read as a game that plays over nothing, so the page
      states either "nothing of this kind has been seen yet" or "this machine cannot discover
      it, and here is why" depending on the flow status it is given.
      The chart draws every endpoint regardless — "which of these is the odd one out" is a
      question about all of them — and takes the grouping as emphasis only: match traffic at
      full weight, supporting connections lighter and thinner. Colour still identifies and
      never states.
- [x] **2. Names for the games, not file names.** `assets/apps/presets.json` labels six
      applications; everything else shows an executable name, so the titles the user named —
      World of Tanks, Forza Horizon, Deadlock — appear as `WorldOfTanks.exe` and the picker
      reads like a task manager.
      – **Split the two jobs a preset does today.** *Grouping* joins several executables into
        one application; *labelling* names one. A single-process game needs no grouping and
        still needs a name. This matters for the existing safety rule rather than against it:
        "never list an executable several applications share" is a rule about **grouping** —
        `steam.exe` may be *labelled* "Steam" as long as labelling joins nothing to it. The
        schema gains a name-only entry, and the test that refuses a shared executable keeps
        applying, unchanged, to grouping entries.
      – **Fill the list out**: the five titles of item 7 first, then the launchers and platform
        applications a player meets in the picker (Steam, Epic, Riot, EA, Battle.net, Xbox) and
        the usual companions (Discord, browsers). Every executable name verified against a real
        installation, not recalled — a wrong name is a preset that never fires and a label
        nobody ever sees, and it fails silently.
      – Labels stay proper nouns, shown as written and never translated. The picker shows the
        label with the executable name beside it: a grouping the user cannot inspect is one
        they cannot correct.
      — `presets.json` gained a second list, `labels`, with its own validation: one executable
      each, never one a grouping entry claims, ids unique across both. The shared-executable
      test keeps applying to grouping alone, which is exactly the split the item asked for —
      `steam.exe` may now be *named* and still may never be *grouped*.
      **The five titles were read off real installations on this machine**, not recalled;
      all five are on a Steam library here, which is also what makes the Phase 6.5 acceptance
      pass runnable at all.
      **Then the scale problem, raised by the user: five titles is not a games library and
      hand-checking does not scale.** There is a ready-made reference — Discord publishes the
      index it uses to recognise a running game — so `assets/apps/labels.json` now holds 9 314
      generated names, consulted only *after* the curated lists so a curated entry always
      wins. `scripts/build-app-labels.mjs` produces it, and **the application never runs it**:
      the script reaches the network, the output is committed and compiled in, and a release
      is the only thing that changes it — the rule the target lists already follow. No licence
      is stated for that endpoint and `assets/apps/README.md` says so rather than assuming it
      away.
      The filter is harsh because a confident wrong name is worse than a file name: dropped
      are 408 names claimed by more than one title, generic names and runtimes, and everything
      with fewer than four characters before the extension — the index really does claim
      `at.exe`, which has shipped with Windows since NT. A catalogue entry can only ever
      supply a *name*, never a grouping, so its worst failure is a wrong label rather than
      another program's traffic in a game's endpoint list.
- [x] **3. The chart grows from the left instead of sliding under the cursor.**
      `nm_core::series::Grid` ends at `now`, so a fresh application draws a short line pinned to
      the right edge with empty space behind it, and every emission walks the whole picture one
      second to the left — which is what makes a line something the user has to *catch* with
      the pointer.
      – **Anchor at the beginning.** Time starts where monitoring did, at the left edge, and the
        drawing grows rightwards until it fills the window's span; only then does the window
        begin to scroll.
      – **Sliding becomes stepping**, and the mechanism already exists: slots are three seconds,
        so quantising the axis to slot boundaries advances the chart once every three seconds
        instead of drifting every one. Cheapest fix first; if it is not enough, hold the axis
        still while the pointer is inside the chart and catch up when it leaves.
      – The axis then wants labelling as elapsed time from the start rather than as negative
        ages, and the note under the chart has to agree with it.
      — `Grid` is now anchored: it takes the instant monitoring began and places samples in
      slots counted in whole steps from *there*, so `AppView.chartElapsedSecs` runs `0, 3, 6,
      …` and the axis is labelled `0:00`, `0:03`. Both halves of the fix fall out of that one
      change. The whole ladder is emitted from the first moment, which fixes the width of the
      axis so a fresh application's line **grows into it from the left** rather than being
      stretched across it or pinned to the right; and because slot boundaries are whole steps
      from the anchor, the picture **steps once every three seconds** instead of drifting on
      every emission. A test pins that the axis does not move part-way through a slot and does
      move when the boundary is crossed — which is the property the pointer needs. The
      second, harder fallback the item allowed (freezing the axis while the pointer is inside)
      was not needed.
      Each application is anchored separately, and stopping and restarting one starts a new
      chart — the old session's axis says nothing about the new one.
- [x] **4. The vocabulary: what a player reads first, and what waits for whoever asks.** The
      page states everything it knows at once — probe kind, proven filtering, both egress
      addresses and their adapters, the window a rate is taken over, five separate passive
      figures — and the result is that the numbers that matter are indistinguishable from the
      caveats attached to them. The UDP panel is the worst of it. **Two levels of depth, not
      two products**: the same data from Rust, rendered at the depth the reader asked for.
      – **Level one, the default row**: what it is, one word of state, and three figures —
        response, stability, loss. Nothing else.
      – **Level two is a per-row expander**, and there is **no setting**. A mode is a second
        product to keep consistent and one a user forgets they are in; an expander is a
        question asked and answered in place. What moves there: probe kind, proven filtering,
        egress address and adapter, the span a rate covers, the incoming byte rate, the hop
        count and where the route stops. An egress *conflict* does not move — it is a warning,
        not a detail.
      – **Level three is an ⓘ on every metric**: a tooltip of one or two plain sentences on
        hover *and* on focus — keyboard-reachable, like everything else on this page — with
        "Learn more" opening a **bundled** help page at that metric's own section.
        Bundled, not a website: an external link is a network request this product promised not
        to make on the user's behalf, and it is useless to a user who is being filtered. The
        renderer supports links so that they can be added later, and any link it is given opens
        in the system browser as an explicit act; **none ship now**.
      – **The UDP panel is reworded, not thinned.** Its five figures stay; what changes is that
        each one is named for the thing the player experiences: the server's own update rate,
        the smoothness of arrivals, the worst pause, the drop-off against this endpoint's own
        recent past, and — kept prominent, not demoted — a freeze. The drop-off keeps its
        careful name: it is not loss, because only the far end knows what it sent.
      – **The route panel leads with one number** ("the round trip to a router on the way, not
        to the server") and puts its hop count and stopping point on level two. The rule Phase 5
        exists to protect is unchanged: this is never labelled a round trip to the server, and
        it never merges with the flow figures.
      – **"Why this is not the ping your game shows" is its own block on the match-server card**,
        collapsed to one line, and the first section of the help. Three points: the game times a
        packet only it can answer; we do not read its protocol, because that means a capture
        driver, its memory, or its wire format, and all three are refused; so what is shown is
        the path and the flow, and their disagreement is the diagnosis. **This is the single
        most important string in the application** — without it the honest answer looks like a
        wrong one, and the user concludes the tool is broken rather than that the number they
        knew was never what they thought.
      – **Endpoints are not labelled by role** — no "match server", no "voice", no "login".
        Everything except the transport and the traffic volume would be a guess, and a wrong
        label on the right number is worse than no label. Naming a destination is the Phase 8+
        enrichment item, where it is done from data rather than inference.
      – Every string is an i18next key, so this is additive for Russian, help text included.
      — the row is now three figures and one word of state; everything that *qualifies* a
      number rather than being one moved into the row's own `<details>`. A test asserts what
      the **closed** row contains and that the moved fields are present but not visible,
      because the whole item is about what is not shown by default and that is exactly the
      kind of thing that grows back one field at a time.
      **Nothing was deleted and no figure lost its meaning.** The UDP panel keeps all five,
      renamed for what a player feels — updates from the server, evenness, worst pause,
      drop-off, and a freeze kept prominent. The drop-off keeps its careful name and a test
      pins that it is never called packet loss. The route panel leads with "round trip to
      that router" and sends its hop count and stopping point down a level.
      **The ⓘ is a disclosure, not a hover tooltip**, and that is a correction the plan's
      own requirement forced: a "Learn more" that vanished the moment a keyboard user moved
      towards it would not be reachable at all. It opens on focus and on hover, and the
      pointer can travel into the panel because the handlers sit on the wrapper.
      The help is a page of its own — thirteen topics, one list shared with the ⓘ so a
      tooltip can never point at a section that does not exist. **No link ships, and that is
      a decision rather than an omission**: following one honestly means opening the system
      browser, a link that navigated the window would leave the user with no application, and
      that means a Tauri plugin. Adding a dependency for zero links is not a trade worth
      making; it becomes one the first time a link earns its place. The plan asked for a
      renderer that supports links — this is the deviation, and the reason.
      "Why this is not the ping your game shows" sits on any row whose own round trip is a
      dash while a route or its traffic speaks for it — which is what a match server looks
      like, and without naming any endpoint as one.
      **This item is now a standing rule rather than a one-off**, at the user's instruction:
      the three levels, the "no setting", the never-demoted warning and the "a new metric
      ships with its help topic" requirement are written into `CLAUDE.md` and apply to every
      feature built from here on, including everything in Phases 7 and 8+.
- [x] **5. A warm-up window, so the first seconds are not read as findings.** The samples right
      after an application is picked are the least informative it will ever have: no window is
      full, jitter is computed over a handful of samples, the fallback chain is still trying
      kinds, ranking has not run yet, and the flow figures need eight updates before they say
      anything at all. The page presents all of it as measurement.
      – Show the application, and each newly discovered endpoint, as **warming up** for a stated
        period with the remaining time visible, and withhold the *derived* figures — jitter,
        loss, shortfall, the verdict banner — until the window behind them is real. **Rust
        decides this**, not the UI: it is the same "absent knowledge stays absent" rule that
        already withholds a figure whose precondition failed. The duration follows the health
        window (`nm_app::monitor::health_window`) rather than becoming a new constant.
      – **A warm-up must never hide something already certain.** Filtering proven, unreachable,
        carrying traffic proven by bytes, a stall — those are answers, and arriving fast is the
        point of them. Warm-up suppresses figures that are still noisy, never states that are
        known.
      – And it must end. Nothing discovered is a state of its own — "no endpoints yet, and here
        is what that means" — not an indefinite spinner.
      — the warm-up is the health window, as the item asked, and it is computed in Rust from
      the endpoint's own first sighting and from the moment the application was chosen.
      Withheld while it runs: steadiness, lost replies, and the traffic drop-off — each of
      them a figure whose *denominator* is still a handful. **The round trip is not
      withheld**: one reply is one real measurement of the route, and the mean of a few is
      the mean of a few.
      **The verdict banner waits by not being given anything to conclude from.** During
      warm-up the application is simply not offered as evidence — which is exactly what the
      verdict engine's existing `None` already means — so the banner falls back to the
      general network, which has been measured since the session began and is not less
      certain for the application being new. Nothing needed a new rule.
      A test pins that the same evidence produces "the route this application takes" once the
      window is real and never during warm-up, and another pins that filtering proven,
      nothing getting through, and alive-by-its-own-traffic all still arrive at full speed.
- [x] **6. The page must stop jumping.** Four separate causes, and they need separate fixes:
      rows re-sort as health flickers between states; a row changes height when an optional
      panel or field appears (path, flow, the stack round trip); endpoints appear and disappear
      as discovery finds and forgets them; the chart is rebuilt whenever the *set* of lines
      changes.
      – **Hysteresis on the ordering, in Rust.** A health change moves a row only once it has
        held for a few seconds, while the badge on the row changes immediately — so the order is
        stable without any state on screen ever being stale. Ordering is a judgement and stays
        where the judgements live.
      – **Reserved space** for the optional panels and fields, so their arrival changes nothing
        above them, and a fixed chart height across a rebuild.
      – **A pinned endpoint holds its place.** Pinning is the user's own answer to a page that
        moves while they read it, and it must survive re-sorting, new endpoints and a chart
        rebuild.
      — **the hysteresis separates two uses of one verdict** (`nm_core::settle`): what the
      badge shows changes at once, so nothing on screen is ever stale, while what the
      *ordering* uses adopts a state only once it has held for five seconds. A flicker
      therefore never moves a row, because it never survives long enough to settle. A test
      replays sixty seconds of a state alternating every second and asserts the order never
      moves; another asserts that a change which persists does move it.
      The ordering sorts the *reports*, not the views, so the settled state never crosses the
      IPC boundary — it explains an ordering the UI does not compute. `AppMonitor::endpoints`
      takes `&mut self` for this: how long a state has held can only advance where both the
      verdict and the clock are known, and reports are produced on the emission beat, which is
      exactly the cadence the hold is counted in.
      **Reserved space**: the chart already had a fixed height; added are the badge strip —
      the one part of a row that really does flicker, as a warm-up badge and a warning come
      and go — and the metric values, so a figure alternating between a number and a dash
      cannot change a line's height. The largest single win was item 4's: the optional
      *fields* moved into the expander, so their arrival now changes nothing at all.
      **The pin is positional** (`holdPlace`, pure and tested): it returns the pinned row to
      the index it occupied when it was pinned, and leaves every other row in Rust's order. A
      pin that promoted the row to the top would be the same jump the pin exists to prevent.
      It survives a new endpoint above it, the list shrinking under it, and an endpoint that
      is no longer listed at all.
- **Accept**: on a live session, the match traffic is the first thing on the page and named as
  such; a title from item 2's list appears by its own name; the chart can be pointed at without
  chasing it, and neither a new endpoint nor a health change moves the row being read; the first
  seconds of a capture show a warm-up rather than a verdict; a closed row carries three figures
  and no caveats, every one of them has a keyboard-reachable explanation, and the match-server
  card says in as many words why the game's own ping is a different number.
  A test asserts what the *closed* row contains, because the whole item is about what is not
  shown by default, and that is exactly the kind of thing that grows back one field at a time.

**Phase 6.5 status: items 1–6 are built and tested; nothing here has been seen running.**

Every rule above is pinned by tests — the grouping and its ordering, the anchored chart and
its stepping, what a closed row contains, the warm-up and what it refuses to hide, the
hysteresis and the pin. What no test can establish is what the page *looks like*: jsdom has
no canvas, and "the chart can be pointed at without chasing it" is a claim about a pointer.
The live pass that would settle it is the item below, now at the end of Phase 7 — so this
phase's acceptance criterion is met in the parts tests can reach and open in the parts they
cannot, exactly as Phases 4 and 6 recorded theirs.

## Phase 6.6 — The words on the page (amendments from use, 2026-08-02)

Stated by the user after running the Phase 6/6.5 build. Nine items, **ordered by priority
here rather than as they were stated** — the ordering is mine and the reasoning is under each
one. It runs before Phase 7 and after 6.5, because every item here is a correction to what
6.5 shipped, and Phase 7's five-title verification pass should be run against the page as it
will actually ship rather than against one that is about to be reworded.

Nothing here changes what is measured. Two items *remove* things from the screen (2, 9) and
one removes rows from a list (1); the rest are naming, explanation and page structure. As in
6.5, **no figure is deleted** — a figure that leaves level one moves down a level, and the one
case where a figure disappears entirely (item 9) is a figure that was never there, shown as a
dash.

- [x] **1. Common network vocabulary: ping, jitter, loss — and this reverses a standing rule.**
      *First, because it is the largest comprehension change on the page, because every other
      item here writes text that has to use these words, and because it contradicts a rule
      written into `CLAUDE.md` and must not be discovered halfway through a later item.*
      The row today reads *Response*, *Steadiness*, *Lost replies*. The user's instruction is to
      go back to the names a player has seen in every other tool they have ever used — **RTT
      (or "Ping", if it is the same quantity), jitter, loss** — and to explain the rarer ones
      with the ⓘ that already exists, rather than to rename the quantity around the reader.
      – **`CLAUDE.md` must be amended in the same commit.** Its two-level rule currently says a
        level-one figure is "named for what the user *experiences* … never for the quantity an
        engineer would name". That clause is what produced *Steadiness*, and the user has now
        rejected it. The replacement rule: **level one carries the standard network term; the
        ⓘ carries the plain-language sentence.** The rest of the two-level rule is untouched —
        three figures, one word of state, everything that qualifies a number one level down.
        A plan that quietly disagreed with `CLAUDE.md` would be worse than either rule.
      – **"Ping" only where it *is* the round trip we measured.** The word is the one the
        audience knows, and it is also the exact word Phase 5 exists to keep honest. So: the
        probe figure may be called **Ping (RTT)** on an endpoint whose own round trip we
        measured; the route figure is **never** called ping, keeps its "to a router on the way,
        not to the server" wording, and the *Why this is not the ping your game shows* block
        stays exactly where it is. A test pins that the route panel never renders the word.
      – **The passive flow figures keep experiential names**, and that is not an exception to
        the new rule but an application of it: *arrival jitter* is a standard term and gets it,
        while "the server's update rate" and "worst pause" name quantities that have no
        standard network term to return to. What must not happen is a second figure called
        jitter with no way to tell it from the probe's — the flow panel's is labelled
        **arrival jitter** wherever both can be read at once.
      – Every renamed key gets its ⓘ sentence rewritten to match, and the help topic list moves
        with it: the ⓘ and the help read from one list, so a rename that missed a topic is a
        tooltip pointing at a section that no longer exists.
      – Units belong to the figure, not the tooltip: `ms`, `%`, through `Intl` as now.
      — the row now reads **Ping (RTT) · Jitter · Loss**, and `CLAUDE.md`'s level-one clause was
      rewritten in the same commit, with the reversal recorded in it rather than silently
      replaced: a plan and a contract that disagree are worse than either rule.
      **The word "ping" earns its place by being true where it is used.** It sits on the probe
      figure, which really is a round trip we measured, and the route panel is pinned by a test
      that asserts the whole panel's text never matches it — not the label alone, since the word
      could creep back in through a note or a hop line. The *Why this is not the ping your game
      shows* block is untouched, and it now sits beside a figure the reader recognises rather
      than beside an invented one, which makes it easier to read rather than harder.
      **The flow panel's spread of arrivals is `arrival jitter`, never `jitter`.** Both jitters
      can be read on one card, so the qualifier is the whole point; a test asserts the bare word
      appears nowhere in that panel. `updates`, `worst pause` and `drop-off` keep experiential
      names because no standard term exists to return to — and the drop-off's help now says in
      as many words how it differs from the loss figure it sits near, which the rename made a
      sharper question than it was.
      Three help topics were renamed with the figures (`response`→`rtt`, `stability`→`jitter`,
      `smoothness`→`arrivalJitter`) and every ⓘ sentence rewritten around the new names, because
      the ⓘ and the help page read from one list and a missed rename is a tooltip pointing at a
      section that no longer exists.
- [x] **2. An endpoint nothing can probe shows no ping, jitter or loss at all.** *Second,
      because it is the same complaint as item 1 aimed at the most important row on the page:
      the match server, which is what the whole product exists to watch.* A UDP endpoint that
      answers no probe kind currently renders three dashes where the three headline figures
      go, and three dashes read as a broken tool rather than as an honest absence.
      – **Rust decides, and the row changes shape rather than emptying.** Where `measurable` is
        false — no probe kind can honestly measure it, which for a game's match server is the
        normal and permanent state — the row does not render the probe figures at all. What
        takes their place is what *is* known: the route figure and the flow figures, which are
        already computed and already on the card.
      – **This is not a relaxation of "absent stays absent".** That rule forbids replacing a
        missing figure with a nearby number; it does not require printing a dash forever for a
        quantity that will never exist. The distinction is between *not yet* and *never*: an
        endpoint still working through its fallback chain keeps its dashes, because a figure is
        coming. A test pins both halves, since the difference between them is exactly the sort
        of thing a later refactor flattens.
      – The reason stays reachable: the expander says which kinds were tried and what the
        endpoint answered, as it does now.
      – The chart is unaffected — a silent endpoint is already drawn as its dashed route line.
      — `EndpointView.probesMeasureIt`, decided in Rust from the fallback chain and nothing
      else. It was already being computed — `measurable && !walking_path` is the predicate that
      decides whether "not measured yet" is still the honest word for a silent endpoint — so
      the same fact now answers both questions instead of being derived twice.
      **`measurable` alone would have been the wrong field**, and quietly so: a match server
      being path-walked is still *measurable*, because a route walk is a measurement of
      something. What the row needed was the narrower question — is a probe kind aimed at the
      endpoint itself still in play — which is false for a match server and for the tunnelled
      endpoint that exhausts even the TLS hello.
      **A figure that exists is never hidden to change shape.** Where the window still holds a
      real sample from a kind since ruled out, the block stays until it ages out; the row
      changes shape once, when there is nothing left to shape, rather than dropping a
      measurement the user could still read.
      Two tests pin the two halves in Rust — a chain still trying kinds against one that has
      run out — and two more in the page: the match-server row has no *Ping (RTT)* and says in
      one line why, and a row whose figures have not arrived yet keeps its three dashes.
- [x] **3. The status page must explain its own numbers and its cells.** *Third, because it is
      the same defect as 1 and 2 on a page the user could not read at all: they asked what the
      figures and the coloured cells mean, which is a page failing at its one job.*
      Today a card shows *Latest*, *Mean*, *Loss* with no statement of what a check is, and a
      strip of green and red cells with no legend, no time span and no per-cell reading.
      – **The strip needs a legend and an axis.** One cell is one check; the strip covers the
        last N checks, which at the shipped cadence is a stated number of minutes; oldest is
        left. All three facts are on the page rather than in the source. Every cell already
        carries its translated word (`answered` / `slow` / `lost` / `refused` / `filtered`) —
        that stays, and gains the check's own time, reachable by keyboard as well as by hover.
      – **The three figures get the same treatment as item 1**: standard names, an ⓘ each with
        the plain sentence, and the window they cover stated. *Latest* is one check, not an
        average, and a page that does not say so invites the reader to average them by eye.
      – **Colour is never the only channel**, as now; the legend must read correctly for a
        reader who sees neither green nor red.
      – The existing caveat — that a card states what *this machine* can reach — is a warning
        and stays at level one, per the standing rule.
      — **the legend is one block for the page, not one per card.** The strip is the same strip
      on thirteen cards, so the three facts sit above them once: one cell is one check, oldest
      on the left, and a full strip is the last 24 checks — about 18 min at the shipped cadence.
      The count comes from Rust (`ServiceStatus.timelinePoints`) rather than from counting
      cells, because a fresh endpoint has fewer and the claim is about a *full* strip.
      **A cell's own time is reachable by keyboard, and that cost the `title` attribute.** A
      tooltip is unreachable without a mouse, so the reading moved to a line under the strip
      that both hovering and the arrow keys write into. **One tab stop for the whole strip**,
      not one per cell: thirteen services would otherwise put several hundred stops between a
      keyboard user and the next thing they wanted.
      Colour stops being the only channel in three places at once — every cell still carries
      its word for a screen reader, the readout writes it out, and the legend names all five
      states beside their colours. `CHECK_MARKS` is `satisfies readonly CheckMarkView[]`, so a
      new Rust variant is a compile error rather than a colour with no name.
      **Every figure now says what it is and over what.** *Latest* invited exactly the wrong
      reading — averaging the strip by eye — and is now *Ping, last check*; the mean and the
      loss carry the span they cover; and the card's headline was a bare number with nothing
      saying which round trip it was, so it is labelled *Ping, median* and says why a median.
      Four help topics were added for them, since the ⓘ and the help read from one list.
- [x] **4. The Applications page carries too much prose.** *Fourth: it is a real complaint about
      the busiest page in the product, and it is deliberately behind 1–3 because the text that
      survives it should be written in the vocabulary those items settle.*
      Level one is supposed to be a name, a word of state and three figures; the page also
      holds a paragraph under the chart, a paragraph of picker hint, a pool note, a warm-up
      explanation, two group hints and several multi-sentence banners.
      – **Audit every block of running text on the page** and give each one exactly one
        destination: an ⓘ tooltip, a section of the help page, the row's own expander, or
        deletion where it repeats something the reader has already been told. The chart note is
        the worst of them and is the test case: it is six sentences describing decisions
        (log axis, three-second slots, slot maxima, gaps) that belong in the help.
      – **Nothing that is a warning moves**, per the standing rule: the flow-status banner, the
        egress conflict, an unobservable match-traffic group. Those are things the user must act
        on, and they stay at full length at level one.
      – What is left on the page after the audit should fit the standing rule's description
        without needing an argument: if a paragraph has to be justified, it belongs a level
        down. **A test asserts the surviving level-one text of the page**, in the same spirit as
        6.5's closed-row test, because prose grows back one sentence at a time.
      – The help page absorbs what moves, which means it gains sections rather than lines — see
        item 5, which is renaming it anyway.
      — the audit, with each block's destination:
      *To the help, behind an ⓘ*: the chart note (six sentences of drawing decisions, replaced
      by a caption saying what the axes are — `chart`); what choosing an application actually
      does and that nothing inside a process is ever read (`watching`); how the passive figures
      are taken (`passive`); what the reference pool proves and why an unproven member is not
      counted (`pool`); what a warm-up withholds and what it never withholds (`warmup`).
      *Shortened in place*: the warm-up line, the empty-application line, the supporting-group
      hint, the absent-pool line, the unproven-member count.
      *Untouched, per the standing rule*: the flow-status banner, the egress conflict, the
      unobservable match-traffic group, the route panel's "this is not the round trip to the
      server", and the one clause of the passive note that is a warning — *nothing here is a
      ping*. A warning is never demoted whatever its length, and three of these are long.
      **The route panel's note was the interesting call.** It is three lines of prose and it
      stays, because it says the figure beside it describes something other than what the
      reader thinks — which is precisely the case the standing rule keeps at level one.
      A test asserts the surviving level-one text: each moved paragraph absent from the page,
      each warning present. Not a word count — a list, in the same spirit as 6.5's closed-row
      test, because prose grows back one sentence at a time.
- [x] **5. Merge the dashboard and the service status page; rename the help page.** *Fifth: two
      structural changes to navigation, taken together because they are one edit to the same
      five tabs, and taken after the text items because merging two pages of text is cheaper
      once that text is shorter.*
      – **One page, "Network", holding what the dashboard and the status page hold today**: the
        verdict banner, the domestic and foreign baselines, and the service cards. They answer
        one question — *is it me, my country's border, or that service* — and answering it
        currently means switching tabs and holding one page in your head while reading the
        other. The verdict engine already reads the baselines; the status cards are the
        evidence a reader checks it against.
      – **The merge must not become a wall.** Baselines first with the verdict above them,
        services below grouped as they are now (platforms, infrastructure). Nothing about a
        card's or a baseline's own rules changes: `nm_core::status`'s reaction rule and
        `nm_core::health`'s window rule stay separate, because they answer different questions
        at different cadences, and that difference is stated on the page rather than smoothed
        over.
      – The nav becomes four tabs: **Network · Applications · Help · Settings**.
      – **The help page is renamed.** "What the numbers mean" is a sentence, not a tab; it
        becomes **Help**, with that sentence as the page's own subtitle, so nothing is lost and
        the tab reads like a tab. Every ⓘ's "Learn more" keeps working — the topic list is one
        list and the rename touches the label, not the anchors.
      — `features/network/NetworkPage` composes the two existing pages rather than absorbing
      them, and that is the whole design: **an arrangement, not a blend.** The two halves are
      measured on different cadences by different rules, each states its own span on the page,
      and a merged component that computed one figure over both would have been exactly the
      smoothing the item forbids. A test asserts both cadence lines survive side by side.
      The order is the argument's order — verdict, the baselines it was drawn from, the
      services to check it against. **What the core itself is doing moved to the bottom**: it
      is a fact about the application rather than about the network, and it was sitting between
      the two halves that need to be read together.
      The nav is four tabs, and `Page` is a four-variant union, so a stale `'dashboard'` is a
      compile error rather than a blank pane. The help tab is **Help** with "What the numbers
      mean" as the page's subtitle; the anchors are untouched, so every "Learn more" still
      lands where it did.
- [x] **6. The picker offers only applications the app can name.** *Sixth: a real ergonomic win
      with a real risk, which is why it sits behind the items that cannot hide anything.*
      The picker lists every running process grouped by executable name, so it reads like a
      task manager — hundreds of rows, of which a handful are things anyone would watch.
      – **Offer only candidates that resolve to a name**: a preset, a curated label, or the
        bundled catalogue generated from Discord's detectable-games index (`assets/apps/
        labels.json`, 9 314 entries — Phase 6.5 item 2). `PresetList::label_of` already answers
        exactly this question, so the filter is one predicate in `applications::candidates`.
      – **With an escape hatch, and it is not optional.** The catalogue is large but not
        complete: a title too new for it, a regional client, or anything not in Discord's index
        would become unwatchable, and "the app cannot see my game" is a worse failure than a
        long list. So the filtered list is the default and a **"show everything running"**
        toggle sits beside the search, off by default, with the count of what is hidden shown
        rather than implied.
      – **A name is not a claim.** A catalogue entry supplies a label and never a grouping —
        the rule 6.5 established stands, and the filter must not become a second grouping rule
        by the back door.
      – Search still searches the executable name, so a user who knows the file name finds it
        with the toggle off.
      — `Candidate.named` is the predicate, decided in `applications::candidates` from
      `label_of` and carried to the picker as `ApplicationChoiceView.named`. **A fact about
      the bundled lists, never a claim about the program**, and it cannot become a grouping by
      the back door: the offer's key is computed before the name is consulted, and a test
      asserts two differently-named executables stay two offers.
      The escape hatch is a checkbox beside the search, off by default, **with the hidden
      count stated rather than implied** — a user who cannot find their game has to be able to
      tell "this app cannot see it" from "the filter hid it".
      One thing the item did not ask for and the build needed: **"no application matches that
      name" must not be the answer when the filter is why.** Searching for a hidden process
      would otherwise return the flat refusal, which is exactly the conclusion the toggle
      exists to prevent; the message now points at the toggle instead, and a test pins it.
      Search matches the executable as well as the label, so knowing the file name is enough.
- [x] **7. Age in the connection's header, and the honest word for it.** *Seventh: new
      information rather than a correction, and the user asked for it in exactly those terms —
      "how long it has been established, or failing that how long we have been watching it".*
      – **TCP has a real answer and it is already arriving.** The flow-metrics spike recorded
        `ConnectionTimeMs` on event `1477`, the TCP summary the app already subscribes to for
        passive round-trip time (`docs/flow-metrics-spike.md`). It is not parsed today. Parsing
        it costs one more field on an event that is already decoded, past the local-port gate
        that already protects the data-minimisation rule.
      – **It is a stale reading and must be shown as one**, like the passive round trip beside
        it: summaries arrive at connection close and every few tens of seconds, so the header
        shows the age with the same "as of N s ago" treatment, or ages it forward from the
        event's own stamp — never mixing `FlowInstant` with `Instant`, which the type system
        already forbids.
      – **UDP has no such thing and must not borrow it.** There is no connection to have been
        established, so what the header can honestly show is **how long this app has been
        watched talking to that endpoint** — `Endpoint::first_seen`, which `nm_core` has held
        since Phase 4 and never surfaced. Two different facts, two different words, never one
        field that means whichever was available.
      – What the user actually asked for is served either way: telling a new endpoint from one
        that has been there all match. It also gives the warm-up badge a natural neighbour.
      – Level one gets one figure; which of the two facts it is, is stated in the expander.
      — `EndpointAge` is a two-variant enum in Rust rather than a duration and a flag, so
      there is no shape in which the page could receive a number without the claim attached.
      `ConnectionTimeMs` cost one `try_parse` on an event already being decoded — and it is
      **optional where the round trip is not**: the field has moved between Windows versions
      before, and losing an age must never cost the measurement beside it. Where it is absent
      the endpoint falls back to *watched*, which is always available and always true.
      **Aged forward rather than reported as of the summary.** A summary arrives every few
      tens of seconds at best, so quoting its figure unchanged would make a connection appear
      to stop ageing between events. The elapsed time added is local and exact; `FlowInstant`
      is never mixed with `Instant`, which the type system forbids anyway.
      Three tests in Rust — established, watched, and a connection the system never dated —
      and two in the page, one per word. `spanOf` is a tested pure function of its own, so a
      long session reads `8 h 12 min` rather than the `492 min` the chart axis learned about
      the hard way, and every unit stays an i18next key.
- [x] **8. More services on the status page, starting with Ubisoft.** *Eighth: pure data, no
      code, and it can land at any time — it is late only because it competes with nothing.*
      Eleven services ship today. Missing and named by the user: **Ubisoft Connect**. Others
      worth the same check: Rockstar, Wargaming, Roblox, HoYoverse, Nintendo, and on the
      infrastructure side Akamai, Fastly and Azure, which host a large share of what the
      platform cards depend on.
      – Every rule in `assets/targets/README.md` applies unchanged, and the tests enforce them:
        **names, never addresses**; the probe kind is a hint and never a permission; the whole
        list stays inside its share of the global cap, which is what bounds how many services
        can be added at all.
      – **Every host verified from this machine before it is committed**, with the probe kind
        the app will use — the check Phase 6 ran for the existing thirteen. A front door that
        does not answer is a permanently red card about a service that is up, which is the
        exact failure the bundled SDR hostnames caused in Phase 2.
      – Adding a service must stay a data change; if any candidate needs code, it does not ship
        here.
      — nine services added, all nine of the candidates named: **Ubisoft Connect** (two front
      doors — the launcher's service and the account API, which is exactly the storefront-
      versus-gateway distinction the schema asks for), Rockstar, Wargaming, Roblox, HoYoverse,
      Nintendo, Akamai, Fastly and Azure. Twenty services and twenty-three endpoints now, which
      at the shipped cadence is **0.51 probes/s** against a cap of 32 — the test that bounds
      this asserts under 1.
      **Every host was connected to from this machine before it was committed**, with the
      probe kind the app will use, over four passes. Two are worth recording rather than
      hiding: `www.fastly.com` and `api.akamai.com` each timed out on a first attempt.
      `api.akamai.com` never answered at all and was **dropped** — a front door that does not
      answer is a permanently red card about a service that is up, which is the failure the
      guessed SDR hostnames caused in Phase 2 — while `www.fastly.com` answered on every
      later pass and ships. `Azure` is `management.azure.com`, the ARM endpoint, rather than
      the marketing site, which is a CDN and would measure the CDN.
      Data only: no code changed except a stale comment naming the old endpoint count.
- [x] **9. Sweep for the words item 1 replaced.** *Last, and it is bookkeeping rather than a
      feature.* Once the vocabulary changes, the old names survive in the help text, the
      tooltips, the tray labels, the chart legend and the Russian locale that Phase 7 is about
      to create. One pass across `locales/en/common.json` and the help topics, so Phase 7's
      translator never has to translate a word the product no longer uses.
      — the retired words are gone: *Response*, *Steadiness*, *Lost replies* and *Evenness*
      appear nowhere, including as ordinary prose, where "steadiness" survived in one help
      body and would have read as a reference to a label that no longer exists.
      **The sweep turned up more than words, and the acceptance criterion is why.** It asks
      that a reader can say what every figure *on the merged Network page* measures — and the
      baselines were still `RTT` / `RTT (median)`, with no ⓘ at all, sitting directly above
      service cards that had both. So they now read **Ping (RTT)**, **Ping, median** and
      **Jitter, median**, and every one of them carries its explanation. That is the last
      place in the product where two names existed for one quantity.
      `serviceRtt` became `medianRtt` and its text was generalised with it: the same topic now
      serves a service's front doors and a baseline's members, which are the same rule about
      the same kind of roll-up, and a topic that spoke only of services would have been wrong
      the moment the dashboard pointed at it.
- **Accept**: a reader who has never used the app can say what every figure on the Applications
  page and on the merged Network page measures, using the words the rest of the networking
  world uses, without leaving level one to find out; a match server shows a route figure and
  flow figures and **no dashes where a ping would be**; the status page states what one cell is
  and how far back the strip reaches; the picker opens on a list a person can read, with the
  hidden count visible and one click to see everything; a connection header says how old it is
  and which kind of "old" that is; Ubisoft Connect appears on the status page and answered a
  real probe from this machine before it was committed. Tests: the surviving level-one text of
  the Applications page, the never-and-not-yet distinction of item 2, and that the word "ping"
  appears on no route figure.

### Amendment from use (2026-08-02, after running the build): the page carries no explanations

Stated by the user on seeing the running build against a live Apex Legends session, pointing
at the sentence item 2 had just added to the match server's row — *"Nothing this app can send
is answered here, so there is no ping, jitter or loss to show for it."* — and generalising it
into a doctrine: **the everyday UI is clean; whoever wants detail finds it in a tooltip or in
the help.**

- [x] The sentence is gone, and with it the two remaining explanatory paragraphs of the same
      kind: the route panel's *"This endpoint answers nothing we can send…"* and the flow
      panel's *"Nothing here is a ping…"*. What replaces them is nothing at all on the page.
      **The claim each one made now lives in the name of the figure it was about**, which is
      the part of the change worth recording rather than the deletions. The route panel is
      honest because its figure is called *round trip to that router* and its heading *the
      route towards it*; the flow panel is honest because **not one of its four labels is a
      round trip**. A name cannot be skipped by a reader in a hurry; a paragraph underneath
      can, and the reader who skips it is exactly the one it was written for.
      The ⓘ moved onto both panel headings, since what it explains is the panel rather than
      any one figure in it, and the route figure's own ⓘ went with it — two disclosures
      carrying the same words are a second thing to read, not a second thing to learn. The
      route panel's jitter and loss gained the ⓘ they never had.
      **`CLAUDE.md` is amended in the same commit**, as item 1's reversal was: the doctrine is
      written above the three levels because it governs them, and the never-demote rule is
      sharpened with the test it was missing — *a warning is something to act on*. "This
      figure is measured differently from what you expect" is an explanation, not a warning,
      which is precisely how three paragraphs had been justified at level one.
      The tests changed shape with the rule rather than being deleted: PathPanel now asserts
      that the claim is in the labels **and that no prose survives**, and FlowPanel asserts
      that the words "round trip" appear nowhere in it — a stronger guarantee than the
      sentence it replaces, because it fails if any future figure there is named one.

**Phase 6.6 status: all nine items are built and tested, and the build has now been run.**

The three tests the criterion names all exist, and so do the ones the items asked for on their
own account: the never-and-not-yet distinction in Rust and in the page, the route panel's whole
text never matching "ping", the surviving level-one prose of the Applications page, the strip's
legend and its keyboard readout, the picker's hidden count and its refusal to answer "no match"
when the filter is why, both words for a connection's age, and every figure on the Network page
having an explanation reachable without a mouse.

**What the live run settled**, on 2026-08-02 against Apex Legends mid-session — the first time
anything from this phase was seen rather than asserted:

- The chart drew nine minutes of eight endpoints on the log axis, and the match server's route
  was the dashed line among them, exactly as the tests describe it.
- The match server (`UDP`, carrying traffic) showed **no ping, jitter or loss at all** — item 2
  working, and the amendment above is what the user said on seeing it.
- Every renamed figure read correctly: *Ping (RTT) · Jitter · Loss* on the supporting
  connections, *Arrival jitter* in the flow panel, *Round trip to that router* in the route
  panel, *Age 3 min* on each header — items 1, 3 and 7 in one screen.
- The other UDP endpoint was mid-chain — dashes for ping and jitter beside a measured 100 %
  loss — which is the *not yet* half of item 2 appearing on its own, unprompted.

It also confirmed two rules of 6.5 that had never been watched happening: a newly discovered
endpoint carried *Warming up · 22 s* with a round trip shown and its jitter and loss withheld
— item 5's exact rule, arriving on its own — and the endpoints kept their places while the
figures under them changed.

**One thing worth a look before Phase 7, and one that looked wrong and was not:**

- **The verdict may be over-firing.** *"The network is fine and this application's own
  endpoints are not — the route it takes"* on endpoints sitting at 190–234 ms, which for
  transatlantic AWS is ordinary rather than degraded. Worth checking whether the degraded
  threshold is absolute where it should be relative to what the rest of the group sees.
- **The reference pool was not broken.** It first read "Answering —" beside "Ping, median 122",
  which looked like a gap; ten minutes later it read "Answering 100" and *OK*. The share
  answering needs a minimum before it will report and the median does not, so the two are
  absent and present at different moments — which is "absent knowledge stays absent" working,
  not a bug. Recorded because the first reading was flagged as a suspect and was wrong.

Still **not** verified: the merged Network page and the picker were not opened — driving the
window would have taken focus from a running game — so items 5 and 6 have been seen only in
tests. That, and the four remaining titles, is the Phase 7 pass.

### Amendment from use (2026-08-03): a tunnel that takes traffic by routing, not by DNS

Found by the user running the build with a local TUN client active and reading the status
page: **twenty services, worldwide, all between 1.0 and 2.5 ms, all green.** The number is
impossible and the page was stating it with confidence — the exact failure this product exists
to prevent, and worse than a fake-bad reading because it says the user's network is fine.

The spike (`docs/measurement-reality-check.md`) had already met this behaviour and had drawn
the boundary in the wrong place. It found a *router* running sing-box with fake-IP, so it
concluded the tell was the **address**: a synthetic address out of `198.18.0.0/15`, detected by
`AddressPolicy` and routed to a TLS probe. A TUN client on the machine itself takes traffic the
other way — by installing a fan of routes (`0.0.0.0/5`, `8.0.0.0/7`, … `240.0.0.0/5`) that
covers the whole public internet at a better metric while the real default route stays where it
was. Every name resolves to the real public address of the real service. Nothing about the
address is unusual, and everything about the route is. The lie is identical: the tunnel's own
stack completes the handshake and answers the echo without a packet leaving the machine.

Measured on the development machine before writing anything, because the design rests on it:

| | through the tunnel | bound to the physical adapter |
|---|---|---|
| TCP connect to a public resolver | 0.6–1.1 ms | 4.2–4.6 ms |
| ICMP echo to the same | <1 ms, **TTL 128** | 3–14 ms, TTL 57 |

The TTL is the part worth keeping. 128 is Windows' *initial* hop limit, so the reply crossed no
router; from a public address that is not a suspicious reading but an impossible one. It is
proof rather than inference, it is free — the field is already in `ICMP_ECHO_REPLY` — and it
needs no route table, which is what makes it the backstop on platforms that have none.

- [x] **`nm-core`: an endpoint is its address *and* the start of its route.**
      `AddressClass::TunnelledEgress`, reached by `through(Egress)`. Held apart from
      `TunnelSentinel` rather than folded into it: measured identically, but a sentinel means
      the *name* was answered with a stand-in and this means the name was answered truthfully
      and the *packets* are being taken. A real machine has both at once.
      `Egress::Unknown` deliberately decides nothing — read as a tunnel it would refuse the
      cheap kinds on every platform without a route backend, which today is two of three.
      `nm_core::forgery` is the rule above, pure and exhaustively tested, including the case it
      knowingly gets wrong: a machine genuinely sharing a segment with a public address. That
      is accepted because of *which way* it fails — a false positive costs a more expensive
      probe, a false negative costs the whole product.
- [x] **`nm-platform`: a route lookup.** `GetBestRoute2` for the adapter, `GetIfEntry2` for what
      it is. The deciding rule takes plain numbers and lives outside the Windows module so it is
      tested anywhere; a Windows-only test asserts the constants against the headers so they
      cannot rot. An adapter carrying **bare IP rather than frames** is a tunnel — that is every
      TUN-shaped client of this generation, and it separated cleanly on a machine that also has
      a Hyper-V switch, a bridge and an idle TAP adapter. Measured at **80 µs**, and it runs once
      per target registration, not per probe. Linux reads the same from netlink, macOS from the
      route socket. A layer-2 TAP adapter is *not* caught here and is not chased with a list of
      driver names that would rot; the hop-limit proof covers it instead.
- [x] **`nm-probes`: the refusal, and the correction.** Either finding reclassifies the
      **endpoint**, never demotes one probe kind — stepping to the next kind would swap ICMP's
      invented number for TCP-connect's, since the same tunnel answers both. A claim of
      filtering does not survive it: a kind that went silent behind a tunnel was never offered
      to the path. `reconsider` asks the route again rather than trusting registration, which is
      the only thing that would ever notice a VPN switched off.
- [x] **The page.** `ProbeOutcome::AnsweredLocally` is neither loss nor silence and stays out of
      every delivery ratio. Its check mark had been falling through a catch-all to *no answer* —
      reporting a dropped packet where none was sent — and *probe filtered* would have been the
      other wrong answer, since filtering happens **on** the path and this happens before it.
      "Through a tunnel" is promoted to level one on the Applications page and is the one badge
      carrying an ⓘ: it is not a caveat on a figure but the reason every figure on the row was
      measured differently. Its help topic states what the app does, and that **nothing is ever
      sent outside the tunnel to compare** — which would show a provider what the tunnel exists
      to hide. *That was the user's decision, taken when the scope was set.*

**What the live run settled (2026-08-03).** Every baseline and all twenty services now carry
*TLS* and *Through a tunnel*, and the figures are real: a spread of 53–227 ms that differs per
service, with two platforms and one cloud sitting far above the rest while another cloud sits at
the bottom. One domestic baseline reads *Degraded* on a genuine figure. A domestic target with
no working end-to-end probe shows *Cannot be measured* and dashes rather than zeros — the
tunnelled `ChainStep::Nothing` path, seen for the first time.

Not one cell of any strip is grey, and that is the result rather than a gap: the route lookup
caught the tunnel at registration, so the lying kinds were never tried and the hop-limit proof
never had to fire. The cheap structural check did the work; the expensive proof stayed in
reserve for the platforms and adapters it exists for.

## Endpoint enrichment — name the destination

Stated by the user on 2026-07-31 alongside the two Phase 4 amendments and placed by them
explicitly last of the three; brought forward and built on 2026-08-03 on their instruction.
It sat under Phase 8+ until then, which is where its options were argued out.

An address means nothing to a user. "Amazon", "Cloudflare", the name of their own provider
means something, and it is what turns the route panel and Phase 6's verdicts from a column of
numbers into a sentence about where the traffic goes and where it stops going.

- [x] **A bundled offline IP→ASN table.** Chosen by the user over the cheaper option of
      bundling the published cloud and CDN prefix lists, because those name only the five or
      six providers that publish them — never a transit network and never an ISP, which is
      exactly what the route panel needs named.
      **The licence was verified before anything was bundled, as this plan required.**
      iptoasn's combined table is Open Data Commons PDDL v1.0: public domain, no attribution,
      no obligation of any kind. DB-IP's Lite database was rejected — CC BY 4.0, requiring a
      visible link back to db-ip.com on any page showing its results — and MaxMind's GeoLite2
      for requiring an account and acceptance of an EULA, which is a poor thing to ask of this
      audience before its download is even considered.
- [x] **Bundled, not downloaded — and that was reconsidered rather than inherited.** The user
      proposed fetching the database at install time with a forced-update button in Settings,
      which would have meant a smaller installer and a fresher table. Checking it killed it:
      every ready-made copy of this data — iptoasn, DB-IP, GitHub raw, jsDelivr — is served
      from behind Cloudflare or Fastly, and Russian ISPs have throttled Cloudflare-backed
      responses to roughly the first 16 KB of any asset since June 2025, which breaks a
      multi-megabyte download outright. The only sources on their own infrastructure are RIPE
      NCC and RouteViews, and neither publishes a lookup table — they publish raw BGP dumps,
      an order of magnitude larger and needing an MRT parser. There is no host reliably
      reachable from the countries this product is for, and one reachable today may not be
      next year. **The user reached this conclusion themselves and stated it: bundle is the
      only durable option.** A manual "import from file" fallback was proposed and rejected by
      them as a bad answer for this audience, correctly — it is a workaround, not a design.
- [x] **The bundle is 5.11 MB, not 8.46.** Upstream repeats each AS description on every row
      it appears in; splitting the descriptions into a directory of their own removes the
      repetition and leaves both halves plain tab-separated text that can be diffed against
      upstream. `assets/asn/README.md` records the source, the licence, the retrieval date,
      the upstream checksum and the one-line command that regenerates both files.
- [x] **The lookup is pure and lives in `nm-core`**: sorted arrays binary-searched per address
      family, names in a shared arena — 12 bytes per IPv4 block, 36 per IPv6, ~12 MB resident
      for 573 125 announced blocks and 86 628 autonomous systems. `nm-app` streams the two
      gzipped assets through it a line at a time, on the blocking pool, so neither the 17 MB of
      decompressed text nor a stalled runtime is ever paid for.
- [x] **It is a setting, because it costs memory rather than privacy.** ~12 MB is a quarter of
      the core's budget and the only part of this application a user might reasonably trade
      away on a machine where the game needs every megabyte. On by default; switching it off
      releases the table at once rather than at the next restart. Nothing is looked up over the
      internet, so there is no privacy cost to weigh against it.
- [x] **Runtime lookup — RDAP, whois, Team Cymru's DNS interface — stays rejected rather than
      deferred.** It tells a third party which servers this user is playing on, from a machine
      under surveillance, and that is the phone-home the product promises never to make.
- [x] **The registration country is not a location, and the page never lets it become one.**
      It is worded as *registered in*, it lives at level two, and both the module documentation
      and the help topic state that anycast and cloud regions routinely put it thousands of
      kilometres from the machine that answered — the measured round trip is the better
      evidence of distance, never this. GeoIP city-level data was not adopted at all.
- [x] **Two levels, and the ⓘ.** Level one is the name beside the address, and the name beside
      where the route stops — a label, not a figure, so it does not count against the three.
      Level two carries the AS number and the registration country. The `network` help topic
      carries the explanation, including that the directory is a photograph of one day; the
      snapshot date is stated once in Settings, beside the switch, rather than on every row.
- **Accept**: an endpoint and a route hop are named on the page; a name the directory does not
  have is absent rather than guessed; the licence and the bundling decision are written down
  where the next person will look for them. *Met.*

## Phase 6.7 — The shape of the page (design review, 2026-08-03)

From `docs/design-review-2026-08-03.md`, written after the user read the running Phase 6.6
build and reported that it "looks like a proof of concept, not a product". The review's own
diagnosis: the product is not unfinished, it is **unassembled** — every screen was designed,
and no two screens share a unit of measure.

It runs before Phase 7 for the same reason 6.6 did: Phase 7's five-title verification pass
should be run against the page as it will ship, not against one about to be rebuilt.

**Two rules in `CLAUDE.md` are amended by this phase, both approved by the user on
2026-08-03**, and both are already written into `CLAUDE.md`:

- *Level three* no longer means "an ⓘ on every figure" — the label itself carries the
  explanation, once per surface rather than once per row. At twenty connections the old rule
  produced up to 260 identical marks on one page.
- *Level one* no longer means "at most three figures" flat — it is three figures **per
  subject**, and a second subject is allowed only in a table whose column headings name it.

**Nothing here changes what is measured.** No figure is deleted; figures that leave level one
move down a level, and each item below names the level they land on.

### P0 — defects (no decision needed, independent of everything else)

- [x] **1. Units on every figure.** `formatMs` and `formatPct` return a bare number, so the
      page reads `Loss 3` — three per cent or three packets? Units become i18next keys
      (`unit.ms`, `unit.pct`, `unit.perSecond`); the em-dash never takes a unit.
- [x] **2. `nm-health--bad` does not exist.** `FlowPanel` styles the freeze badge with a class
      no stylesheet defines, so the strongest "something is wrong right now" signal in the
      product renders as a neutral pill. It takes `--nm-unreachable`.
- [x] **3. Help bodies lose their paragraph breaks.** `\n\n` inside a single `<p>` collapses;
      the `network` topic ships as one ~350-word block. Bodies split into paragraphs.
- [x] **4. The TCP group opens and closes under the reader.** `open={group.needsAttention}` is
      re-applied on every flip of a value that flips constantly on a weak link. It becomes the
      *initial* state only; a problem inside a folded group is announced by a badge in the
      heading instead.
- [x] **5. Duplicate CSS rules** — `.nm-visually-hidden` (two different implementations) and
      `.nm-endpoint__age`, each declared twice.
- [x] **6. Visible focus everywhere.** `:focus-visible` exists on two elements in the whole
      stylesheet, in a codebase built around keyboard reach.
- [x] **7. The explanation panel is clipped at the window's right edge** — hard `left: 0` with
      no flip.
- [x] **8. The chart's own units and caption.** No `ms` on the vertical axis, and a caption
      reading "Ping over time" above a chart that also draws dashed *routes*, which are not
      pings. The caption drops the word.

### P1 — the system, and the structure it holds up

- [x] **9. Design tokens.** The stylesheet carries 16 spacing values, 15 font sizes, 10 radii
      and **two incompatible palettes** (Tailwind for surfaces, GitHub Primer for states, plus
      a third amber duplicating a second one). One spacing scale, one type scale, three radii,
      one palette. This is the single change that removes most of the "homemade" reading.
- [x] **10. Vertical rhythm by `gap`, never by `margin-top`.** `.nm-appcard` is a plain block
      whose children each choose their own top margin, so the distances between them are
      arbitrary and collapse unpredictably. Cards become flex columns with one gap, and no
      `margin-top` survives inside one.
- [x] **11. One breakpoint system.** Four different `minmax()` bases on one page mean columns
      reflow at four different window widths while resizing.
- [x] **12. `Match traffic` / `Supporting connections` → `UDP flows` / `TCP connections`.**
      *Approved by the user on 2026-08-03, variant A.* The old names claim a **role** from a
      **transport**, which `view.rs` itself refuses to do in the same breath — and they are
      simply wrong outside a game: Discord's UDP is voice, a browser's is QUIC, and several
      games play over TCP. The two group hints leave the page for two new help topics
      (`udpFlows`, `tcpConnections`), which is where an explanation belongs.
- [x] **13. The help page becomes an index.** 26 topics in a flat list, each opening with a
      150–350-word body, no contents, no grouping, no search, no way back. Default becomes
      title plus the one-sentence `short` that is already written; the body sits behind
      "Read more". Five named sections with a contents column, a filter, and a Back that
      returns the reader to the page they came from.
- [x] **14. Verdicts get help topics.** The one place in the app that states a conclusion has
      no explanation of any kind. `verdict` and `verdictEvidence`, plus the disclosure on the
      banner's own heading.
- [x] **15. `WhyNotYourPing` once per card**, not once per silent endpoint. For a game that is
      the same three-point disclosure repeated on every row.
- [x] **16. The reference-pool panel appears only when there is a pool.** A bordered, headed
      panel whose whole content is "None for this application." is the ordinary case for most
      titles; it becomes one muted line.
- [x] **17. The picker folds away, and there is a first-run state.** A setup tool with its own
      search, checkbox and 16 rem scrolling list is mounted permanently above the monitoring
      surface. Folded it is one line naming what is being watched; the empty state gets a
      heading, a sentence and a primary action instead of one grey line.

### P2 — the chart becomes a reading surface (no decision needed)

Placed above the table rework deliberately: it is independent, it is visible immediately, and
it waits on none of the contested decisions.

- [x] **18. A tooltip that is also the legend.** Today the chart has no legend, no cursor
      point and no tooltip, and the *only* effect of hovering a line happens somewhere else —
      a row that may be off screen, which is exactly what the user reported. The tooltip lists
      **every** line at the pointed moment (the chart's stated job is "which of these is the
      odd one out", which is a question about all of them), each entry naming its quantity as
      `ping` or `route` — the never-merge rule applied to the tooltip — and a line with no
      value in that slot reads `no reply`, never `0` and never a silently missing entry. It
      carries no words about health: the list remains the only authority on state. Plus a
      crosshair, and the same edge-flip as item 7.
- [x] **19. Selecting from the tooltip pins the endpoint and scrolls its row into view.** On
      click only — scrolling on hover is nauseating — which is the direct fix for "it
      highlights something I cannot see".
- [x] **20. Keyboard parity with `CheckTimeline`.** The chart is `role="img"` with a static
      label, so it is inert to a keyboard, while the far less important check strip on the
      services page already does this properly: one tab stop, arrows inside, a live text
      readout. The chart gets the same — `←`/`→` through time, `↑`/`↓` between lines, Enter to
      keep, Escape to release, and an `aria-live` readout mirroring the tooltip.

### P3 — explanations move onto the labels

*Implements the amended level-three rule. Approved 2026-08-03.*

- [x] **21. `MetricHelp` wraps the label instead of standing beside it.** Same behaviour —
      hover *and* focus, "Learn more", Escape — with no glyph and no second target. A dotted
      underline marks a label as explainable; one sentence in the empty state and in the help
      introduction says so, once, rather than two hundred times.
- [x] **22. One explanation per surface.** Column headings and panel headings carry it; cells
      and figures carry nothing. Applications with twenty endpoints goes from 130–260
      disclosures to 9; Network with twenty-three services from ~200 to 6.
      A test asserts the count **does not grow with the number of rows**.

### P4 — the endpoint list becomes a table

*Implements the amended level-one rule. Approved 2026-08-03.*

- [x] **23. `AppView.primaryEndpoint`, and the card leads with it.** The busiest flow by
      recent bytes in the window — a *measured* fact, which is the one thing `view.rs` already
      allows beside the transport, and therefore not the role guess it refuses. Rust decides
      it, as every judgement here does. Where no endpoint carries materially more than another
      there is **no primary**, stated as such, and the card leads with the table alone.
- [x] **24. `EndpointRow` becomes a table row.** One card at six endpoints is ~1200 px today
      and ~480 px after. The route's jitter and loss, the route's position and the four flow
      figures move to **level two** for every endpoint except the primary; the freeze warning
      moves nowhere and stays at level one, as warnings do. The blank `Ping` column beside a
      filled `Route` column is what makes the never-merge rule visible on every row at once —
      the reason the amended rule was worth asking for.
- [x] **25. The order freezes while the list is being read.** `holdPlace` holds one pinned row;
      everything else that reorders a list mid-read — a discovery, a forgotten endpoint, a
      health change elsewhere — still moves it. Rust's order is buffered while the pointer is
      over the list or focus is inside it, and applied a couple of seconds after it leaves.
      View state, like `holdPlace` itself.
- [x] **26. A second channel for endpoint identity besides colour.** The swatch tying a row to
      its line is the one place in the product where colour is the sole carrier of meaning.
      Shape, or a dashed swatch for a route.

### P5 — the Network page becomes one page

- [x] **27. Merge the two halves of Network into a single list.** *Instructed by the user on
      2026-08-03, and it reverses Phase 6.6 item 5's "an arrangement, not a blend".* That item
      put the baselines and the service cards on one page and deliberately left them as two
      compositions. The user's reading of the result: we measure the same thing and draw it
      twice. That is correct, and the duplication is larger than it looks:
      – **The same address is measured twice and drawn twice.** Two of the four foreign
        baseline targets — `discord.com` and `api.steampowered.com` — are also service
        endpoints of Discord and Steam. Half of one baseline is a second probe of a row already
        on the page, in another visual language, under another name. It also spends probe
        budget twice for one fact.
      – **Two card designs and two row designs** for one object: `GroupCard`+`TargetRow` beside
        `ServiceCard`+`EndpointRow`, with two distribution renderings that share their CSS.
      – **Two histories.** `Sparkline` draws RTT over time and **strokes it in a colour that
        states health**, which is the one rule about colour this product does not break
        anywhere else; `CheckTimeline` draws one cell per check and names six distinguishable
        outcomes in words. For a page whose question is "which of these is red", the strip is
        strictly the better instrument, and it is the one that survives. *This is the only
        thing on the page that goes away rather than moving down a level, and it goes because
        the element replacing it carries more, not less.* Continuous RTT-over-time reading
        belongs to the applications chart, which is where it already is.
      – **Five names for one round trip on one page**: `Ping (RTT)`, `Ping, median` (baseline
        group), `Ping, median` again (service card), `Ping, last check`, `Ping, mean`. Two
        survive: **`Ping, median`** on a group heading and **`Ping (RTT)`** on a row, over the
        window that row states. Last-check-versus-mean is a *which window* qualifier and moves
        to level two; both help topics (`latestCheck`, `meanRtt`) stay, because the distinction
        is real and worth explaining where it is made.
      – **Two IPC shapes** for the same data (`GroupView`/`TargetView` and
        `ServiceView`/`ServiceEndpointView`) converge on one.
      **What a baseline actually is: a tag, not a list.** The merge's real content is that
      "domestic baseline" and "foreign baseline" are *roles a target plays*, not separate
      inventories — which is why two of them are literally service endpoints copied into
      another file. Targets become one schema carrying tags, `foreign.json`'s duplicates stop
      being separate entries, and the tag moves onto the service entries instead.
      **This must not thin the verdict's evidence, and naïvely deleting the duplicates would.**
      Strip Discord and Steam out and the foreign baseline is two anycast DNS resolvers, which
      are famously reachable almost everywhere — a thin and biased sample for the one verdict
      that decides whether to suggest a VPN. So they stay in the evidence *as tagged services,
      probed once*, and the foreign tag is spread across enough members that the sample is not
      two DNS services in a trenchcoat.
      **The measurement layer does not merge, and the page says so.** `nm_core::health`'s
      window and `nm_core::status`'s reaction rule answer different questions and stay separate
      — a component computing one figure across both would be exactly the smoothing 6.6
      forbade. What merges is the **view** and the **target schema**. The cadence difference
      stops being two subsystems and becomes a **per-target interval field**, which is what
      `nm-core`'s scheduler has ordered by since Phase 1 ("scheduling priority is interval
      length, not a separate rank"); the reason the services are slow is the probe budget, and
      that is a number, not an architecture.
      **The page after the merge** is the verdict, then one list under four section headings —
      Domestic, Foreign, Gaming platforms, Infrastructure — every row drawn by one component,
      one legend for the whole page, one caveat, and each section stating its own cadence at
      level two. The two baseline sections carry a marker saying they are what the verdict was
      drawn from, so the banner above stays checkable against the rows below.
- [x] **28. Rows fold to one line** — name, state, strip, `Ping (RTT)` — with endpoints, badges
      and the remaining figures at level two. Rows worse than `Ok` start open. *Approved
      2026-08-03.* Applies to the whole merged list, not to services alone; twenty-three cards
      each carrying a strip and three figures per endpoint turns "which of these is red" into a
      scrolling task.
- [x] **29. A count chip must not look like a state badge.** Both use `.nm-health` today, so
      "this service is degraded" and "two of its endpoints are degraded" are visually the
      same claim.
- [x] **30. One `MetricRow` for all pages.** The same three quantities are laid out by a rigid
      3-column grid on one page and by wrapping flex on another, so they do not line up. The
      merge settles this for Network; the item remains because Applications shares it.

### P6 — the shell

- [x] **31. Header on a grid; `Quit` moves to Settings; the tagline leaves the header.**
      *Approved 2026-08-03.* `Quit` currently sits next to `Minimize`, distinguished by
      muted text — two adjacent buttons where one hides the window and the other ends the
      monitoring. Settings is always reachable, so the guarantee that a user is never stuck
      survives the move.

- **Accept**: the closed views assert what they must not show (the review's §11 lists every
  one); an application card with three endpoints and one with twenty carry the **same** number
  of explanations; the tooltip names every line's quantity and never says "ping" of a route;
  the chart is fully operable from the keyboard; no `margin-top` remains inside a card; one
  spacing scale, one type scale and one palette are the only ones in the stylesheet; and the
  Network page draws every target with one component, names the round trip two ways rather than
  five, and **measures no address twice** — a test walks the bundled target lists and fails on
  a duplicate.

**Phase 6.7 status: every item is done and every quality gate is green.**

What item 27 actually cost, recorded because the shape of it was not obvious from the
review: it is the only item in the phase that was not a change to the frontend.

- **One inventory replaced two schemas.** `baselines.rs` and `services.rs` became
  `targets.rs`, schema v2, where a target names the [`Section`] it is listed under and the
  file may state one for all of its entries. "Domestic" and "foreign" stopped being separate
  inventories and became two of the four sections — which is what they always were, and
  exactly why two of them had turned out to be copies of service entries.
- **No address is measured twice, and a test walks the bundled lists to keep it that way.**
  `discord.com` and `api.steampowered.com` were in `foreign.json` *and* in `services.json`.
  A hard error rather than a deduplication: which of two entries the reader is meant to see
  is a question only a person can answer.
- **The foreign sample is not two DNS resolvers in a trenchcoat.** Deleting the duplicates
  naïvely would have collapsed the foreign evidence to two anycast resolvers, which are
  reachable almost everywhere — a thin and biased sample for the one verdict that decides
  whether to suggest a VPN. Steam, Discord, Epic and Riot moved into the foreign section
  wholesale, probed once, and a test asserts the sample stays wider than its resolvers.
- **One monitor replaced two.** `monitor.rs` and `status.rs` became `network.rs`. They had
  been the same shape all along — an entry per endpoint, a row per named thing, a history
  each — over two target schemas.
- **The measurement layer did *not* merge, and that is now stated in one place.**
  `Section::judged_by_window` decides whether a section is judged by `nm_core::health`'s
  window or `nm_core::status`'s reaction rule. A figure computed across both would be the
  smoothing 6.6 forbade. The cadence difference stopped being two subsystems and became
  `targets::interval_for` — a number, as the review said.
- **`Sparkline` is the one thing that went rather than moved down a level**, and it went
  because the element replacing it carries more: it stroked round-trip time in a colour that
  *stated health*, the one rule about colour this product keeps everywhere else, while
  `CheckTimeline` names six distinguishable outcomes in words.
- **Item 28 landed with it**, and it had to: a service card's headline is a median across its
  endpoints, and folding the cards before the merge would have put `Ping (RTT)` — a single
  round trip — on it. After the merge each row's endpoints are level two and the name is true.

Not verified, and not verifiable here: how any of it looks. The chart draws nothing under
jsdom and no headless renderer can answer that question — Phase 7's five-title pass is where
the whole page is read on a running build.

## Phase 7 — Polish, persistence, packaging

- [ ] Local history persistence (bounded, e.g. rolling 24 h; SQLite or compact custom format) + history view
- [ ] Russian locale (`locales/ru/*`) — validates i18n discipline end-to-end
- [ ] Windows installer (NSIS/MSI via Tauri bundler), code-signing hook point, portable build
- [ ] Perf pass: measure real CPU/RAM against budget, profile hot paths, document results in README
- [ ] Error UX: offline mode, no-targets state, ETW-unavailable banner
- [ ] **A verification pass across five titles**: CS2, Dota 2, World of Tanks, Forza Horizon,
      Deadlock — one live session each. *Moved here from Phase 6.5 item 7 by the user on
      2026-08-02, to be run last.* The reason it belongs at the end is that it is the only
      item in the plan that cannot be run by anyone but the person whose machine the games
      are installed on, and every correction it produces lands on code that should by then
      have stopped moving.
      They are not five instances of the same check: Valve's SDR backs three of them (CS2,
      Dota 2, Deadlock), which is the cross-check Phase 6's reference pools were built for;
      World of Tanks runs on Wargaming's own infrastructure; and Forza Horizon is a Microsoft
      title with its own launcher and peer traffic, which is the case none of the others
      cover. All five are installed on the development machine, which is what makes this
      runnable at all.
      Record per title: whether a preset named it, which processes it grouped and whether that
      grouping was right, whether a UDP match server appeared and whether it was the busiest
      endpoint, whether the path edge engaged and how far it reached, what the flow figures
      said, what the reference pool had, and the CPU/RAM cost. Report in `docs/` (Russian), and
      the corrections it produces come back into this plan — the same loop the two spikes ran.
      It also settles what Phase 6.5's own acceptance criterion could not: whether the chart
      can be pointed at without chasing it, and whether a row being read stays still.
      This does **not** replace the acceptance criteria still open from Phases 4 and 5: Discord
      and a game at once, a single endpoint blocked by a firewall rule, five applications
      against the probe budget.
- **Accept**: signed-ready installer; budgets verified and documented; switching to ru requires zero code changes; the five titles are recorded in `docs/` with the corrections they force.

## Phase 8+ — Later (do not start without explicit go-ahead)

- Linux support (`sock_diag` netlink + `/proc`, unprivileged-ICMP handling)
- macOS support (`libproc`; note: Tauri e2e tooling unavailable on macOS — rely on unit/headless layers)
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
