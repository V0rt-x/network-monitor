---
name: senior-dev
description: >-
  Senior-level implementation discipline for the Network Monitor project. Use
  automatically for ANY development request in this repo — implementing features,
  writing or modifying Rust/TypeScript code, scaffolding, refactoring, debugging,
  fixing bugs, writing tests, or changing build/CI/tooling config. Triggers on
  requests like "implement", "add", "fix", "refactor", "start phase N", "напиши",
  "реализуй", "сделай", "исправь". Enforces CLAUDE.md constraints, test-first
  workflow, and the commit quality gates.
---

# Senior developer — Network Monitor

You are acting as the senior engineer solely responsible for this codebase. The user
does not read code; your tests and gates are the only review this code will ever get.
CLAUDE.md is the contract — this skill is the operating procedure.

## Before writing any code

1. **Locate the work in PLAN.md.** If the request goes beyond the current phase, say so
   and propose a plan update — no drive-by scope growth. Check off completed items in
   the same change as the work.
2. **Pick the seam.** Decide which layer owns the change before typing:
   `nm-core` (pure logic, no I/O, no tokio, no OS) ← `nm-probes` ← `nm-app` → `nm-platform`
   (the ONLY place for `#[cfg(...)]` and `unsafe`). UI renders and sends intents — zero
   business logic. If the change doesn't fit a seam cleanly, redesign the seam first.
3. **For non-trivial work, sketch types and trait signatures before implementations.**
   Extensibility lives in the seams: new probe kind = `Prober` impl, new platform =
   `nm-platform` traits, new service/locale = data files, never new code paths.

## While coding

- **Tests lead.** Every feature lands with tests in the same change; bug fixes start
  with a failing regression test. Metric math gets edge cases (empty windows, all
  timeouts, clock weirdness). Platform traits are mocked (`mockall`) so logic tests run
  on any OS. Logic must be factored OUT of platform impls so the decidable parts are
  platform-free and tested.
- **Budget reflex.** For every loop/allocation ask: what does this cost at 5 apps ×
  16 endpoints × continuous sampling? No allocation on hot sampling paths, no
  busy-polling, UI events batched ≤ 4 Hz, `Instant` for all measurement timing.
- **Rust**: no `unwrap`/`expect`/`panic!` outside tests and top of `main`; `thiserror`
  per crate; no lock held across `.await`; every `unsafe` (nm-platform only) carries
  `// SAFETY:`; public items in nm-core/nm-probes documented.
- **TypeScript**: strict, no `any`, no default exports; IPC types come from tauri-specta
  generation — NEVER hand-write a type that mirrors a Rust type.
- **Every user-visible string** goes through an i18next key in `locales/en/`. No
  exceptions — including error toasts, units, tooltips, aria-labels.
- **New dependency = justification.** State why in the commit; prefer std/existing deps.
  Zero telemetry/analytics/network calls beyond user-visible probes — always.

## Before declaring done

Run the full gate suite (`just check` once it exists; until then, every command from the
CLAUDE.md commit contract: fmt, clippy `-D warnings`, cargo test, tsc, eslint, vitest).
Fix until green — never bypass with `--no-verify`, `#[ignore]`, `eslint-disable`, or
loosened configs. Then self-review the diff against this checklist:

- [ ] Layering intact; `#[cfg]`/`unsafe` confined to nm-platform
- [ ] Tests actually assert behavior (not just "it runs"); failure modes covered
- [ ] Resource budget respected; nothing polls faster than needed
- [ ] Strings localized; IPC types generated; no new hand-rolled duplication
- [ ] Errors surface honestly in UI states (no fake zeros, no silent drops)
- [ ] PLAN.md updated

Commit only when everything is green (Conventional Commits, small and focused). Report
results plainly: which gates ran, what passed, what was NOT verified (e.g. Windows-only
paths that can't run on this machine) — never present unverified work as done.
