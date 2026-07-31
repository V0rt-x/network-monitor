import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { SettingsPage } from './SettingsPage';
import type { Settings, SettingsView } from '../../shared/ipc';

const { fetchSettings, storeSettings } = vi.hoisted(() => ({
  fetchSettings: vi.fn(),
  storeSettings: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({ fetchSettings, storeSettings }));

const view = (overrides: Partial<SettingsView> = {}): SettingsView => ({
  settings: { language: 'en', country: 'ru', baselineIntervalSecs: 5, autostart: false },
  problem: null,
  countries: ['ru', 'ir'],
  minIntervalSecs: 1,
  maxIntervalSecs: 60,
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
        settings: { language: 'en', country: 'ru', baselineIntervalSecs: 5, autostart: false },
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
});
