import { describe, expect, it } from 'vitest';

import { EndpointColours } from './endpointColours';

describe('EndpointColours', () => {
  it('gives each endpoint a colour of its own', () => {
    const colours = new EndpointColours();
    colours.reconcile(['a', 'b', 'c']);

    const assigned = ['a', 'b', 'c'].map((key) => colours.of(key));
    expect(new Set(assigned).size).toBe(3);
  });

  it('keeps an endpoint on its colour when the list reorders', () => {
    // The list is sorted worst first and re-sorts on every emission, so an index into it
    // would repaint every line the moment one endpoint got worse.
    const colours = new EndpointColours();
    colours.reconcile(['a', 'b', 'c']);
    const before = colours.of('c');

    colours.reconcile(['c', 'a', 'b']);

    expect(colours.of('c')).toBe(before);
  });

  it('keeps an endpoint on its colour when another appears', () => {
    const colours = new EndpointColours();
    colours.reconcile(['a']);
    const before = colours.of('a');

    colours.reconcile(['a', 'b']);

    expect(colours.of('a')).toBe(before);
    expect(colours.of('b')).not.toBe(before);
  });

  it('releases the colour of an endpoint that is gone', () => {
    // Without this a long session of endpoints coming and going would drift off the
    // distinct end of the palette and start handing out the spare.
    const colours = new EndpointColours();
    colours.reconcile(['a', 'b']);
    const freed = colours.of('a');

    colours.reconcile(['b']);
    colours.reconcile(['b', 'c']);

    expect(colours.of('c')).toBe(freed);
  });

  it('still answers for an endpoint it was never told about', () => {
    const colours = new EndpointColours();
    expect(colours.of('unknown')).toMatch(/^#[0-9a-f]{6}$/i);
  });

  it('runs out of distinct colours rather than repeating a live one', () => {
    const colours = new EndpointColours();
    const many = Array.from({ length: 40 }, (_, index) => `endpoint-${String(index)}`);
    colours.reconcile(many);

    const assigned = many.map((key) => colours.of(key));
    // Every colour is a real one; the palette runs out and the rest share the spare, which
    // is honest — colour identifies here, and beyond the palette it stops being able to.
    expect(assigned.every((colour) => /^#[0-9a-f]{6}$/i.test(colour))).toBe(true);
    expect(new Set(assigned).size).toBeGreaterThan(8);
  });
});
