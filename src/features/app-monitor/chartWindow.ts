/**
 * Which stretch of the chart is on screen, and how panning and zooming move it.
 *
 * View state, not a measurement: it decides nothing about the numbers, only which of them
 * are drawn. That is why it lives in the frontend at all — Rust owns where a slot begins and
 * what is in it, and this owns where the reader is looking.
 *
 * **The resolution never changes.** Zooming shows fewer or more slots; it does not re-bucket
 * them, because a chart that re-buckets under the reader shows a different spike from the one
 * they zoomed in on. A slot is three seconds at every zoom level, and level two says so.
 */

/** The narrowest span the reader may zoom to, in seconds. */
export const MIN_SPAN_SECS = 120;

/** What the chart opens on, in seconds. */
export const DEFAULT_SPAN_SECS = 20 * 60;

/** A stretch of the time axis, in seconds since monitoring began. */
export interface ChartWindow {
  readonly fromSecs: number;
  readonly toSecs: number;
}

/** The whole of what there is to look at. */
export interface ChartBounds {
  readonly oldestSecs: number;
  readonly newestSecs: number;
}

/**
 * Holds a window inside what exists, keeping its span wherever it can.
 *
 * A reader who has panned to the oldest slot must not be able to drag empty space onto the
 * screen — there is nothing there, and a chart that scrolls past its own data looks broken
 * rather than exhausted. Where the whole history is narrower than the requested span the
 * window becomes the whole history rather than the other way round.
 */
export const clampWindow = (window: ChartWindow, bounds: ChartBounds): ChartWindow => {
  const available = Math.max(0, bounds.newestSecs - bounds.oldestSecs);
  const span = Math.min(Math.max(window.toSecs - window.fromSecs, MIN_SPAN_SECS), available);
  if (span <= 0) return { fromSecs: bounds.oldestSecs, toSecs: bounds.newestSecs };
  const from = Math.min(
    Math.max(window.fromSecs, bounds.oldestSecs),
    Math.max(bounds.newestSecs - span, bounds.oldestSecs),
  );
  return { fromSecs: from, toSecs: from + span };
};

/** The window that follows new samples: the newest `span` seconds there are. */
export const liveWindow = (bounds: ChartBounds, spanSecs: number): ChartWindow =>
  clampWindow({ fromSecs: bounds.newestSecs - spanSecs, toSecs: bounds.newestSecs }, bounds);

/**
 * Zooms about a moment, so what is under the pointer stays under it.
 *
 * `factor` below one narrows the window. The anchor keeps its fractional position across the
 * change, which is what makes a wheel feel like a magnifying glass rather than a slider.
 */
export const zoomWindow = (
  window: ChartWindow,
  bounds: ChartBounds,
  factor: number,
  anchorSecs: number,
): ChartWindow => {
  const span = window.toSecs - window.fromSecs;
  if (span <= 0) return clampWindow(window, bounds);
  const at = Math.min(Math.max((anchorSecs - window.fromSecs) / span, 0), 1);
  const wanted = Math.min(
    Math.max(span * factor, MIN_SPAN_SECS),
    Math.max(bounds.newestSecs - bounds.oldestSecs, MIN_SPAN_SECS),
  );
  const from = anchorSecs - at * wanted;
  return clampWindow({ fromSecs: from, toSecs: from + wanted }, bounds);
};

/** Moves the window by `bySecs`, keeping its span. */
export const panWindow = (window: ChartWindow, bounds: ChartBounds, bySecs: number): ChartWindow =>
  clampWindow({ fromSecs: window.fromSecs + bySecs, toSecs: window.toSecs + bySecs }, bounds);

/**
 * Whether a window is still looking at the newest samples.
 *
 * What decides whether the view follows new data. The tolerance is one slot: a window pinned
 * to the right edge drifts by less than that between emissions, and a reader who has not
 * touched anything must not silently stop following because of a rounding.
 */
export const isLive = (window: ChartWindow, bounds: ChartBounds, stepSecs: number): boolean =>
  window.toSecs >= bounds.newestSecs - stepSecs;
