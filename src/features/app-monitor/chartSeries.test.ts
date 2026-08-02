import { describe, expect, it } from 'vitest';

import type { ChartLine } from './chartSeries';
import { alignSeries, formatAxisMs, LOG_FLOOR_MS } from './chartSeries';

const line = (values: (number | null)[], overrides: Partial<ChartLine> = {}): ChartLine => ({
  endpoint: 'udp/1.1.1.1:27015',
  transport: 'udp',
  label: '1.1.1.1:27015',
  values,
  colour: '#58a6ff',
  isPath: false,
  ...overrides,
});

describe('alignSeries', () => {
  it('puts the shared axis first and every line after it', () => {
    const data = alignSeries(
      [-2, -1, 0],
      [line([1, 2, 3]), line([4, 5, 6], { endpoint: 'b', label: 'b' })],
    );

    expect(data).toEqual([
      [-2, -1, 0],
      [1, 2, 3],
      [4, 5, 6],
    ]);
  });

  it('keeps a gap inside a line untouched', () => {
    // uPlot does not draw across a null, which is how an outage stays a break in the line
    // rather than a straight segment through it.
    const data = alignSeries([-1, 0], [line([null, 5])]);

    expect(data[1]).toEqual([null, 5]);
  });

  it('drops an unrepresentable slot from the axis and from every line together', () => {
    // Dropping it from one and not the others would slide the remaining measurements onto
    // the wrong moments — which is a chart that lies rather than one with a hole in it.
    const data = alignSeries(
      [-2, null, 0],
      [line([1, 2, 3]), line([4, 5, 6], { endpoint: 'b', label: 'b' })],
    );

    expect(data).toEqual([
      [-2, 0],
      [1, 3],
      [4, 6],
    ]);
  });

  it('pads a line that is shorter than the axis rather than shifting it', () => {
    const data = alignSeries([-2, -1, 0], [line([1])]);

    expect(data[1]).toEqual([1, null, null]);
  });

  it('survives an axis with no slots and a chart with no lines', () => {
    expect(alignSeries([], [])).toEqual([[]]);
    expect(alignSeries([-1, 0], [])).toEqual([[-1, 0]]);
  });

  it('draws a value a logarithmic axis cannot place at the floor rather than breaking', () => {
    // A logarithmic scale has no zero. A round trip of zero needs one faster than a
    // microsecond and cannot happen over a network, but an axis that broke if it ever did
    // would be a latent crash. The exact figure is in the row beside the chart.
    const data = alignSeries([-1, 0], [line([0, -5])]);

    expect(data[1]).toEqual([LOG_FLOOR_MS, LOG_FLOOR_MS]);
  });

  it('leaves an ordinary measurement exactly as it was measured', () => {
    const data = alignSeries([-1, 0], [line([0.5, 240])]);

    expect(data[1]).toEqual([0.5, 240]);
  });
});

describe('formatAxisMs', () => {
  it('labels a blanked minor tick with nothing rather than throwing', () => {
    // The bug this exists for: a logarithmic axis runs its splits through a filter that
    // blanks the minor ticks between powers, and the filtered array is what reaches the
    // formatter. Treating one of those as a number threw in the middle of a draw, which
    // left an empty canvas, no error anywhere, and a note underneath still describing a
    // chart that was not there.
    expect(formatAxisMs(null)).toBe('');
    expect(formatAxisMs(undefined)).toBe('');
    expect(formatAxisMs(Number.NaN)).toBe('');
    expect(formatAxisMs(Number.POSITIVE_INFINITY)).toBe('');
  });

  it('labels milliseconds the way someone comparing latencies reads them', () => {
    expect(formatAxisMs(100)).toBe('100');
    expect(formatAxisMs(1000)).toBe('1000');
    expect(formatAxisMs(23.6)).toBe('24');
    expect(formatAxisMs(1)).toBe('1');
  });

  it('keeps a decimal below a millisecond, where rounding would print zero', () => {
    expect(formatAxisMs(0.4)).toBe('0.4');
    expect(formatAxisMs(0.05)).toBe('0.1');
  });
});
