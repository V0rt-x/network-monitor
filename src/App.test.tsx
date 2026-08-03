import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { i18next } from './i18n';
import { App } from './App';

const { registerTrayLabels } = vi.hoisted(() => ({ registerTrayLabels: vi.fn() }));

// The pages themselves are tested where they live; what is under test here is the shell.
vi.mock('./shared/ipc', () => ({ registerTrayLabels }));
vi.mock('./features/network/NetworkPage', () => ({ NetworkPage: () => <p>network page</p> }));
vi.mock('./features/app-monitor/AppMonitorPage', () => ({
  AppMonitorPage: () => <p>applications page</p>,
}));
vi.mock('./features/settings/SettingsPage', () => ({ SettingsPage: () => <p>settings page</p> }));
vi.mock('./features/help/HelpPage', () => ({ HelpPage: () => <p>help page</p> }));

describe('the shell has two exits, not four', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    registerTrayLabels.mockResolvedValue(true);
  });

  it('carries no button in the header at all', () => {
    // `Minimize to tray` duplicated the window's own close button, in the header of every
    // screen; `Quit` had already moved to Settings to get away from it and left with it.
    // What remains are the four tabs — and nothing that ends or hides anything.
    render(<App />);

    const buttons = screen.getAllByRole('button').map((button) => button.textContent);
    expect(buttons).toEqual(['Network', 'Applications', 'Help', 'Settings']);
  });

  it('registers the tray labels on mount, which is what makes closing a hide', () => {
    // Rust starts the tray with an icon and no menu, and treats closing the window as a
    // real quit until the menu exists — an app that vanished into a tray icon with no way
    // back would be worse than one that closed. This call is that menu.
    render(<App />);

    expect(registerTrayLabels).toHaveBeenCalledWith({ show: 'Open', quit: 'Quit' });
  });

  it('registers them again when the language changes', async () => {
    render(<App />);
    registerTrayLabels.mockClear();

    await act(async () => {
      await i18next.changeLanguage('en-GB');
    });

    expect(registerTrayLabels).toHaveBeenCalled();
    await act(async () => {
      await i18next.changeLanguage('en');
    });
  });

  it('offers the four pages as one segmented control', async () => {
    // One container with one background, rather than four bordered, filled boxes competing
    // with each other. What the keyboard sees is unchanged: four stops, and `aria-current`
    // saying which page you are on.
    render(<App />);

    const nav = screen.getByRole('navigation', { name: 'Sections' });
    expect(nav).toHaveClass('nm-nav');
    expect(screen.getByRole('button', { name: 'Network' })).toHaveAttribute('aria-current', 'page');

    await userEvent.click(screen.getByRole('button', { name: 'Applications' }));

    expect(screen.getByText('applications page')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Network' })).not.toHaveAttribute('aria-current');
  });
});
