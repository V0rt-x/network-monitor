/**
 * Everything the page will explain, and the only things it can explain.
 *
 * A label's own explanation and the help page read from the same list, so a topic can never
 * have a disclosure with no section behind it or a section nothing points at. The identifiers
 * are stable — they are the anchors the "Learn more" links jump to — and every string they
 * name lives in `common.json` under `help.topic.<id>`, so Russian stays additive.
 *
 * **A topic belongs to a section, and the list *is* the sections.** Twenty-eight topics in one
 * flat run, each opening with a hundred and fifty to three hundred and fifty words, is six
 * thousand words with no way in. Deriving the flat list from the grouping rather than keeping
 * two lists in step makes "every topic has a section" true by construction, exactly as
 * "every topic has a page" already was.
 */
export const HELP_SECTIONS = [
  {
    id: 'startHere',
    // First, and deliberately: without it the honest answer looks like a wrong one.
    topics: ['ping', 'watching'],
  },
  {
    id: 'figures',
    // The figures carry the standard network terms, because the audience has met those words
    // in every other tool they have used; the plain-language sentence lives here rather than
    // in a name invented for the reader. Where two figures on one card would otherwise both
    // be "jitter", the passive one is qualified — `arrivalJitter` — never left ambiguous.
    topics: [
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
      'age',
    ],
  },
  {
    id: 'measured',
    // Not figures, but the reasons figures mean what they mean: which probe produced one,
    // which adapter it left by, whether a tunnel is in the way, whose network answered, and
    // what the two transport groups are.
    topics: [
      'probeKind',
      'egress',
      'tunnel',
      'network',
      'chart',
      'warmup',
      'passive',
      'pool',
      'udpFlows',
      'tcpConnections',
    ],
  },
  {
    id: 'verdicts',
    // The one place in the application that states a conclusion had no explanation of any
    // kind, which by the standing rule that a metric is not done without a help topic was a
    // debt rather than an omission.
    topics: ['verdict', 'verdictEvidence'],
  },
  {
    id: 'services',
    // The status page's own figures. Its round trips are the same quantity as the ones above
    // and still need topics of their own: what distinguishes them is *which* checks each one
    // folds in, and that is exactly what a reader averaging a strip by eye has got wrong.
    topics: ['checks', 'medianRtt', 'latestCheck', 'meanRtt'],
  },
] as const;

/** One thing the page can explain. */
export type HelpTopic = (typeof HELP_SECTIONS)[number]['topics'][number];

/** One part of the help page. */
export type HelpSection = (typeof HELP_SECTIONS)[number]['id'];

/** Every topic, in reading order. Derived, so a topic cannot exist outside a section. */
export const HELP_TOPICS: readonly HelpTopic[] = HELP_SECTIONS.flatMap(
  (section) => section.topics as readonly HelpTopic[],
);

/** The one-or-two plain sentences shown in place, on hover and on focus. */
export const shortKey = (topic: HelpTopic) => `help.topic.${topic}.short` as const;

/** What the topic is called, in the disclosure and as its heading in the help page. */
export const titleKey = (topic: HelpTopic) => `help.topic.${topic}.title` as const;

/** The full explanation, behind "Read more". */
export const bodyKey = (topic: HelpTopic) => `help.topic.${topic}.body` as const;

/** What a section is called. */
export const sectionKey = (section: HelpSection) => `help.section.${section}` as const;

/** The anchor a "Learn more" jumps to. */
export const anchorOf = (topic: HelpTopic) => `nm-help-${topic}` as const;

/** The anchor a contents entry jumps to. */
export const sectionAnchorOf = (section: HelpSection) => `nm-help-section-${section}` as const;

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
