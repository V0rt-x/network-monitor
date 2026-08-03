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
 * The state tokens — the marks that say what one thing's state *is*, read by their words.
 *
 * Distinct from {@link countChips}, and that distinction is the point: a count of endpoints
 * in a state and the state of one endpoint carry the same word and are different claims, so
 * a test that cannot tell them apart is testing the thing that was wrong.
 *
 * The word comes off the accessible name rather than off the text, because that is where it
 * now lives at all times — a token shows it on hover, on focus and in the row's expander, and
 * a reader who never does any of those three still has it read to them.
 */
export const stateBadges = (root: ParentNode): readonly string[] =>
  [...root.querySelectorAll('.nm-tokens')].map((token) =>
    (token.getAttribute('aria-label') ?? '').trim(),
  );

/**
 * The warnings — the only states still said in words on a row.
 *
 * A freeze, an egress conflict, an endpoint nothing can measure. A warning is never demoted,
 * whatever a layout costs, so these keep their pill while every other state became a mark.
 */
export const warningBadges = (root: ParentNode): readonly string[] =>
  [...root.querySelectorAll('.nm-health, .nm-badge--warn')].map((badge) =>
    badge.textContent.trim(),
  );
