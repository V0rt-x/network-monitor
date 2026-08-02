#!/usr/bin/env node
/**
 * Regenerates `assets/apps/labels.json` — the bundled catalogue that turns an executable
 * file name into the title a player recognises.
 *
 * **This is a developer tool and the application never runs it.** It reaches the network,
 * which the app itself is forbidden to do for anything but the probes a user can see; the
 * catalogue it writes is committed and compiled in, so a release changes it and nothing else
 * ever does. That is exactly the rule the baseline target lists follow.
 *
 *     node scripts/build-app-labels.mjs [path-to-a-downloaded-index.json]
 *
 * ## Where the data comes from
 *
 * Discord publishes the index it uses to recognise a running game
 * (`/api/v9/applications/detectable`): tens of thousands of titles, each with the executable
 * names it runs as, per operating system. It is factual data — a file name beside the name
 * of the program it belongs to — and it is the only maintained list of its kind that is
 * reachable without an account. No licence is stated for it; that is recorded in
 * `assets/apps/README.md` rather than assumed away, and nothing here depends on it staying
 * available.
 *
 * ## What is thrown away, and why
 *
 * A wrong name is worse than a file name: the user reads a proper noun and believes the app
 * knows what it is watching. So the filter is deliberately harsh.
 *
 * - **Anything claimed by more than one title.** Which name won would otherwise depend on
 *   the order of a file nobody reads.
 * - **Generic names** (`game.exe`, `launcher.exe`, `client.exe`, runtimes like `java.exe`).
 *   Even when only one title in the index claims one, the next program to use that name on a
 *   real machine will be mislabelled with confidence.
 * - **Names with fewer than four characters before the extension.** The index really does
 *   claim `at.exe` for a tycoon game from 2003, and `at.exe` is a Windows system binary that
 *   has shipped since NT. Short names are where a file name stops identifying a program, and
 *   4 % of the catalogue is a cheap price for removing the whole class.
 * - **Anything the curated file already names or groups**, so the two never disagree and the
 *   curated entry always wins.
 * - Directory parts of a path (`win64/cs2.exe`), since the app compares base names.
 */

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, '..');
const CURATED = join(REPO, 'assets/apps/presets.json');
const OUTPUT = join(REPO, 'assets/apps/labels.json');
const INDEX_URL = 'https://discord.com/api/v9/applications/detectable';

/**
 * Names too generic to attach a title to.
 *
 * Each is either a name several unrelated programs really use, or a runtime that hosts a
 * program rather than being one. A single claimant in the index proves nothing about the
 * next machine.
 */
const GENERIC = new Set([
  'app.exe',
  'application.exe',
  'bin.exe',
  'client.exe',
  'cmd.exe',
  'dotnet.exe',
  'engine.exe',
  'explorer.exe',
  'game.exe',
  'gamelauncher.exe',
  'java.exe',
  'javaw.exe',
  'launch.exe',
  'launcher.exe',
  'main.exe',
  'node.exe',
  'play.exe',
  'player.exe',
  'python.exe',
  'pythonw.exe',
  'run.exe',
  'server.exe',
  'setup.exe',
  'start.exe',
  'steam.exe',
  'update.exe',
  'updater.exe',
  'win64.exe',
  'windows.exe',
]);

/** The base name of an executable, however the index spelled its path. */
const baseName = (name) => name.split(/[\\/]/).pop()?.trim().toLowerCase() ?? '';

const loadIndex = async (argument) => {
  if (argument !== undefined) return JSON.parse(readFileSync(argument, 'utf8'));
  const response = await fetch(INDEX_URL);
  if (!response.ok) throw new Error(`${INDEX_URL} answered ${response.status}`);
  return response.json();
};

const main = async () => {
  const index = await loadIndex(process.argv[2]);
  const curated = JSON.parse(readFileSync(CURATED, 'utf8'));

  // Everything the curated file speaks for, so the generated catalogue never contradicts it.
  const spokenFor = new Set();
  for (const application of curated.applications ?? []) {
    for (const executable of application.executables ?? []) spokenFor.add(baseName(executable));
  }
  for (const label of curated.labels ?? []) spokenFor.add(baseName(label.executable));

  // Claimants per base name. A set, because one title listing the same executable twice is
  // not a disagreement.
  const claimants = new Map();
  for (const application of index) {
    const title = (application.name ?? '').trim();
    if (title === '') continue;
    for (const executable of application.executables ?? []) {
      if (executable.os !== 'win32') continue;
      const name = baseName(executable.name ?? '');
      if (name === '' || !name.endsWith('.exe')) continue;
      if (!claimants.has(name)) claimants.set(name, new Set());
      claimants.get(name).add(title);
    }
  }

  /** How many characters an executable's stem needs before it identifies anything. */
  const SHORTEST_STEM = 4;

  const labels = {};
  const dropped = { ambiguous: 0, generic: 0, short: 0, curated: 0 };
  for (const [name, titles] of [...claimants.entries()].sort(([left], [right]) =>
    left < right ? -1 : 1,
  )) {
    if (titles.size > 1) {
      dropped.ambiguous += 1;
      continue;
    }
    if (GENERIC.has(name)) {
      dropped.generic += 1;
      continue;
    }
    if (name.length - '.exe'.length < SHORTEST_STEM) {
      dropped.short += 1;
      continue;
    }
    if (spokenFor.has(name)) {
      dropped.curated += 1;
      continue;
    }
    labels[name] = [...titles][0];
  }

  writeFileSync(
    OUTPUT,
    `${JSON.stringify({ schemaVersion: 1, source: 'discord-detectable', labels }, null, 2)}\n`,
    'utf8',
  );

  const kept = Object.keys(labels).length;
  process.stdout.write(
    `assets/apps/labels.json: ${kept} names kept; dropped ${dropped.ambiguous} claimed by ` +
      `several titles, ${dropped.generic} too generic, ${dropped.short} too short, ` +
      `${dropped.curated} already curated.\n`,
  );
};

await main();
