import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { ProcessPicker } from './ProcessPicker';
import type { ProcessListView } from '../../shared/ipc';

const { fetchProcesses } = vi.hoisted(() => ({ fetchProcesses: vi.fn() }));

vi.mock('../../shared/ipc', () => ({ fetchProcesses }));

const listing = (overrides: Partial<ProcessListView> = {}): ProcessListView => ({
  processes: [
    { pid: 100, name: 'Discord.exe' },
    { pid: 200, name: 'game.exe' },
    { pid: 300, name: 'svchost.exe' },
  ],
  problem: null,
  ...overrides,
});

describe('ProcessPicker', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    fetchProcesses.mockResolvedValue(listing());
  });

  it('lists the running processes with their identifiers', async () => {
    render(<ProcessPicker monitored={[]} limit={5} onMonitor={vi.fn()} onForget={vi.fn()} />);

    expect(await screen.findByText('Discord.exe')).toBeInTheDocument();
    expect(screen.getByText('PID 100')).toBeInTheDocument();
    expect(screen.getByText('svchost.exe')).toBeInTheDocument();
  });

  it('filters by name as the user searches', async () => {
    render(<ProcessPicker monitored={[]} limit={5} onMonitor={vi.fn()} onForget={vi.fn()} />);
    await screen.findByText('Discord.exe');

    await userEvent.type(screen.getByRole('searchbox'), 'game');

    expect(screen.getByText('game.exe')).toBeInTheDocument();
    expect(screen.queryByText('Discord.exe')).not.toBeInTheDocument();
  });

  it('starts monitoring the process the user chooses', async () => {
    const onMonitor = vi.fn();
    render(<ProcessPicker monitored={[]} limit={5} onMonitor={onMonitor} onForget={vi.fn()} />);
    await screen.findByText('Discord.exe');

    // The first entry is Discord.exe — the list is sorted by name in Rust.
    const [first] = screen.getAllByRole('button', { name: 'Monitor' });
    if (!first) throw new Error('the picker rendered no monitorable process');
    await userEvent.click(first);

    expect(onMonitor).toHaveBeenCalledWith(100);
  });

  it('offers to stop a process that is already followed', async () => {
    const onForget = vi.fn();
    render(<ProcessPicker monitored={[200]} limit={5} onMonitor={vi.fn()} onForget={onForget} />);
    await screen.findByText('game.exe');

    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));

    expect(onForget).toHaveBeenCalledWith(200);
  });

  it('refuses further choices at the cap rather than letting a click fail silently', async () => {
    render(<ProcessPicker monitored={[200]} limit={1} onMonitor={vi.fn()} onForget={vi.fn()} />);
    await screen.findByText('Discord.exe');

    for (const button of screen.getAllByRole('button', { name: 'Monitor' })) {
      expect(button).toBeDisabled();
    }
    // The one already chosen can still be released.
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();
  });

  it('reports a refused enumeration instead of an empty list', async () => {
    fetchProcesses.mockResolvedValue(listing({ processes: [], problem: 'refused' }));
    render(<ProcessPicker monitored={[]} limit={5} onMonitor={vi.fn()} onForget={vi.fn()} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The system refused to list running processes',
    );
  });

  it('reports a platform with no process enumerator at all', async () => {
    fetchProcesses.mockResolvedValue(listing({ processes: [], problem: 'unsupportedPlatform' }));
    render(<ProcessPicker monitored={[]} limit={5} onMonitor={vi.fn()} onForget={vi.fn()} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'This build cannot list processes on this operating system',
    );
  });

  it('says a search matched nothing rather than looking broken', async () => {
    render(<ProcessPicker monitored={[]} limit={5} onMonitor={vi.fn()} onForget={vi.fn()} />);
    await screen.findByText('Discord.exe');

    await userEvent.type(screen.getByRole('searchbox'), 'nothing-like-this');

    expect(screen.getByText('No process matches that name')).toBeInTheDocument();
  });

  it('re-reads the list when asked, rather than polling for it', async () => {
    render(<ProcessPicker monitored={[]} limit={5} onMonitor={vi.fn()} onForget={vi.fn()} />);
    await screen.findByText('Discord.exe');
    expect(fetchProcesses).toHaveBeenCalledTimes(1);

    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));

    expect(fetchProcesses).toHaveBeenCalledTimes(2);
  });
});
