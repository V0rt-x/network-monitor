// The commit contract from CLAUDE.md, in one command: `npm run check`.
// Every gate runs even after an earlier one fails, so a single pass reports everything
// that needs fixing. Exit code is non-zero if any gate failed.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import process from 'node:process';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const rustRoot = join(repoRoot, 'src-tauri');

// `npm` is a .cmd shim on Windows, so a shell is unavoidable. Passing the whole command
// as one string rather than (command, args[]) keeps Node from warning about unescaped
// arguments — none of the gate commands contain shell metacharacters.
const shell = (command, cwd, options = {}) =>
  spawnSync(command, { cwd, shell: true, stdio: 'inherit', ...options });

const run = (command, cwd) => shell(command, cwd).status ?? 1;

/**
 * `cargo test` regenerates src/bindings.ts from the live Rust IPC surface. Anything
 * other than a clean, committed file means the TypeScript the UI compiles against no
 * longer matches the Rust types.
 */
const bindingsAreCommitted = () => {
  const result = shell('git status --porcelain -- src/bindings.ts', repoRoot, {
    stdio: 'pipe',
    encoding: 'utf8',
  });
  if (result.status !== 0) {
    process.stdout.write(result.stderr || 'git status failed\n');
    return 1;
  }
  const dirty = result.stdout.trim();
  if (dirty) {
    process.stdout.write(`src/bindings.ts is not committed as generated:\n${dirty}\n`);
    return 1;
  }
  process.stdout.write('src/bindings.ts matches the Rust IPC surface.\n');
  return 0;
};

const GATES = [
  { name: 'rustfmt', run: () => run('cargo fmt --all --check', rustRoot) },
  {
    name: 'clippy',
    run: () => run('cargo clippy --workspace --all-targets -- -D warnings', rustRoot),
  },
  { name: 'cargo test', run: () => run('cargo test --workspace', rustRoot) },
  { name: 'bindings up to date', run: bindingsAreCommitted },
  { name: 'tsc', run: () => run('npm run typecheck', repoRoot) },
  { name: 'eslint', run: () => run('npm run lint', repoRoot) },
  { name: 'prettier', run: () => run('npm run format:check', repoRoot) },
  { name: 'vitest', run: () => run('npm run test', repoRoot) },
];

const failed = [];
for (const gate of GATES) {
  process.stdout.write(`\n=== ${gate.name} ===\n`);
  if (gate.run() !== 0) failed.push(gate.name);
}

process.stdout.write('\n');
if (failed.length > 0) {
  process.stdout.write(`FAILED: ${failed.join(', ')}\n`);
  process.exit(1);
}
process.stdout.write('All gates passed.\n');
