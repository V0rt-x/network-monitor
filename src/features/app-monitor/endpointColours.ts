/**
 * Stable per-endpoint line colours for the application chart.
 *
 * Two rules, both from the constraint the chart is under.
 *
 * **Colour identifies, it never states.** The worst-first ordered list remains the
 * authority on health; a line's colour says only "this line is that endpoint". Reusing the
 * health palette here would make the chart a second, quieter verdict — and one that
 * contradicts the list the moment two endpoints share a state.
 *
 * **A colour belongs to an endpoint for as long as it exists.** The list reorders itself by
 * severity on every emission, so an index into it would repaint every line whenever one
 * endpoint got worse. Assignment is therefore held, not derived: a new endpoint takes the
 * lowest free slot and keeps it until it is gone.
 */

/**
 * Mid-tone hues, distinguishable from each other and from the health colours.
 *
 * Deliberately neither very dark nor very light: the app ships one dark theme today, and a
 * palette that only worked there would have to be redrawn the day a light one lands.
 */
const PALETTE = [
  '#58a6ff',
  '#e3a008',
  '#4ac1a5',
  '#f778ba',
  '#a371f7',
  '#79c0ff',
  '#d18616',
  '#56d364',
  '#ff7b72',
  '#8ddb8c',
  '#c9a0ff',
  '#39c5cf',
] as const;

/** The colour a line falls back to once the palette is exhausted. */
const SPARE = '#8b949e';

/**
 * Shapes the swatch takes, so colour is never the only thing telling two rows apart.
 *
 * The swatch tying a row to its line was the one place in this product where colour carried
 * meaning by itself — everywhere else a state has its word beside it. Twelve hues are
 * distinguishable to most people and to nobody with a red-green deficiency; four shapes
 * crossed with twelve colours are.
 *
 * The cycle length is deliberately coprime with nothing in particular: four shapes over
 * twelve colours means adjacent slots differ in shape, which is the case that matters,
 * because adjacent slots are what a freshly discovered endpoint gets.
 */
const SHAPES = ['square', 'circle', 'diamond', 'triangle'] as const;

/** One of the shapes a swatch can take. */
export type SwatchShape = (typeof SHAPES)[number];

/**
 * Keeps each endpoint on the colour it was first given.
 *
 * Mutated in place and held in a ref by the component that owns the chart: this is view
 * state, not a measurement, and it must survive the re-render that new data causes.
 */
export class EndpointColours {
  readonly #assigned = new Map<string, number>();

  /**
   * Assigns colours for exactly this set of endpoints, releasing any that have gone.
   *
   * Called with the current keys on every emission. Releasing first is what lets a long
   * session of endpoints coming and going keep using the low, distinct end of the palette
   * rather than drifting off it.
   */
  reconcile(keys: readonly string[]): void {
    const wanted = new Set(keys);
    for (const key of this.#assigned.keys()) {
      if (!wanted.has(key)) this.#assigned.delete(key);
    }

    const taken = new Set(this.#assigned.values());
    for (const key of keys) {
      if (this.#assigned.has(key)) continue;
      let slot = 0;
      while (slot < PALETTE.length && taken.has(slot)) slot += 1;
      this.#assigned.set(key, slot);
      taken.add(slot);
    }
  }

  /** The colour an endpoint's line is drawn in. */
  of(key: string): string {
    const slot = this.#assigned.get(key);
    if (slot === undefined) return SPARE;
    return PALETTE[slot] ?? SPARE;
  }

  /** The shape its swatch takes, so the pairing is not carried by colour alone. */
  shapeOf(key: string): SwatchShape {
    const slot = this.#assigned.get(key) ?? 0;
    return SHAPES[slot % SHAPES.length] ?? 'square';
  }
}
