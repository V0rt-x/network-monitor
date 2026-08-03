import { useEffect, useRef, useState } from 'react';

/** How long after the reader leaves the list before Rust's order is applied, in ms. */
const RELEASE_MS = 2_000;

/**
 * Rust's order, replayed in the order that was on screen when the reader arrived.
 *
 * The figures are always the newest ones — this reorders, it never staleness-freezes a
 * measurement. Keys the held order has never seen go to the end in the order Rust sent them,
 * and keys that have gone simply drop out.
 *
 * Appending rather than inserting is the point: an endpoint discovered mid-read is a real
 * finding and must appear, but making room for it in the middle would move every row below
 * it, which is the thing being prevented.
 */
export const applyHeldOrder = <T extends { readonly key: string }>(
  held: readonly string[],
  incoming: readonly T[],
): readonly T[] => {
  const byKey = new Map(incoming.map((entry) => [entry.key, entry]));
  const ordered: T[] = [];
  for (const key of held) {
    const entry = byKey.get(key);
    if (entry === undefined) continue;
    ordered.push(entry);
    byKey.delete(key);
  }
  for (const entry of incoming) {
    if (byKey.has(entry.key)) ordered.push(entry);
  }
  return ordered;
};

/**
 * Holds a list still while it is being read, and lets Rust reorder it once it is not.
 *
 * Rust orders endpoints worst first, and it re-orders on every emission — so any change of
 * state anywhere swaps rows under whoever is reading one. `holdPlace` solved that for a
 * single pinned row; this solves it for the case that does not involve pinning anything,
 * which is most of them: a reader with the pointer over the list, or with focus inside it,
 * is reading, and the list waits.
 *
 * **View state, not business logic.** What the order *is* remains Rust's judgement about
 * severity; when to apply a new one is a fact about where the pointer is, which only the
 * browser knows. The precedent is `holdPlace`, which lives here for the same reason.
 *
 * The delay after leaving is deliberate rather than immediate: a pointer travelling from one
 * row to a control just outside the list would otherwise reshuffle everything it had just
 * left.
 */
export const useHeldOrder = <T extends { readonly key: string }>(
  incoming: readonly T[],
  releaseMs: number = RELEASE_MS,
): {
  readonly shown: readonly T[];
  /** Spread onto the element that contains the rows. */
  readonly holdProps: {
    readonly onMouseEnter: () => void;
    readonly onMouseLeave: () => void;
    readonly onFocus: () => void;
    readonly onBlur: () => void;
  };
} => {
  const [reading, setReading] = useState(false);
  const [held, setHeld] = useState<readonly string[] | null>(null);
  // The newest order, kept out of state so that arriving data does not re-render on its own
  // account — it is already re-rendering for the figures.
  const latest = useRef<readonly string[]>([]);
  latest.current = incoming.map((entry) => entry.key);

  useEffect(() => {
    if (reading) {
      setHeld((current) => current ?? latest.current);
      return undefined;
    }
    const timer = setTimeout(() => {
      setHeld(null);
    }, releaseMs);
    return () => {
      clearTimeout(timer);
    };
  }, [reading, releaseMs]);

  const enter = () => {
    setReading(true);
  };
  const leave = () => {
    setReading(false);
  };

  return {
    shown: held === null ? incoming : applyHeldOrder(held, incoming),
    holdProps: { onMouseEnter: enter, onMouseLeave: leave, onFocus: enter, onBlur: leave },
  };
};
