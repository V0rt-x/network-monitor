import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import type { MonitoredBy } from './ApplicationPicker';
import { ApplicationPicker } from './ApplicationPicker';
import type { ApplicationListView } from '../../shared/ipc';

const { fetchApplications } = vi.hoisted(() => ({ fetchApplications: vi.fn() }));

vi.mock('../../shared/ipc', () => ({ fetchApplications }));

const listing = (overrides: Partial<ApplicationListView> = {}): ApplicationListView => ({
  applications: [
    // Six processes, one application — the shape that made the raw process list unusable.
    {
      key: 'discord',
      label: 'Discord',
      seedPid: 100,
      pids: [100, 101, 102, 103, 104, 105],
    },
    {
      key: 'example-game',
      label: 'Example Game',
      seedPid: 200,
      pids: [200],
    },
  ],
  problem: null,
  ...overrides,
});

const nothing = new Map<number, MonitoredBy>();

const picker = (props: Partial<Parameters<typeof ApplicationPicker>[0]> = {}) => (
  <ApplicationPicker
    monitored={nothing}
    watching={[]}
    limit={5}
    // Open is the state most of these tests are about; the folded line has its own.
    open
    onOpenChange={vi.fn()}
    onMonitor={vi.fn()}
    onForget={vi.fn()}
    {...props}
  />
);

describe('ApplicationPicker', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    fetchApplications.mockResolvedValue(listing());
  });

  it('names an application and never its file, and never a process identifier', async () => {
    // A player picks *Discord*, and `Discord.exe` beside it and `PID 100` beside that are
    // three restatements of a fact they did not ask for — the product's implementation
    // showing through. Rust does not send either any more, so there is nothing to render.
    render(picker());

    expect(await screen.findByText('Discord')).toBeInTheDocument();
    expect(screen.getByText('Example Game')).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/PID|\.exe/);
  });

  it('offers applications rather than a row per process, with the count as a chip', async () => {
    // The failure this exists to fix: six identical Discord.exe rows, and the user asked to
    // pick one arbitrarily when what they want is Discord. The count is what says how large
    // a group the rule caught — a chip, not a sentence.
    render(picker());
    await screen.findByText('Discord');

    const entry = screen.getByText('Discord').closest('.nm-picker__entry');
    expect(entry?.querySelector('.nm-count')?.textContent.trim()).toBe('6 processes');
    // One action per entry, in the same place down the list. It was a wrapping flex line, so
    // the button landed under a different word on every row.
    expect(entry?.querySelectorAll('button')).toHaveLength(1);
  });

  it('filters by name as the user searches', async () => {
    render(picker());
    await screen.findByText('Discord');

    await userEvent.type(screen.getByRole('searchbox'), 'game');

    expect(screen.getByText('Example Game')).toBeInTheDocument();
    expect(screen.queryByText('Discord')).not.toBeInTheDocument();
  });

  it('seeds an application from the process Rust chose, not one the user picked', async () => {
    const onMonitor = vi.fn();
    render(picker({ onMonitor }));
    await screen.findByText('Discord');

    const [first] = screen.getAllByRole('button', { name: 'Watch' });
    if (!first) throw new Error('the picker rendered no monitorable application');
    await userEvent.click(first);

    expect(onMonitor).toHaveBeenCalledWith(100);
  });

  it('marks an application any of whose processes is already monitored', async () => {
    // The picker's grouping and the monitor's need not agree exactly — the monitor also
    // adopts descendants — so an overlap anywhere means the application is taken.
    const monitored = new Map<number, MonitoredBy>([[104, { app: 7, name: 'Discord' }]]);
    render(picker({ monitored, watching: ['Example Game'] }));
    await screen.findByText('Discord');

    expect(screen.getByText('Part of Discord')).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: 'Watch' })).toHaveLength(1);
  });

  it('stops the application rather than the process', async () => {
    const onForget = vi.fn();
    const monitored = new Map<number, MonitoredBy>([[200, { app: 7, name: 'Example Game' }]]);
    render(picker({ monitored, watching: ['Example Game'], onForget }));
    await screen.findByText('Example Game');

    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));

    expect(onForget).toHaveBeenCalledWith(7);
  });

  it('refuses further choices at the cap rather than letting a click fail silently', async () => {
    const monitored = new Map<number, MonitoredBy>([[200, { app: 7, name: 'Example Game' }]]);
    render(picker({ monitored, watching: ['Example Game'], limit: 1 }));
    await screen.findByText('Discord');

    for (const button of screen.getAllByRole('button', { name: 'Watch' })) {
      expect(button).toBeDisabled();
    }
    // The one already chosen can still be released.
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();
  });

  it('counts applications rather than processes against the cap', async () => {
    // One application holds several processes; measuring the cap in processes would refuse
    // a second game to an Electron app that happens to run six helpers.
    const monitored = new Map<number, MonitoredBy>(
      [100, 101, 102, 103, 104, 105].map((pid) => [pid, { app: 7, name: 'Discord' }]),
    );
    render(picker({ monitored, watching: ['Example Game'], limit: 2 }));
    await screen.findByText('Discord');

    for (const button of screen.getAllByRole('button', { name: 'Watch' })) {
      expect(button).toBeEnabled();
    }
  });

  it('reports a refused enumeration instead of an empty list', async () => {
    fetchApplications.mockResolvedValue(listing({ applications: [], problem: 'refused' }));
    render(picker());

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The system refused to list running processes',
    );
  });

  it('reports a platform with no process enumerator at all', async () => {
    fetchApplications.mockResolvedValue(
      listing({ applications: [], problem: 'unsupportedPlatform' }),
    );
    render(picker());

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'This build cannot list applications on this operating system',
    );
  });

  it('says a search matched nothing rather than looking broken', async () => {
    // One answer now, and it is the plain one. There used to be a second — "turn on Show
    // everything running" — because the list was filtered in the UI and the filter could be
    // the reason. Rust sends only what it can name, so there is nothing left to look behind.
    render(picker());
    await screen.findByText('Discord');

    await userEvent.type(screen.getByRole('searchbox'), 'nothing-like-this');

    expect(screen.getByText('No application matches that name')).toBeInTheDocument();
    expect(screen.queryByRole('checkbox')).toBeNull();
  });

  it('re-reads the list when asked, rather than polling for it', async () => {
    render(picker());
    await screen.findByText('Discord');
    expect(fetchApplications).toHaveBeenCalledTimes(1);

    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));

    expect(fetchApplications).toHaveBeenCalledTimes(2);
  });
});
