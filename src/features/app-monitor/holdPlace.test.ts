import { describe, expect, it } from 'vitest';

import { holdPlace } from './holdPlace';

describe('holdPlace', () => {
  it('leaves the order alone when nothing is pinned', () => {
    expect(holdPlace(['a', 'b', 'c'], null, null)).toEqual(['a', 'b', 'c']);
    expect(holdPlace(['a', 'b', 'c'], 'b', null)).toEqual(['a', 'b', 'c']);
  });

  it('holds the pinned row where the reader left it', () => {
    // Rust re-sorted it to the front — a genuine change, not a flicker — and the reader is
    // in the middle of reading it where it was.
    expect(holdPlace(['b', 'a', 'c'], 'b', 1)).toEqual(['a', 'b', 'c']);
  });

  it('survives a new endpoint appearing above it', () => {
    // Discovery finds one mid-match and Rust sorts it first; the pinned row must not slide.
    expect(holdPlace(['new', 'a', 'b'], 'a', 0)).toEqual(['a', 'new', 'b']);
  });

  it('survives the list shrinking under it', () => {
    // An endpoint the application stopped using is forgotten. The held index can now be past
    // the end, and clamping is what keeps the row on the page rather than dropping it.
    expect(holdPlace(['a', 'b'], 'b', 7)).toEqual(['a', 'b']);
    expect(holdPlace(['a', 'b'], 'a', 5)).toEqual(['b', 'a']);
  });

  it('does nothing for a row that is no longer listed', () => {
    expect(holdPlace(['a', 'b'], 'gone', 0)).toEqual(['a', 'b']);
  });

  it('does not move a row that is already where it belongs', () => {
    const order = ['a', 'b', 'c'];
    expect(holdPlace(order, 'b', 1)).toBe(order);
  });
});
