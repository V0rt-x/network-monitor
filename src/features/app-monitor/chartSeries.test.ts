import { describe, expect, it } from 'vitest';

import type { ChartLine } from './chartSeries';
import {
  alignSeries,
  formatAxisMs,
  formatClock,
  formatSpan,
  LOG_FLOOR_MS,
  readingAt,
  stitchAxis,
  stitchValues,
} from './chartSeries';

const line = (values: (number | null)[], overrides: Partial<ChartLine> = {}): ChartLine => ({
  endpoint: 'udp/1.1.1.1:27015',
  transport: 'udp',
  label: '1.1.1.1:27015',
  address: '1.1.1.1:27015',
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

describe('formatClock', () => {
  /** Midday UTC, so the assertions below do not depend on where the machine is. */
  const NOON = Date.UTC(2026, 7, 4, 12, 0, 0);

  it('labels a blanked tick with nothing rather than throwing', () => {
    // The same hazard as the round-trip axis, and the same consequence: a formatter that
    // throws leaves an empty canvas and no error anywhere.
    expect(formatClock(null, NOON, 'en-GB')).toBe('');
    expect(formatClock(undefined, NOON, 'en-GB')).toBe('');
    expect(formatClock(Number.NaN, NOON, 'en-GB')).toBe('');
  });

  it('says nothing at all where there is no epoch to anchor it to', () => {
    // Absent stays absent: an axis labelled from 1970 would be a figure that is wrong rather
    // than a figure that is missing.
    expect(formatClock(90, null, 'en-GB')).toBe('');
  });

  it('reads as a time of day, to the second', () => {
    // "−45 s" answers "how long ago", and the reader's question after a stutter is "was that
    // when it happened" — which needs a clock. Seconds, because a three-second slot cannot be
    // placed without them; no date, because the axis never spans one.
    expect(formatClock(0, NOON, 'en-GB')).toMatch(/^\d\d:00:00$/);
    expect(formatClock(90, NOON, 'en-GB')).toMatch(/^\d\d:01:30$/);
  });

  it('moves with the epoch rather than with the sample', () => {
    // The epoch is the wall clock minus the *monotonic* elapsed, so a system clock adjusted
    // mid-session moves every label together and moves no sample relative to its neighbours.
    const shifted = formatClock(90, NOON + 3_600_000, 'en-GB');
    expect(shifted).not.toBe(formatClock(90, NOON, 'en-GB'));
    expect(shifted).toMatch(/^\d\d:01:30$/);
  });
});

describe('stitching a fetched history onto the pushed window', () => {
  it('takes the live window wherever the two overlap, and never draws a slot twice', () => {
    // Rust decided where every slot begins and what is in it; this concatenates two arrays
    // already on the same ladder. A slot they both hold is the live one's.
    const axis = stitchAxis([0, 3, 6, 9], [6, 9, 12]);

    expect(axis.elapsedSecs).toEqual([0, 3, 6, 9, 12]);
    expect(axis.fromHistory).toBe(2);
    expect(stitchValues([1, 2, 3, 4], axis.fromHistory, [30, 40, 50])).toEqual([1, 2, 30, 40, 50]);
  });

  it('pads a history shorter than the axis with gaps rather than sliding it', () => {
    // Every array on one axis has to be the same length, or the samples land on the wrong
    // moments — which would be a fabricated measurement rather than a missing one.
    expect(stitchValues([], 2, [30])).toEqual([null, null, 30]);
  });

  it('keeps the whole history when the live window has nothing in it yet', () => {
    const axis = stitchAxis([0, 3], [null, null]);
    expect(axis.fromHistory).toBe(2);
    expect(axis.elapsedSecs).toEqual([0, 3, null, null]);
  });
});

describe('formatSpan', () => {
  it('says what the view covers in a unit a reader thinks in', () => {
    expect(formatSpan(45)).toBe('45 s');
    expect(formatSpan(1_200)).toBe('20 min');
    expect(formatSpan(3_600)).toBe('1 h 0 min');
  });
});

describe('readingAt', () => {
  const rtt = line([24, 26, 25]);
  const slow = line([80, 91, null], {
    endpoint: 'udp/1.1.1.2:27015',
    address: '1.1.1.2:27015',
    label: '1.1.1.2:27015',
  });

  it('reports every line at the moment, not the nearest one', () => {
    // The chart's stated job is "which of these is the odd one out", and at a given second
    // that is a question about all of them at once.
    const reading = readingAt(alignSeries([0, 3, 6], [rtt, slow]), [rtt, slow], 1);

    expect(reading.elapsedSecs).toBe(3);
    expect(reading.entries).toHaveLength(2);
  });

  it('orders them worst first, the way the list beside the chart already is', () => {
    const reading = readingAt(alignSeries([0, 3, 6], [rtt, slow]), [rtt, slow], 1);

    expect(reading.entries.map((entry) => entry.valueMs)).toEqual([91, 26]);
  });

  it('keeps a slot with nothing in it, last and still absent', () => {
    // Dropping it would be indistinguishable from a line that is doing fine; turning it into
    // a zero would be a measurement that did not happen.
    const reading = readingAt(alignSeries([0, 3, 6], [rtt, slow]), [rtt, slow], 2);

    expect(reading.entries.map((entry) => entry.valueMs)).toEqual([25, null]);
  });

  it('carries which of its entries is a route, so the tooltip can never call one a ping', () => {
    const route = line([80], { isPath: true });
    const reading = readingAt(alignSeries([0], [route]), [route], 0);

    expect(reading.entries[0]?.isPath).toBe(true);
    // And it is named by the address, not by the "Route to …" label the chart draws with:
    // the quantity is stated separately, so the entry never says the same thing twice.
    expect(reading.entries[0]?.address).toBe('1.1.1.1:27015');
  });
});
