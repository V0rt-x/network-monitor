import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import '../../i18n';
import type { ChartLine } from './chartSeries';
import { EndpointChart } from './EndpointChart';

/**
 * jsdom implements no canvas, so uPlot cannot draw here at all — it fails inside an animation
 * frame rather than at construction, which leaves nothing to assert against and a stream of
 * errors from a library that is not what these tests are about.
 *
 * What they *are* about is everything around the drawing: the tooltip that is also the
 * legend, the keyboard, and what selecting an entry does. None of it goes through the canvas,
 * which is what makes it testable at all — and what the chart looked like was checked by eye
 * on a real render, because no headless renderer can answer that question.
 */
vi.mock('uplot', () => {
  class FakePlot {
    readonly cursor = { idx: null, left: 0 };
    readonly over = { offsetLeft: 0 };
    setData() {
      /* the data is asserted through the tooltip, not through the canvas */
    }
    redraw() {
      /* nothing is drawn */
    }
    destroy() {
      /* nothing to tear down */
    }
    valToPos() {
      return 0;
    }
    static rangeLog() {
      return [1, 100] as const;
    }
  }
  return { default: FakePlot };
});

const line = (overrides: Partial<ChartLine> = {}): ChartLine => ({
  endpoint: 'udp/1.1.1.1:27015',
  address: '1.1.1.1:27015',
  transport: 'udp',
  label: '1.1.1.1:27015',
  values: [24, 26, 25],
  colour: '#58a6ff',
  isPath: false,
  ...overrides,
});

const chart = (props: Partial<Parameters<typeof EndpointChart>[0]> = {}) => (
  <EndpointChart
    elapsedSecs={[0, 3, 6]}
    lines={[line()]}
    highlighted={null}
    onHover={vi.fn()}
    onSelect={vi.fn()}
    label="Round-trip time over time"
    {...props}
  />
);

/** Puts the chart on a moment, the way a keyboard reader would. */
const readNewest = async () => {
  const surface = screen.getByRole('img');
  surface.focus();
  await userEvent.keyboard('{End}');
};

describe('EndpointChart', () => {
  it('lists every line at the moment being read, not only the nearest one', async () => {
    // The chart's stated job is "which of these is the odd one out", and at a given second
    // that is a question about all of them at once — so the tooltip is the legend the chart
    // never had rather than a label for whatever the pointer happened to be near.
    render(
      chart({
        lines: [
          line(),
          line({
            endpoint: 'udp/1.1.1.2:27015',
            address: '1.1.1.2:27015',
            label: '1.1.1.2:27015',
            values: [80, 91, 84],
          }),
        ],
      }),
    );

    await readNewest();

    const entries = within(screen.getByRole('list')).getAllByRole('button');
    expect(entries).toHaveLength(2);
    // Worst first, which is the order the list beside the chart is already in.
    expect(entries[0]).toHaveTextContent('1.1.1.2:27015 · ping 84 ms');
    expect(entries[1]).toHaveTextContent('1.1.1.1:27015 · ping 25 ms');
  });

  it('names a route as a route and never as a ping', async () => {
    // The never-merge rule, applied to the tooltip. A route is a round trip to a router short
    // of the endpoint, and calling it a ping here would undo everything the dashed line and
    // the row's own labels do.
    render(
      chart({
        lines: [
          line({
            label: 'Route to 1.1.1.1:27015',
            values: [80, 91, 84],
            isPath: true,
          }),
        ],
      }),
    );

    await readNewest();

    const entry = within(screen.getByRole('list')).getByRole('button');
    expect(entry).toHaveTextContent('1.1.1.1:27015 · route 84 ms');
    expect(entry.textContent).not.toMatch(/ping/i);
  });

  it('reads a slot with nothing in it as no reply, never as zero and never by omission', async () => {
    // A silently missing entry is indistinguishable from a line that is doing fine, and a
    // zero is a measurement that did not happen.
    render(
      chart({
        lines: [
          line({ values: [24, 26, null] }),
          line({
            endpoint: 'udp/1.1.1.2:27015',
            address: '1.1.1.2:27015',
            label: '1.1.1.2:27015',
            values: [80, 91, 84],
          }),
        ],
      }),
    );

    await readNewest();

    const entries = within(screen.getByRole('list')).getAllByRole('button');
    expect(entries).toHaveLength(2);
    expect(entries[1]).toHaveTextContent('1.1.1.1:27015 · no reply');
    expect(entries[1]?.textContent).not.toContain('0 ms');
  });

  it('says nothing about health, leaving the list the only authority on it', async () => {
    render(chart());

    await readNewest();

    const tooltip = screen.getByRole('list');
    expect(tooltip.textContent).not.toMatch(/OK|Degraded|Unreachable|blocked/i);
  });

  it('moves through time and between lines from the keyboard, and it is one tab stop', async () => {
    // The check strip on the services page — a far less important surface — already did this
    // properly, and the chart was inert: `role="img"` with a static label, no focus, no
    // arrows, nothing read out.
    const onHover = vi.fn();
    render(
      chart({
        onHover,
        lines: [
          line(),
          line({
            endpoint: 'udp/1.1.1.2:27015',
            address: '1.1.1.2:27015',
            label: '1.1.1.2:27015',
            values: [80, 91, 84],
          }),
        ],
      }),
    );

    const surface = screen.getByRole('img');
    expect(surface).toHaveAttribute('tabindex', '0');

    surface.focus();
    await userEvent.keyboard('{End}');
    expect(screen.getByRole('status')).toHaveTextContent('At 0:06');

    await userEvent.keyboard('{ArrowLeft}');
    expect(screen.getByRole('status')).toHaveTextContent('At 0:03');

    await userEvent.keyboard('{ArrowDown}');
    expect(onHover).toHaveBeenLastCalledWith('udp/1.1.1.2:27015');

    // And Escape lets go of it entirely.
    await userEvent.keyboard('{Escape}');
    expect(screen.getByRole('status')).toHaveTextContent('');
  });

  it('reads out every line the tooltip shows', async () => {
    render(
      chart({
        lines: [
          line(),
          line({
            endpoint: 'udp/1.1.1.2:27015',
            address: '1.1.1.2:27015',
            label: '1.1.1.2:27015',
            values: [80, 91, null],
          }),
        ],
      }),
    );

    await readNewest();

    const readout = screen.getByRole('status').textContent;
    expect(readout).toContain('1.1.1.1:27015 · ping 25 ms');
    expect(readout).toContain('1.1.1.2:27015 · no reply');
  });

  it('pins the endpoint that was chosen, not the one the pointer was nearest', async () => {
    const onSelect = vi.fn();
    render(
      chart({
        onSelect,
        lines: [
          line(),
          line({
            endpoint: 'udp/1.1.1.2:27015',
            address: '1.1.1.2:27015',
            label: '1.1.1.2:27015',
            values: [80, 91, 84],
          }),
        ],
      }),
    );

    await readNewest();
    // The first entry is the worst one, which is the *second* line.
    const [worst] = within(screen.getByRole('list')).getAllByRole('button');
    if (!worst) throw new Error('the tooltip listed no entries');
    await userEvent.click(worst);

    expect(onSelect).toHaveBeenCalledWith('udp/1.1.1.2:27015');
  });
});
