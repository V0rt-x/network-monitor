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
  // The figures carry the standard network terms, because the audience has met those words in
  // every other tool they have used; the plain-language sentence lives here, in the ⓘ, rather
  // than in a name invented for the reader. Where two figures on one card would otherwise both
  // be "jitter", the passive one is qualified — `arrivalJitter` — never left ambiguous.
  'rtt',
  'jitter',
  'loss',
  'route',
  'updates',
  'arrivalJitter',
  'worstPause',
  'dropOff',
  'freeze',
  'traffic',
  'probeKind',
  'egress',
  // The status page's own figures. Its round trips are the same quantity as the ones above
  // and still need topics of their own: what distinguishes them is *which* checks each one
  // folds in, and that is exactly what a reader averaging a strip by eye has got wrong.
  'checks',
  'serviceRtt',
  'latestCheck',
  'meanRtt',
  // The prose the applications page used to carry. Each of these was a paragraph competing
  // with the figures beside it; what a reader needs on the page is a name and a number, and
  // what a reader who *asks* needs is here.
  'chart',
  'watching',
  'warmup',
  'passive',
  'pool',
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
