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
    { key: 'game.exe', label: 'game.exe', seedPid: 200, pids: [200] },
    { key: 'svchost.exe', label: 'svchost.exe', seedPid: 300, pids: [300, 301] },
  ],
  problem: null,
  ...overrides,
});

const nothing = new Map<number, MonitoredBy>();

const picker = (props: Partial<Parameters<typeof ApplicationPicker>[0]> = {}) => (
  <ApplicationPicker
    monitored={nothing}
    count={0}
    limit={5}
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

  it('offers applications rather than a row per process', async () => {
    // The failure this exists to fix: six identical Discord.exe rows, and the user asked to
    // pick one arbitrarily when what they want is Discord.
    render(picker());

    expect(await screen.findByText('Discord')).toBeInTheDocument();
    expect(screen.getByText('6 processes')).toBeInTheDocument();
    expect(screen.queryByText('Discord.exe')).not.toBeInTheDocument();
  });

  it('names the process identifier only where there is exactly one', async () => {
    render(picker());
    await screen.findByText('Discord');

    expect(screen.getByText('PID 200')).toBeInTheDocument();
    expect(screen.getByText('2 processes')).toBeInTheDocument();
  });

  it('filters by name as the user searches', async () => {
    render(picker());
    await screen.findByText('Discord');

    await userEvent.type(screen.getByRole('searchbox'), 'game');

    expect(screen.getByText('game.exe')).toBeInTheDocument();
    expect(screen.queryByText('Discord')).not.toBeInTheDocument();
  });

  it('seeds an application from the process Rust chose, not one the user picked', async () => {
    const onMonitor = vi.fn();
    render(picker({ onMonitor }));
    await screen.findByText('Discord');

    const [first] = screen.getAllByRole('button', { name: 'Monitor' });
    if (!first) throw new Error('the picker rendered no monitorable application');
    await userEvent.click(first);

    expect(onMonitor).toHaveBeenCalledWith(100);
  });

  it('marks an application any of whose processes is already monitored', async () => {
    // The picker's grouping and the monitor's need not agree exactly — the monitor also
    // adopts descendants — so an overlap anywhere means the application is taken.
    const monitored = new Map<number, MonitoredBy>([[104, { app: 7, name: 'Discord' }]]);
    render(picker({ monitored, count: 1 }));
    await screen.findByText('Discord');

    expect(screen.getByText('Part of Discord')).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: 'Monitor' })).toHaveLength(2);
  });

  it('stops the application rather than the process', async () => {
    const onForget = vi.fn();
    const monitored = new Map<number, MonitoredBy>([[200, { app: 7, name: 'game.exe' }]]);
    render(picker({ monitored, count: 1, onForget }));
    await screen.findByText('game.exe');

    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));

    expect(onForget).toHaveBeenCalledWith(7);
  });

  it('refuses further choices at the cap rather than letting a click fail silently', async () => {
    const monitored = new Map<number, MonitoredBy>([[200, { app: 7, name: 'game.exe' }]]);
    render(picker({ monitored, count: 1, limit: 1 }));
    await screen.findByText('Discord');

    for (const button of screen.getAllByRole('button', { name: 'Monitor' })) {
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
    render(picker({ monitored, count: 1, limit: 2 }));
    await screen.findByText('Discord');

    for (const button of screen.getAllByRole('button', { name: 'Monitor' })) {
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
    render(picker());
    await screen.findByText('Discord');

    await userEvent.type(screen.getByRole('searchbox'), 'nothing-like-this');

    expect(screen.getByText('No application matches that name')).toBeInTheDocument();
  });

  it('re-reads the list when asked, rather than polling for it', async () => {
    render(picker());
    await screen.findByText('Discord');
    expect(fetchApplications).toHaveBeenCalledTimes(1);

    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));

    expect(fetchApplications).toHaveBeenCalledTimes(2);
  });
});
