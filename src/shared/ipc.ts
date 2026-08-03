import type { UnlistenFn } from '@tauri-apps/api/event';

import { commands, events } from '../bindings';
import type {
  AppEndpoints,
  ApplicationListView,
  CoreHeartbeat,
  CoreStatus,
  NetworkSnapshot,
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
  AppEndpoints,
  AppProcessView,
  ApplicationChoiceView,
  ApplicationListProblem,
  ApplicationListView,
  AppView,
  CheckMarkView,
  CheckView,
  CoreHeartbeat,
  CoreReadiness,
  CoreStatus,
  DiagnosisView,
  EndpointAgeKindView,
  EndpointAgeView,
  EndpointGroupView,
  EndpointView,
  FlowStatusView,
  PassiveRttView,
  FlowView,
  HealthCountsView,
  HealthView,
  LivenessView,
  NetworkRowView,
  NetworkSectionView,
  NetworkSnapshot,
  NetworkView,
  PathPositionView,
  PathQualityView,
  PathView,
  PlatformKind,
  PoolView,
  ProbeKindView,
  ProbingView,
  RowEndpointView,
  Section,
  Settings,
  SettingsProblem,
  SettingsView,
  TransportView,
  TrayLabels,
  VerdictView,
} from '../bindings';

export const fetchCoreStatus = (): Promise<CoreStatus> => commands.coreStatus();

export const subscribeToHeartbeat = (onBeat: (beat: CoreHeartbeat) => void): Promise<UnlistenFn> =>
  events.coreHeartbeat.listen((event) => {
    onBeat(event.payload);
  });

export const subscribeToNetwork = (
  onSnapshot: (snapshot: NetworkSnapshot) => void,
): Promise<UnlistenFn> =>
  events.networkSnapshot.listen((event) => {
    onSnapshot(event.payload);
  });

export const subscribeToAppEndpoints = (
  onEndpoints: (endpoints: AppEndpoints) => void,
): Promise<UnlistenFn> =>
  events.appEndpoints.listen((event) => {
    onEndpoints(event.payload);
  });

/** The applications the picker may offer, grouped by Rust rather than a raw process list. */
export const fetchApplications = (): Promise<ApplicationListView> => commands.listApplications();

/** Starts monitoring the application a running process belongs to; the pid is the seed. */
export const monitorApp = (pid: number): Promise<void> => commands.monitorApp(pid);

/** Stops monitoring one application, by the identity its endpoints are reported under. */
export const forgetApp = (app: number): Promise<void> => commands.forgetApp(app);

export const fetchSettings = (): Promise<SettingsView> => commands.getSettings();

export const storeSettings = (settings: Settings): Promise<SettingsView> =>
  commands.setSettings(settings);

export const registerTrayLabels = (labels: TrayLabels): Promise<boolean> =>
  commands.applyTrayLabels(labels);
