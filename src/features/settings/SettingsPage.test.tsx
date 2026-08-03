import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { SettingsPage } from './SettingsPage';
import type { Settings, SettingsView } from '../../shared/ipc';

const { fetchSettings, storeSettings, quitApp } = vi.hoisted(() => ({
  fetchSettings: vi.fn(),
  storeSettings: vi.fn(),
  quitApp: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({ fetchSettings, storeSettings, quitApp }));

const view = (overrides: Partial<SettingsView> = {}): SettingsView => ({
  settings: {
    language: 'en',
    country: 'ru',
    baselineIntervalSecs: 5,
    autostart: false,
    rememberGameServers: true,
    nameNetworks: true,
  },
  problem: null,
  countries: ['ru', 'ir'],
  minIntervalSecs: 1,
  maxIntervalSecs: 60,
  networkSnapshot: '2026-08-03',
  ...overrides,
});

describe('SettingsPage', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    fetchSettings.mockResolvedValue(view());
    // Stands in for Rust: whatever comes back is what the page must show, which is why
    // every assertion below reads the reply rather than the request.
    storeSettings.mockImplementation((settings: Settings) => Promise.resolve(view({ settings })));
  });

  it('offers exactly the countries Rust has lists for', async () => {
    render(<SettingsPage />);

    const select = await screen.findByLabelText('Country');
    const options = Array.from((select as HTMLSelectElement).options).map((o) => o.value);
    expect(options).toEqual(['ru', 'ir']);
  });

  it('sends a country change to Rust', async () => {
    render(<SettingsPage />);
    const select = await screen.findByLabelText('Country');

    await userEvent.selectOptions(select, 'ir');

    expect(storeSettings).toHaveBeenCalledWith({
      language: 'en',
      country: 'ir',
      baselineIntervalSecs: 5,
      autostart: false,
      rememberGameServers: true,
      nameNetworks: true,
    });
  });

  it('takes the interval bounds from Rust rather than hard-coding them', async () => {
    fetchSettings.mockResolvedValue(view({ minIntervalSecs: 2, maxIntervalSecs: 30 }));
    render(<SettingsPage />);

    const slider = await screen.findByLabelText('Baseline probe interval: 5 s');
    expect(slider).toHaveAttribute('min', '2');
    expect(slider).toHaveAttribute('max', '30');
  });

  it('shows what Rust applied, not what was requested', async () => {
    // The platform decides whether autostart really took effect; echoing the request
    // would claim something about the machine that may not be true.
    storeSettings.mockResolvedValue(
      view({
        settings: {
          language: 'en',
          country: 'ru',
          baselineIntervalSecs: 5,
          autostart: false,
          rememberGameServers: true,
          nameNetworks: true,
        },
      }),
    );
    render(<SettingsPage />);

    const toggle = await screen.findByLabelText('Start with the system');
    await userEvent.click(toggle);

    await waitFor(() => {
      expect(storeSettings).toHaveBeenCalled();
    });
    expect(toggle).not.toBeChecked();
  });

  it('starts with autostart off', async () => {
    render(<SettingsPage />);
    expect(await screen.findByLabelText('Start with the system')).not.toBeChecked();
  });

  it('surfaces a settings file that could not be understood', async () => {
    fetchSettings.mockResolvedValue(view({ problem: 'malformed' }));
    render(<SettingsPage />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The settings file could not be understood',
    );
  });

  it('reports an unreachable core instead of an empty form', async () => {
    fetchSettings.mockRejectedValue(new Error('ipc down'));
    render(<SettingsPage />);

    expect(await screen.findByRole('alert')).toHaveTextContent('Settings could not be reached');
  });

  it('offers the network directory as a switch, and says what it costs', async () => {
    render(<SettingsPage />);

    const toggle = await screen.findByLabelText('Name the networks behind addresses');
    expect(toggle).toBeChecked();
    // The one setting here that buys memory rather than behaviour, so the figure is stated.
    expect(screen.getByText(/12 MB/)).toBeVisible();
    // A directory is a photograph of one day, and this is what explains a name that has
    // since gone stale.
    expect(screen.getByText(/Snapshot of 2026-08-03/)).toBeVisible();

    await userEvent.click(toggle);

    expect(storeSettings).toHaveBeenCalledWith(expect.objectContaining({ nameNetworks: false }));
  });
});

describe('SettingsPage, ending the monitoring', () => {
  it('is where quitting lives, away from the button that only hides the window', async () => {
    // They sat side by side in the header, told apart by the muted colour of the second:
    // one hides the window and the other ends the monitoring. Settings is reachable from
    // every page, so the guarantee that a user is never stuck survives the move.
    fetchSettings.mockResolvedValue(view());
    render(<SettingsPage />);
    await screen.findByLabelText('Language');

    await userEvent.click(screen.getByRole('button', { name: 'Quit' }));

    expect(quitApp).toHaveBeenCalled();
  });
});
