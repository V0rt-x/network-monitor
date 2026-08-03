import { describe, expect, it } from 'vitest';

import { applyHeldOrder } from './heldOrder';

const of = (...keys: string[]) => keys.map((key) => ({ key }));

describe('applyHeldOrder', () => {
  it('replays the order that was on screen', () => {
    expect(applyHeldOrder(['a', 'b', 'c'], of('c', 'a', 'b'))).toEqual(of('a', 'b', 'c'));
  });

  it('keeps the newest data, only the order is held', () => {
    const incoming = [
      { key: 'a', health: 'unreachable' },
      { key: 'b', health: 'ok' },
    ];
    expect(applyHeldOrder(['b', 'a'], incoming)).toEqual([incoming[1], incoming[0]]);
  });

  it('drops what has gone rather than leaving a hole', () => {
    expect(applyHeldOrder(['a', 'b', 'c'], of('a', 'c'))).toEqual(of('a', 'c'));
  });

  it('appends what is new instead of making room for it in the middle', () => {
    // A newly discovered endpoint is a real finding and must appear. Inserting it where its
    // severity puts it would move every row below, which is what the hold exists to prevent.
    expect(applyHeldOrder(['a', 'b'], of('new', 'a', 'b'))).toEqual(of('a', 'b', 'new'));
  });

  it('keeps the order Rust sent among the newcomers', () => {
    expect(applyHeldOrder(['a'], of('worst', 'a', 'better'))).toEqual(of('a', 'worst', 'better'));
  });

  it('is the incoming list when nothing was held', () => {
    expect(applyHeldOrder([], of('b', 'a'))).toEqual(of('b', 'a'));
  });
});
