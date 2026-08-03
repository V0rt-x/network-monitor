/**
 * Reading the count chips in a test.
 *
 * A chip is two elements — the number, larger, and the state, smaller — because a count of
 * things in a state must not look like the state badge beside it. Testing Library's text
 * matcher works on the element whose *own* text matches, so "2 Degraded" spread over two
 * spans is not something it can find; this reads the chips as the reader sees them.
 */
export const countChips = (root: ParentNode): readonly string[] =>
  [...root.querySelectorAll('.nm-count')].map((chip) => chip.textContent.trim());

/**
 * The state badges — the pills that say what one thing's state *is*.
 *
 * Distinct from {@link countChips}, and that distinction is the point: a count of endpoints
 * in a state and the state of one endpoint carry the same word and are different claims, so
 * a test that cannot tell them apart is testing the thing that was wrong.
 */
export const stateBadges = (root: ParentNode): readonly string[] =>
  [...root.querySelectorAll('.nm-health')].map((badge) => badge.textContent.trim());
