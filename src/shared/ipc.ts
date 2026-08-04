import type { UnlistenFn } from '@tauri-apps/api/event';

import { commands, events } from '../bindings';
import type {
  AppEndpoints,
  ApplicationListView,
  ChartHistoryView,
  CoreHeartbeat,
  CoreStatus,
  NetworkCatalogueView,
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
  ApplicationChoiceView,
  ChartHistoryEntryView,
  ChartHistoryView,
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
  NetworkCatalogueEntryView,
  NetworkCatalogueView,
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

/**
 * The bundled catalogue an edit chooser may offer over the Network page's editable groups.
 *
 * Over the bundled list only — there is no free-text address field anywhere in this product.
 * `domestic` and `foreign` never appear: they are the verdict's own evidence, not a user's
 * services, and stay probed whether or not a selection includes them.
 */
export const fetchNetworkCatalogue = (): Promise<NetworkCatalogueView> =>
  commands.networkCatalogue();

/**
 * One application's chart history, as far back as the hour Rust keeps.
 *
 * **Fetched, never pushed.** The event carries the last forty slots at the emission rate, so
 * the steady-state cost of a running session is unchanged; the depth behind them is asked for
 * a handful of times a session — when a card mounts, when the window is shown again, when the
 * reader scrolls past what they hold — and pushing it would spend the whole render budget
 * answering a question asked rarely.
 *
 * A core that has stopped answers with nothing rather than an error: the chart then shows the
 * live window alone, which is what it showed before this existed.
 */
export const fetchChartHistory = async (app: number): Promise<ChartHistoryView> => {
  const answer = await commands.chartHistory(app);
  return answer.status === 'ok' ? answer.data : { elapsedSecs: [], endpoints: [] };
};
