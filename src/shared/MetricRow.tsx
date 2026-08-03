import type { ReactNode } from 'react';

/** One figure: what it is called, and what it reads. */
export interface Metric {
  /** Stable within the row. */
  readonly key: string;
  /** The label, which is also where the explanation hangs. */
  readonly label: ReactNode;
  /** Already written down with its unit, or the dash. */
  readonly value: ReactNode;
}

interface MetricRowProps {
  readonly metrics: readonly Metric[];
  /** Larger figures, for the one place on a card that leads with a number. */
  readonly size?: 'default' | 'headline';
}

/**
 * The same three quantities, laid out the same way, wherever they appear.
 *
 * There were two layouts for one thing: a rigid `repeat(3, 1fr)` grid on the services page
 * and a wrapping flex row on the applications page. The consequence was that ping, jitter and
 * loss did not line up with each other between two pages of the same product — which is the
 * sort of thing a reader registers as *wrong* long before they could say what it was.
 *
 * A definition list rather than a table, because that is what it is: a set of name-value
 * pairs about one subject. The tabular figures and the reserved line height are what stop a
 * value alternating between a number and a dash from moving everything below it.
 */
export const MetricRow = ({ metrics, size = 'default' }: MetricRowProps) => (
  <dl className={size === 'headline' ? 'nm-metrics nm-metrics--headline' : 'nm-metrics'}>
    {metrics.map((metric) => (
      <div key={metric.key}>
        <dt>{metric.label}</dt>
        <dd>{metric.value}</dd>
      </div>
    ))}
  </dl>
);
