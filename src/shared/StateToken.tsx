import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { healthKey, healthModifier } from '../features/dashboard/labels';
import type { HealthView } from './ipc';

/**
 * Something that changes what the figures beside it *mean*, without being a state of its own.
 *
 * A tunnel makes a round trip end-to-end through it rather than a round trip to the server;
 * a warm-up says the window is not full yet. Neither is a fault and neither is a warning —
 * there is nothing to do about either — so they sit with the state token rather than with the
 * warnings, and like it they are a mark with their name one interaction away.
 */
export type QualifierKind = 'tunnelled' | 'warmup';

export interface Qualifier {
  readonly kind: QualifierKind;
  /** Already translated, because a warm-up carries the seconds left. */
  readonly name: string;
}

interface StateTokenProps {
  readonly health: HealthView;
  readonly qualifiers?: readonly Qualifier[];
}

/**
 * One thing's state, as a mark rather than as a word.
 *
 * On the running build one endpoint's state occupied more width than every figure on the row
 * combined: `OK`, `Warming up · 46 s` and `Through a tunnel`, three bordered pills, on every
 * row of every group. The words were repeating what the reader had already been told at the
 * top of the group and pushing the measurements they came for off to the right.
 *
 * **Shape as well as colour**, always: colour may never be the sole carrier of meaning in this
 * product, so the six states are a circle, a triangle, a square, a diamond, a ring and an
 * outline — distinguishable to a reader who sees no colour at all.
 *
 * **The word is one interaction away and never further.** It is in the accessible name at all
 * times, so a screen reader loses nothing at any level; it appears on hover and on focus for
 * everyone else; and the row's own expander writes it out in full.
 *
 * **One tab stop, not one per mark.** The state and its qualifiers are a single focusable
 * group naming all of them together. Three separate focusable marks per row would be the same
 * mistake 6.7 fixed when it cut two hundred and sixty ⓘ marks down to nine — a table of
 * sixteen endpoints would have carried forty-eight of them between the reader and the next
 * thing they wanted.
 *
 * **What keeps the page honest with the words gone from the row**: the group heading's count
 * chips still say "4 clean, 2 degraded, 1 unreachable" in words at level one, and Rust's
 * worst-first order still puts the bad rows at the top. This is a second channel for a
 * distribution that is already stated, not the only place the state exists.
 */
export const StateToken = ({ health, qualifiers = [] }: StateTokenProps) => {
  const { t } = useTranslation();
  const [shown, setShown] = useState(false);

  const names = [t(healthKey(health)), ...qualifiers.map((qualifier) => qualifier.name)];

  return (
    <span
      className="nm-tokens"
      role="img"
      aria-label={names.join(' · ')}
      tabIndex={0}
      onMouseEnter={() => {
        setShown(true);
      }}
      onMouseLeave={() => {
        setShown(false);
      }}
      onFocus={() => {
        setShown(true);
      }}
      onBlur={() => {
        setShown(false);
      }}
    >
      <span
        className={`nm-token nm-token--${health} ${healthModifier(health)}`}
        aria-hidden="true"
      />
      {qualifiers.map((qualifier) => (
        <span
          key={qualifier.kind}
          className={`nm-token nm-token--qualifier nm-token--${qualifier.kind}`}
          aria-hidden="true"
        />
      ))}
      {shown && (
        <span className="nm-tokens__names" aria-hidden="true">
          {names.join(' · ')}
        </span>
      )}
    </span>
  );
};
