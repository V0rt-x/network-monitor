import '@testing-library/jest-dom/vitest';

/**
 * jsdom implements no `matchMedia`, and uPlot calls it while its *module* is being
 * evaluated — so a component that merely imports uPlot cannot be loaded in a test without
 * this, whether or not it ever draws anything.
 *
 * Deliberately the narrowest possible stand-in: it answers "no" to every query and
 * registers no listeners. Nothing in this app branches on a media query; uPlot uses it to
 * follow device pixel ratio, which a headless renderer does not have either.
 */
if (typeof window !== 'undefined' && typeof window.matchMedia !== 'function') {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => undefined,
      removeListener: () => undefined,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      dispatchEvent: () => false,
    }) as MediaQueryList;
}
