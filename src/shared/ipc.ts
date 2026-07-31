import type { UnlistenFn } from '@tauri-apps/api/event';

import { commands, events } from '../bindings';
import type {
  CoreHeartbeat,
  CoreStatus,
  NetworkHealth,
  Settings,
  SettingsView,
  TrayLabels,
} from '../bindings';

/**
 * The single place the UI touches the generated bindings.
 *
 * `src/bindings.ts` is produced from the Rust IPC surface by tauri-specta and must never
 * be edited or mirrored by hand; funnelling access through this module keeps the rest of
 * the app insulated from the generator's output shape.
 */
export type {
  BaselineGroup,
  CoreHeartbeat,
  CoreReadiness,
  CoreStatus,
  GroupView,
  HealthCountsView,
  HealthView,
  NetworkHealth,
  PlatformKind,
  ProbeKindView,
  Settings,
  SettingsProblem,
  SettingsView,
  TargetView,
  TrayLabels,
} from '../bindings';

export const fetchCoreStatus = (): Promise<CoreStatus> => commands.coreStatus();

export const subscribeToHeartbeat = (onBeat: (beat: CoreHeartbeat) => void): Promise<UnlistenFn> =>
  events.coreHeartbeat.listen((event) => {
    onBeat(event.payload);
  });

export const subscribeToNetworkHealth = (
  onHealth: (health: NetworkHealth) => void,
): Promise<UnlistenFn> =>
  events.networkHealth.listen((event) => {
    onHealth(event.payload);
  });

export const fetchSettings = (): Promise<SettingsView> => commands.getSettings();

export const storeSettings = (settings: Settings): Promise<SettingsView> =>
  commands.setSettings(settings);

export const registerTrayLabels = (labels: TrayLabels): Promise<boolean> =>
  commands.applyTrayLabels(labels);

export const hideToTray = (): Promise<void> => commands.hideToTray();

export const quitApp = (): Promise<void> => commands.quitApp();
