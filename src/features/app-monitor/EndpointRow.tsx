import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { EndpointView } from '../../shared/ipc';
import { useFigures } from '../../shared/useFigures';
import { healthKey, healthModifier } from '../dashboard/labels';
import type { SwatchShape } from './endpointColours';
import { EndpointBadges } from './EndpointBadges';
import { EndpointDetails } from './EndpointDetails';
import { networkName } from './networkName';

interface EndpointRowProps {
  /** The row's own identifier, so a selection on the chart can bring it into view. */
  readonly id: string;
  readonly endpoint: EndpointView;
  /** Span the byte count covers, for the expander's traffic figure. */
  readonly trafficWindowSecs: number;
  /** How many columns the row spans, so its expander can fill the width. */
  readonly columns: number;
  /** The colour this endpoint's line is drawn in, so the row can be tied to it. */
  readonly colour: string;
  /** Its swatch's shape, so the pairing is not carried by colour alone. */
  readonly shape: SwatchShape;
  /** Whether this is the endpoint currently raised on the chart. */
  readonly raised: boolean;
  /** Whether another endpoint is raised, so this one steps back. */
  readonly dimmed: boolean;
  /** Whether the raise is pinned to this endpoint rather than following the cursor. */
  readonly pinned: boolean;
  readonly onPin: () => void;
  readonly onHover: (endpoint: string | null) => void;
}

/**
 * One endpoint, as a row of a table.
 *
 * **The table is what makes the never-merge rule visible.** It was a stack of mini-cards,
 * each carrying its own route panel and traffic panel — seven figures on every silent
 * endpoint, which for a game is six rows of seven. That obeyed the letter of "at most three
 * figures" by splitting them across panels and broke its meaning completely. In a table a
 * blank `Ping` beside a filled `Route` states the rule on every row at once, which is
 * stronger than any paragraph: the round trip we could not measure is *visibly* absent, and
 * the thing standing in for it is *visibly* something else, with the column heading saying
 * which subject it belongs to.
 *
 * That is also why the amended level-one rule allows two subjects here at all: three figures
 * about the endpoint, three about the route to it, **only** because the headings name which
 * is which.
 *
 * *Level one*, in the closed row: the swatch, the address, whose network it is, one word of
 * state, and the four figures. Nothing else — not the probe kind, not the egress adapter, not
 * the hop count, not which kind of age it has, not the traffic volume, not "filtering
 * confirmed", not the averaging window, not where the route stops.
 *
 * *Warnings do not move.* A freeze, an egress conflict, an endpoint nothing can measure stay
 * beside the state whatever the layout costs, because the test for level one is whether there
 * is something to do about it.
 *
 * *Level two* is the row's own expander, and it holds the route and traffic panels in full
 * along with everything that qualifies a figure rather than being one.
 *
 * *Level three* is the column headings, once per table rather than once per row — which is
 * the whole reason the table exists in the shape it does.
 */
export const EndpointRow = ({
  id,
  endpoint,
  trafficWindowSecs,
  columns,
  colour,
  shape,
  raised,
  dimmed,
  pinned,
  onPin,
  onHover,
}: EndpointRowProps) => {
  const { t } = useTranslation();
  const figures = useFigures();
  const [open, setOpen] = useState(false);

  const modifiers = [raised ? 'nm-endpoint--raised' : '', dimmed ? 'nm-endpoint--dimmed' : '']
    .filter(Boolean)
    .join(' ');

  return (
    <>
      <tr
        id={id}
        className={`nm-endpoint ${modifiers}`.trimEnd()}
        onMouseEnter={() => {
          onHover(endpoint.key);
        }}
        onMouseLeave={() => {
          onHover(null);
        }}
      >
        <td className="nm-endpoint__identity">
          <button
            type="button"
            className="nm-endpoint__select"
            aria-pressed={pinned}
            onClick={onPin}
            onFocus={() => {
              onHover(endpoint.key);
            }}
            onBlur={() => {
              onHover(null);
            }}
          >
            {/* Ties the row to its line. Shape as well as colour, because this was the one
                place in the product where colour carried meaning entirely by itself. */}
            <span
              className={`nm-endpoint__swatch nm-endpoint__swatch--${shape}`}
              style={{ backgroundColor: colour }}
              aria-hidden="true"
            />
            <span className="nm-endpoint__address">{endpoint.address}</span>
            <span className="nm-visually-hidden">{t('apps.chart.highlight')}</span>
          </button>
          <span className="nm-endpoint__transport">{endpoint.transport.toUpperCase()}</span>
        </td>

        {/* The only thing on the row a reader can recognise without knowing what a single one
            of the figures means. Absent where the directory is off, still loading, or simply
            does not know — a wrong name is a false statement about where someone's traffic
            went, not a rounding. */}
        <td className="nm-endpoint__network">
          {endpoint.network === null ? '' : networkName(endpoint.network, t)}
        </td>

        <td className="nm-endpoint__state">
          <span className={`nm-health ${healthModifier(endpoint.health)}`}>
            {t(healthKey(endpoint.health))}
          </span>
          <EndpointBadges endpoint={endpoint} />
        </td>

        {/* Absent entirely where no probe will ever fill them in, and absent *silently*.
            Rust draws the line between *not yet* and *never*: a chain still trying kinds
            keeps its dashes, because a figure is coming, while a match server would carry
            them for the whole match. The blank cell beside a filled route is the point. */}
        <td className="nm-endpoint__figure">
          {endpoint.probesMeasureIt ? figures.ms(endpoint.rttMs) : ''}
        </td>
        <td className="nm-endpoint__figure">
          {endpoint.probesMeasureIt ? figures.ms(endpoint.jitterMs) : ''}
        </td>
        <td className="nm-endpoint__figure">
          {endpoint.probesMeasureIt ? figures.pct(endpoint.lossPct) : ''}
        </td>
        <td className="nm-endpoint__figure">
          {endpoint.path === null ? '' : figures.ms(endpoint.path.rttMs)}
        </td>

        <td className="nm-endpoint__disclose">
          <button
            type="button"
            aria-expanded={open}
            aria-controls={`${id}-details`}
            onClick={() => {
              setOpen((current) => !current);
            }}
          >
            <span className="nm-visually-hidden">
              {t('apps.details.row', { endpoint: endpoint.address })}
            </span>
            <span aria-hidden="true">{open ? '▾' : '▸'}</span>
          </button>
        </td>
      </tr>

      {open && (
        <tr className="nm-endpoint__detailrow" id={`${id}-details`}>
          <td colSpan={columns}>
            <EndpointDetails endpoint={endpoint} trafficWindowSecs={trafficWindowSecs} withPanels />
          </td>
        </tr>
      )}
    </>
  );
};
