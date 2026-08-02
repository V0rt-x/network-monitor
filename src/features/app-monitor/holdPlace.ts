/**
 * Keeps a pinned endpoint where the reader left it.
 *
 * Rust orders the list worst-first and holds a health change for a few seconds before it
 * moves a row, which covers the ordinary flicker. What it cannot cover is everything else
 * that reorders a list while someone is reading one row of it: a new endpoint discovered
 * mid-match, an old one forgotten, a genuine change in another row.
 *
 * Pinning is the user's own answer to that — "keep this one still" — so it has to survive
 * all of those. This moves the pinned key back to the index it occupied when it was pinned,
 * and leaves every other row in the order Rust decided.
 *
 * The pin is deliberately *positional* rather than a promotion to the top: a row that jumped
 * to the top the moment it was pinned would be the same jump the pin exists to prevent.
 */
export const holdPlace = <T>(
  order: readonly T[],
  pinned: T | null,
  heldAt: number | null,
): readonly T[] => {
  if (pinned === null || heldAt === null) return order;
  const from = order.indexOf(pinned);
  if (from === -1) return order;
  // Clamped: the list can be shorter than it was when the pin was taken — endpoints are
  // forgotten — and an index past the end would drop the row rather than hold it.
  const to = Math.min(Math.max(heldAt, 0), order.length - 1);
  if (from === to) return order;

  const held = [...order];
  held.splice(from, 1);
  held.splice(to, 0, pinned);
  return held;
};
