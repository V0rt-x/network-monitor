import type {
  ApplicationListProblem,
  EndpointAgeKindView,
  FlowStatusView,
  LivenessView,
  PathPositionView,
  PathQualityView,
  ProbingView,
  TransportView,
} from '../../shared/ipc';

/**
 * Maps the app-monitor IPC enums to i18next keys.
 *
 * Exhaustive switches on purpose: when Rust gains a variant, `tauri-specta` widens the
 * TypeScript union and these functions stop compiling — which is how a new state gets a
 * translation instead of silently rendering a raw identifier.
 */
export const transportKey = (transport: TransportView) => {
  switch (transport) {
    case 'tcp':
      return 'apps.transport.tcp' as const;
    case 'udp':
      return 'apps.transport.udp' as const;
  }
};

/**
 * What a transport group is called on the page: the transport, and nothing more.
 *
 * They were *Match traffic* and *Supporting connections*, which claim a **role** from a
 * **transport** — the one inference `view.rs` explicitly refuses to draw, on the grounds
 * that everything except the transport and the volume of traffic would be a guess. The names
 * were also simply wrong outside a game: Discord's UDP is voice, a browser's is QUIC, a
 * torrent client's is peers, and several games play over TCP, which made "supporting" a lie
 * on the most important row on the page.
 *
 * *Approved by the user on 2026-08-03, variant A.* The standing rule that level one carries
 * the standard network term applies here literally: UDP and TCP **are** the standard terms,
 * and the plain-language sentence goes where every other one goes — into the label's own
 * explanation, as the `udpFlows` and `tcpConnections` help topics.
 *
 * If a role is ever wanted it has to come from data that knows one — a `kind` on a preset —
 * and a neutral default would still be needed for everything unrecognised, which is this.
 */
export const groupKey = (transport: TransportView) => {
  switch (transport) {
    case 'udp':
      return 'apps.group.udp' as const;
    case 'tcp':
      return 'apps.group.tcp' as const;
  }
};

/** The help topic a group heading explains itself with. */
export const groupTopic = (transport: TransportView) => {
  switch (transport) {
    case 'udp':
      return 'udpFlows' as const;
    case 'tcp':
      return 'tcpConnections' as const;
  }
};

export const livenessKey = (liveness: LivenessView) => {
  switch (liveness) {
    case 'active':
      return 'apps.liveness.active' as const;
    case 'idle':
      return 'apps.liveness.idle' as const;
  }
};

export const probingKey = (probing: ProbingView) => {
  switch (probing) {
    case 'active':
      return 'apps.probing.active' as const;
    case 'demoted':
      return 'apps.probing.demoted' as const;
  }
};

export const flowStatusKey = (status: FlowStatusView) => {
  switch (status) {
    case 'active':
      return 'apps.flow.active' as const;
    case 'notPermitted':
      return 'apps.flow.notPermitted' as const;
    case 'stopped':
      return 'apps.flow.stopped' as const;
    case 'unavailable':
      return 'apps.flow.unavailable' as const;
  }
};

export const pathQualityKey = (quality: PathQualityView) => {
  switch (quality) {
    case 'notMeasuredYet':
      return 'apps.path.quality.notMeasuredYet' as const;
    case 'ok':
      return 'apps.path.quality.ok' as const;
    case 'degraded':
      return 'apps.path.quality.degraded' as const;
    case 'uncorroborated':
      return 'apps.path.quality.uncorroborated' as const;
    case 'lost':
      return 'apps.path.quality.lost' as const;
  }
};

export const pathPositionKey = (position: PathPositionView) => {
  switch (position) {
    case 'reached':
      return 'apps.path.position.reached' as const;
    case 'nothingAnswered':
      return 'apps.path.position.nothingAnswered' as const;
    case 'insideThisNetwork':
      return 'apps.path.position.insideThisNetwork' as const;
    case 'insideTheAccessNetwork':
      return 'apps.path.position.insideTheAccessNetwork' as const;
    case 'beforeAnyLongHaulLink':
      return 'apps.path.position.beforeAnyLongHaulLink' as const;
    case 'beyondALongHaulLink':
      return 'apps.path.position.beyondALongHaulLink' as const;
  }
};

/** How a path verdict is shown, reusing the health palette rather than inventing one. */
export const pathQualityModifier = (quality: PathQualityView) => {
  switch (quality) {
    case 'ok':
      return 'nm-health--ok' as const;
    case 'degraded':
    case 'lost':
      return 'nm-health--degraded' as const;
    case 'uncorroborated':
    case 'notMeasuredYet':
      return 'nm-health--unknown' as const;
  }
};

/**
 * Which of the two ages the header is showing, named at level two.
 *
 * They are different claims: a TCP connection has an establishment the operating system
 * dates, and a UDP endpoint has none, so what can honestly be said there is how long this
 * application has been watched talking to it. Two facts, two words — never one field meaning
 * whichever was available.
 */
export const ageKindKey = (kind: EndpointAgeKindView) => {
  switch (kind) {
    case 'established':
      return 'apps.age.established' as const;
    case 'watched':
      return 'apps.age.watched' as const;
  }
};

export const applicationProblemKey = (problem: ApplicationListProblem) => {
  switch (problem) {
    case 'unsupportedPlatform':
      return 'apps.picker.unsupportedPlatform' as const;
    case 'refused':
      return 'apps.picker.refused' as const;
  }
};
