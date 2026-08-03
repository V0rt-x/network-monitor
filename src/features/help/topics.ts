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
  // Not a figure either, and the one label on the page a reader can recognise without
  // knowing what any of the numbers mean. It carries the caveats the page must not: that
  // the directory is a snapshot, and that a registration country is not a location.
  'network',
  // Not a figure but a reason figures change, and the one the reader is owed most: with a
  // VPN or proxy client running, this badge is on nearly every endpoint on the page, and
  // it is why their round trips are measured a different way from everyone else's.
  'tunnel',
  // The status page's own figures. Its round trips are the same quantity as the ones above
  // and still need topics of their own: what distinguishes them is *which* checks each one
  // folds in, and that is exactly what a reader averaging a strip by eye has got wrong.
  'checks',
  'medianRtt',
  'latestCheck',
  'meanRtt',
  // The prose the applications page used to carry. Each of these was a paragraph competing
  // with the figures beside it; what a reader needs on the page is a name and a number, and
  // what a reader who *asks* needs is here.
  'age',
  'chart',
  'watching',
  'warmup',
  'passive',
  'pool',
  // The two group headings. They used to carry a sentence apiece on the page — "the
  // connections a game actually plays over" — which claimed a *role* from a *transport*,
  // the one inference the view layer refuses to draw, and was wrong for a browser, a voice
  // call and any game that plays over TCP. The heading now says the transport and the
  // explanation says the rest.
  'udpFlows',
  'tcpConnections',
] as const;

/** One thing the page can explain. */
export type HelpTopic = (typeof HELP_TOPICS)[number];

/** The one-or-two plain sentences shown in place, on hover and on focus. */
export const shortKey = (topic: HelpTopic) => `help.topic.${topic}.short` as const;

/** What the topic is called, in the tooltip and as its heading in the help page. */
export const titleKey = (topic: HelpTopic) => `help.topic.${topic}.title` as const;

/** The full explanation, in the help page. */
export const bodyKey = (topic: HelpTopic) => `help.topic.${topic}.body` as const;

/**
 * A body split into the paragraphs its author wrote.
 *
 * The bodies are written with blank lines in them and were rendered into a single `<p>`,
 * where every line break collapses to a space — so the longest topic shipped as one
 * unbroken block of ~350 words. The split lives here, beside the keys, so every surface
 * that ever renders a body gets the same paragraphs.
 */
export const paragraphsOf = (body: string): readonly string[] =>
  body
    .split(/\n\s*\n/)
    .map((paragraph) => paragraph.trim())
    .filter((paragraph) => paragraph.length > 0);

/** The anchor a "Learn more" jumps to. */
export const anchorOf = (topic: HelpTopic) => `nm-help-${topic}` as const;
