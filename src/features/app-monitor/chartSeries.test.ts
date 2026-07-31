import { describe, expect, it } from 'vitest';

import type { ChartLine } from './chartSeries';
import { alignSeries } from './chartSeries';

const line = (values: (number | null)[], overrides: Partial<ChartLine> = {}): ChartLine => ({
  endpoint: 'udp/1.1.1.1:27015',
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
});
