import { describe, expect, it } from 'vitest';

import { spanOf } from './duration';

describe('spanOf', () => {
  it('keeps seconds while they are still worth more than the rounding', () => {
    expect(spanOf(0)).toEqual({ key: 'span.seconds', params: { seconds: 0 } });
    expect(spanOf(45)).toEqual({ key: 'span.seconds', params: { seconds: 45 } });
    // The case the threshold exists for: "2 min" would be a worse answer than "90 s".
    expect(spanOf(90)).toEqual({ key: 'span.seconds', params: { seconds: 90 } });
  });

  it('moves to minutes once seconds stop being readable', () => {
    expect(spanOf(120)).toEqual({ key: 'span.minutes', params: { minutes: 2 } });
    expect(spanOf(3_599)).toEqual({ key: 'span.minutes', params: { minutes: 60 } });
  });

  it('reads a long session in hours, because that is how long one runs', () => {
    // A monitor is left running for a whole evening on purpose; "492 min" is arithmetically
    // the truth and unreadable as a time, which the chart axis learned the hard way.
    expect(spanOf(3_600)).toEqual({ key: 'span.hours', params: { hours: 1, minutes: 0 } });
    expect(spanOf(29_520)).toEqual({ key: 'span.hours', params: { hours: 8, minutes: 12 } });
  });

  it('never writes a negative duration', () => {
    // A clock that appeared to move backwards must produce a zero, not a minus sign.
    expect(spanOf(-5)).toEqual({ key: 'span.seconds', params: { seconds: 0 } });
  });
});
