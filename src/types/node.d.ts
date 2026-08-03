/**
 * The one Node API the test suite needs, declared rather than depended on.
 *
 * Reading `styles.css` is how the design system is enforced (`styles.test.ts`), and Vite's
 * own `?raw` import cannot supply it: under Vitest the CSS plugin claims the file first and
 * hands back an empty string. Node's `readFileSync` is what is left.
 *
 * `@types/node` would bring the whole of Node's surface — `process`, `Buffer`, `fs` in full —
 * into a browser application's type-check, where any of it could then be used by accident.
 * One function is declared instead, so nothing else becomes reachable.
 */
declare module 'node:fs' {
  export function readFileSync(path: string, encoding: 'utf8'): string;
}
