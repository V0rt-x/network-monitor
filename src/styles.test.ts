import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

// Relative to the project root, which is where Vitest runs. Vite's own `?raw` import was the
// obvious way to do this and does not work: under Vitest the CSS plugin claims the file
// first and hands back an empty string, so the assertions below would all pass on nothing.
const CSS = readFileSync('src/styles.css', 'utf8');

/**
 * The design system, enforced rather than agreed.
 *
 * `styles.css` carried sixteen spacing values, fifteen font sizes, ten radii and two
 * incompatible palettes at once, and that is the single largest reason the product read as
 * homemade — the eye sees that the distances between blocks are arbitrary long before it can
 * say why. A scale nothing checks is a scale that grows a sixteenth step the next time
 * something looks a pixel off, so this reads the stylesheet and fails on one.
 */
/**
 * Everything after the `:root` block, which is where the tokens are allowed to be values.
 *
 * Comments go first: they quote the values they replaced, and a rule about literals cannot
 * be allowed to trip over the sentence explaining why a literal was removed.
 */
const BODY = CSS.slice(CSS.indexOf('* {\n  box-sizing: border-box;\n}')).replace(
  /\/\*[\s\S]*?\*\//g,
  '',
);

/** Every `property: value;` pair outside `:root`, with its declaring selector. */
const declarations = (property: string): { selector: string; value: string }[] => {
  const found: { selector: string; value: string }[] = [];
  const rules = BODY.matchAll(/([^{}]+)\{([^{}]*)\}/g);
  for (const rule of rules) {
    const selector = (rule[1] ?? '').trim().replace(/\s+/g, ' ');
    for (const pair of (rule[2] ?? '').matchAll(/([a-z-]+)\s*:\s*([^;]+);/g)) {
      if (pair[1] === property) found.push({ selector, value: (pair[2] ?? '').trim() });
    }
  }
  return found;
};

/** A length that is not a token: `1.5rem`, `12px`, `0.8em`. Zero and `auto` are neither. */
const RAW_LENGTH = /(?<![\w-])-?\d*\.?\d+(rem|em|px)\b/;

describe('the design system', () => {
  it('sizes every piece of type from the type scale', () => {
    for (const { selector, value } of declarations('font-size')) {
      expect(`${selector} { font-size: ${value} }`).toMatch(/var\(--nm-text-/);
    }
  });

  it('rounds every corner from the radius scale', () => {
    for (const { selector, value } of declarations('border-radius')) {
      // A circle is a shape rather than a size — the one radius no scale can express.
      if (value === '50%') continue;
      expect(`${selector} { border-radius: ${value} }`).toMatch(/var\(--nm-radius-/);
    }
  });

  it('spaces everything from the spacing scale', () => {
    // `-1px` is the one exception and it is not spacing: it is how an element is taken out
    // of the visual flow while staying readable to a screen reader.
    const exempt = new Set(['.nm-visually-hidden']);
    for (const property of ['gap', 'row-gap', 'column-gap', 'padding', 'margin', 'margin-top']) {
      for (const { selector, value } of declarations(property)) {
        if (exempt.has(selector)) continue;
        expect(`${selector} { ${property}: ${value} }`).not.toMatch(RAW_LENGTH);
      }
    }
  });

  it('reflows the columns at the same two widths everywhere', () => {
    // Four different `minmax()` bases meant the page rebuilt itself at four different window
    // widths while it was being resized, which is a large part of why it felt unsettled.
    for (const base of BODY.matchAll(/minmax\(([^,]+),/g)) {
      const value = (base[1] ?? '').trim();
      // `minmax(0, 1fr)` is a column that may shrink, not a breakpoint.
      if (value === '0') continue;
      expect(value).toMatch(/var\(--nm-col-[a-z]+\)/);
    }
  });

  it('draws every colour from one palette', () => {
    // Two palettes were in use at once — Tailwind for the surfaces, Primer for the states —
    // plus a third amber duplicating a second one. A literal colour outside `:root` is how
    // a fourth would arrive.
    expect(BODY).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    expect(BODY).not.toMatch(/--nm-warn/);
  });

  it('lets a card set the distances inside it, once', () => {
    // Every card was a plain block whose children each chose their own top margin, so the
    // distances between them were arbitrary and — margins being what they are — collapsed
    // unpredictably as well.
    for (const card of [
      '.nm-appcard',
      '.nm-picker',
      '.nm-row__detail',
      '.nm-section',
      '.nm-help-page__section',
    ]) {
      const rule = BODY.slice(BODY.indexOf(`${card} {`));
      const block = rule.slice(0, rule.indexOf('}'));
      expect(`${card}: ${block}`).toContain('flex-direction: column');
      expect(`${card}: ${block}`).toMatch(/gap: var\(--nm-space-/);
    }
  });

  it('never makes a table cell a flex container', () => {
    // `display: flex` on a `<td>` takes it out of table layout: the cell stops sharing the
    // row's box and draws its own bottom border at its own height, so the rule under a row
    // steps in the middle of it. The flex layout belongs on a wrapper inside the cell.
    //
    // Selectors that *end* in a cell are what matters — `.nm-endpoint__columns > .nm-path`
    // is a rule about a panel, and `td button` is a rule about a button.
    const CELL = /(^|[\s,>+~])(td|\.nm-endpoint__(identity|state|network|figure|disclose))$/;
    for (const { selector, value } of declarations('display')) {
      if (!value.startsWith('flex') && !value.startsWith('inline-flex')) continue;
      for (const one of selector.split(',').map((part) => part.trim())) {
        expect(`${one} { display: ${value} }`).not.toMatch(CELL);
      }
    }
  });

  it('gives every row of the connection table the same height', () => {
    // A row carrying a warm-up badge and a row carrying none must be the same size, or the
    // list changes shape as badges expire under the reader.
    const rule = BODY.slice(BODY.indexOf('.nm-endpoints tbody tr {'));
    expect(rule.slice(0, rule.indexOf('}'))).toContain('height: var(--nm-row-height)');
  });

  it('keeps a floating panel inside the window, whatever is in it', () => {
    // jsdom measures every rectangle as zero, so the *placement* cannot be asserted by
    // rendering. What can be asserted is the declared maximum, which is what stops a long
    // sentence running past the panel and the panel running past the window — and it is the
    // half of the defect that no amount of flipping fixes.
    for (const floating of ['.nm-help__panel', '.nm-charttip']) {
      const rule = BODY.slice(BODY.indexOf(`${floating} {`));
      const block = rule.slice(0, rule.indexOf('}'));
      expect(`${floating}: ${block}`).toMatch(/max-width: min\(.+, calc\(100vw - /);
      expect(`${floating}: ${block}`).toContain('overflow-wrap: anywhere');
    }
    // And both flip on both axes: 6.7 handled the right edge and left the bottom one, so a
    // panel opened on the last row of a long table still hung outside the window.
    for (const flipped of [
      '.nm-help__panel--flipped',
      '.nm-help__panel--above',
      '.nm-charttip--above',
    ]) {
      expect(BODY).toContain(`${flipped} {`);
    }
  });

  it('tells containers apart by their surface, never by a line around them', () => {
    // Every container was a one-pixel box on a flat background, so a card, a panel, a table
    // and a details block all weighed the same and nothing was obviously more important
    // than anything else — which is why the page read as a form rather than an instrument.
    for (const container of [
      '.nm-panel',
      '.nm-picker',
      '.nm-apps__empty',
      '.nm-appcard',
      '.nm-primary',
      '.nm-row',
      '.nm-verdict',
      '.nm-status__legend',
      '.nm-help-page__contents',
      '.nm-help-page__section',
      '.nm-path,\n.nm-flow',
    ]) {
      const rule = BODY.slice(BODY.indexOf(`${container} {`));
      const block = rule.slice(0, rule.indexOf('}'));
      expect(`${container}: ${block}`).toMatch(/background: var\(--nm-(bg|surface)\)/);
      // A `border-left` is allowed and is exactly one thing: the verdict's own colour.
      expect(`${container}: ${block}`).not.toMatch(/\bborder:/);
    }
  });

  it('leaves a hairline only where one is doing work', () => {
    // Table rows, the two controls that need an affordance edge, the segmented navigation,
    // and what floats over the page — where the edge is what separates an overlay from
    // whatever is under it.
    const allowed = new Set([
      '.nm-nav',
      '.nm-button',
      '.nm-badge',
      '.nm-tokens__names',
      '.nm-charttip',
      '.nm-help__panel',
      ".nm-field select, .nm-field input[type='range']",
      '.nm-picker__search input',
      '.nm-help-page__filter input',
      // Not an edge round a box: it is how two of the six state tokens are *shaped*, and a
      // ring and an outline are the two shapes that cannot be drawn any other way.
      '.nm-token--carryingTraffic',
      '.nm-token--unknown',
      '.nm-token--tunnelled',
    ]);
    for (const { selector, value } of declarations('border')) {
      // `border: 0` is a rule *removing* one, which is the opposite of the thing under test.
      if (value === '0') continue;
      if (selector.includes('table') || selector.endsWith('th') || selector.endsWith('td')) {
        continue;
      }
      expect(allowed).toContain(selector);
    }
  });

  it('says where the focus is, for everything that can take it', () => {
    expect(BODY).toContain(
      ':where(button, a, input, select, textarea, summary, [tabindex]):focus-visible',
    );
  });
});
