/**
 * Everything the page will explain, and the only things it can explain.
 *
 * A metric's ⓘ and the help page read from the same list, so a topic can never have a
 * tooltip with no section behind it or a section nothing points at. The identifiers are
 * stable — they are the anchors the "Learn more" links jump to — and every string they name
 * lives in `common.json` under `help.topic.<id>`, so Russian stays additive.
 */
export const HELP_TOPICS = [
  // First, and deliberately: without it the honest answer looks like a wrong one.
  'ping',
  'response',
  'stability',
  'loss',
  'route',
  'updates',
  'smoothness',
  'worstPause',
  'dropOff',
  'freeze',
  'traffic',
  'probeKind',
  'egress',
] as const;

/** One thing the page can explain. */
export type HelpTopic = (typeof HELP_TOPICS)[number];

/** The one-or-two plain sentences shown in place, on hover and on focus. */
export const shortKey = (topic: HelpTopic) => `help.topic.${topic}.short` as const;

/** What the topic is called, in the tooltip and as its heading in the help page. */
export const titleKey = (topic: HelpTopic) => `help.topic.${topic}.title` as const;

/** The full explanation, in the help page. */
export const bodyKey = (topic: HelpTopic) => `help.topic.${topic}.body` as const;

/** The anchor a "Learn more" jumps to. */
export const anchorOf = (topic: HelpTopic) => `nm-help-${topic}` as const;
