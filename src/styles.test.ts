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

  it('says where the focus is, for everything that can take it', () => {
    expect(BODY).toContain(
      ':where(button, a, input, select, textarea, summary, [tabindex]):focus-visible',
    );
  });
});
