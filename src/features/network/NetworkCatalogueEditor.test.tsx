import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import type { NetworkCatalogueView, Settings, SettingsView } from '../../shared/ipc';
import { NetworkCatalogueEditor } from './NetworkCatalogueEditor';

const { fetchNetworkCatalogue, fetchSettings, storeSettings } = vi.hoisted(() => ({
  fetchNetworkCatalogue: vi.fn<() => Promise<NetworkCatalogueView>>(),
  fetchSettings: vi.fn<() => Promise<SettingsView>>(),
  storeSettings: vi.fn<(settings: Settings) => Promise<SettingsView>>(),
}));

vi.mock('../../shared/ipc', () => ({ fetchNetworkCatalogue, fetchSettings, storeSettings }));

const catalogue = (overrides: Partial<NetworkCatalogueView> = {}): NetworkCatalogueView => ({
  entries: [
    { key: 'services/ea', label: 'EA', section: 'gamingPlatform' },
    { key: 'services/battle-net', label: 'Battle.net', section: 'gamingPlatform' },
    { key: 'services/aws', label: 'Amazon Web Services', section: 'infrastructure' },
  ],
  ...overrides,
});

const settings = (overrides: Partial<Settings> = {}): Settings => ({
  language: 'en',
  country: 'ru',
  baselineIntervalSecs: 5,
  autostart: false,
  rememberGameServers: true,
  nameNetworks: true,
  networkSelection: null,
  ...overrides,
});

const view = (overrides: Partial<SettingsView> = {}): SettingsView => ({
  settings: settings(),
  problem: null,
  countries: ['ru'],
  minIntervalSecs: 1,
  maxIntervalSecs: 60,
  networkSnapshot: '2026-08-04',
  ...overrides,
});

const editor = (open = true) => <NetworkCatalogueEditor open={open} onOpenChange={vi.fn()} />;

describe('NetworkCatalogueEditor', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    fetchNetworkCatalogue.mockResolvedValue(catalogue());
    fetchSettings.mockResolvedValue(view());
    storeSettings.mockImplementation((next: Settings) => Promise.resolve(view({ settings: next })));
  });

  it('is a single button when folded, with no checklist on screen', () => {
    render(editor(false));

    expect(screen.getByRole('button', { name: 'Edit…' })).toBeInTheDocument();
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
  });

  it('opens onto a checklist grouped by section, in the order the page shows tiles', async () => {
    render(editor());

    expect(await screen.findByText('EA')).toBeInTheDocument();
    const groups = (await screen.findAllByRole('group')).map(
      (group) => group.querySelector('legend')?.textContent,
    );
    expect(groups).toEqual(['Gaming platforms', 'Infrastructure']);
  });

  it('draws no heading for a section with nothing bundled in it', async () => {
    // `Other` exists in the schema for an entry that fits neither bundled group; until a
    // release adds one, an empty heading here would be a finding about the page rather than
    // the honest "there is nothing to tick yet".
    render(editor());
    await screen.findByText('EA');

    expect(screen.queryByText('Other')).not.toBeInTheDocument();
  });

  it('ticks everything when the selection is null, meaning "everything bundled"', async () => {
    render(editor());

    const boxes = await screen.findAllByRole('checkbox');
    expect(boxes.every((box) => (box as HTMLInputElement).checked)).toBe(true);
  });

  it('ticks only the entries a stored selection names', async () => {
    fetchSettings.mockResolvedValue(
      view({ settings: settings({ networkSelection: ['services/ea'] }) }),
    );
    render(editor());

    const ea = await screen.findByRole('checkbox', { name: 'EA' });
    const battleNet = screen.getByRole('checkbox', { name: 'Battle.net' });
    expect(ea).toBeChecked();
    expect(battleNet).not.toBeChecked();
  });

  it('unticking an entry stores everything else by name, not by removing null', async () => {
    render(editor());
    const ea = await screen.findByRole('checkbox', { name: 'EA' });

    await userEvent.click(ea);

    expect(storeSettings).toHaveBeenCalledTimes(1);
    const sent = storeSettings.mock.calls[0]?.[0];
    expect(sent?.networkSelection).toEqual(
      expect.arrayContaining(['services/battle-net', 'services/aws']),
    );
    expect(sent?.networkSelection).not.toContain('services/ea');
    expect(sent?.networkSelection).toHaveLength(2);
  });

  it('collapses back to null once every bundled entry is ticked again', async () => {
    // A listed-out array of today's keys would silently exclude an entry a later release
    // adds; `null` is what keeps that entry visible with no settings migration.
    fetchSettings.mockResolvedValue(
      view({ settings: settings({ networkSelection: ['services/battle-net', 'services/aws'] }) }),
    );
    render(editor());
    const ea = await screen.findByRole('checkbox', { name: 'EA' });
    expect(ea).not.toBeChecked();

    await userEvent.click(ea);

    expect(storeSettings).toHaveBeenCalledWith(expect.objectContaining({ networkSelection: null }));
  });

  it('reports when the catalogue could not be requested', async () => {
    fetchNetworkCatalogue.mockRejectedValue(new Error('no channel'));
    render(editor());

    expect(await screen.findByText('The catalogue could not be requested')).toBeInTheDocument();
  });
});
