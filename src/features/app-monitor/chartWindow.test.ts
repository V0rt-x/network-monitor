import { describe, expect, it } from 'vitest';

import {
  clampWindow,
  DEFAULT_SPAN_SECS,
  isLive,
  liveWindow,
  MIN_SPAN_SECS,
  panWindow,
  zoomWindow,
} from './chartWindow';

/** An hour of history, which is what the ring holds. */
const HOUR = { oldestSecs: 0, newestSecs: 3_600 };

describe('the chart window', () => {
  it('opens on twenty minutes of the newest samples', () => {
    // Two minutes cannot answer "is this worse than it was at the start of the match", which
    // is the question the reader has after one.
    expect(liveWindow(HOUR, DEFAULT_SPAN_SECS)).toEqual({ fromSecs: 2_400, toSecs: 3_600 });
  });

  it('shows the whole of a session shorter than the span it wants', () => {
    // A card that mounted a minute ago must not draw nineteen minutes of empty axis.
    expect(liveWindow({ oldestSecs: 0, newestSecs: 60 }, DEFAULT_SPAN_SECS)).toEqual({
      fromSecs: 0,
      toSecs: 60,
    });
  });

  it('never scrolls past what exists, at either end', () => {
    // A chart that scrolls into empty space looks broken rather than exhausted.
    const window = { fromSecs: 3_000, toSecs: 3_600 };
    expect(panWindow(window, HOUR, 5_000)).toEqual({ fromSecs: 3_000, toSecs: 3_600 });
    expect(panWindow(window, HOUR, -10_000)).toEqual({ fromSecs: 0, toSecs: 600 });
  });

  it('keeps its span while panning', () => {
    const panned = panWindow({ fromSecs: 1_000, toSecs: 1_600 }, HOUR, -300);
    expect(panned.toSecs - panned.fromSecs).toBe(600);
    expect(panned.fromSecs).toBe(700);
  });

  it('zooms about the moment under the pointer, keeping it there', () => {
    // What makes a wheel feel like a magnifying glass rather than a slider.
    const window = { fromSecs: 1_000, toSecs: 2_000 };
    const zoomed = zoomWindow(window, HOUR, 0.5, 1_500);

    expect(zoomed.toSecs - zoomed.fromSecs).toBe(500);
    // The anchor sat halfway across; it still does.
    expect((1_500 - zoomed.fromSecs) / (zoomed.toSecs - zoomed.fromSecs)).toBeCloseTo(0.5);
  });

  it('will not zoom in past two minutes', () => {
    // Below a slot or two the chart stops being a picture of anything: it is three seconds
    // per point at every zoom level, because re-bucketing under the reader would show them a
    // different spike from the one they zoomed in on.
    const zoomed = zoomWindow({ fromSecs: 0, toSecs: 200 }, HOUR, 0.1, 100);
    expect(zoomed.toSecs - zoomed.fromSecs).toBe(MIN_SPAN_SECS);
  });

  it('will not zoom out past the whole ring', () => {
    const zoomed = zoomWindow({ fromSecs: 0, toSecs: 3_600 }, HOUR, 10, 1_800);
    expect(zoomed).toEqual({ fromSecs: 0, toSecs: 3_600 });
  });

  it('knows when the view is still following new samples', () => {
    // The only state in which the chart moves on its own — and a window pinned to the right
    // edge drifts by less than a slot between emissions, so the tolerance is one slot.
    expect(isLive({ fromSecs: 2_400, toSecs: 3_600 }, HOUR, 3)).toBe(true);
    expect(isLive({ fromSecs: 2_399, toSecs: 3_599 }, HOUR, 3)).toBe(true);
    expect(isLive({ fromSecs: 0, toSecs: 1_200 }, HOUR, 3)).toBe(false);
  });

  it('holds a window inside a history that has nothing in it yet', () => {
    // The first render of a card, before a single slot has been measured.
    const empty = { oldestSecs: 0, newestSecs: 0 };
    expect(clampWindow({ fromSecs: -100, toSecs: 500 }, empty)).toEqual({
      fromSecs: 0,
      toSecs: 0,
    });
  });
});
