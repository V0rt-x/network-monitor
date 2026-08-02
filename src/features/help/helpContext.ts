import { createContext, useContext } from 'react';

/** Opens the bundled help at one topic's own section. */
export type OpenHelp = (topic: HelpTopic) => void;

import type { HelpTopic } from './topics';

/**
 * How a metric's ⓘ reaches the help page.
 *
 * A context rather than a prop threaded through five layers of endpoint rendering: which
 * page is shown is the shell's business, and every row on every page wants the same door.
 *
 * The default does nothing, which is what a component rendered outside the shell — in a
 * test, most often — should do. A missing provider must not be an exception thrown in the
 * middle of a measurement the user is reading.
 */
export const HelpContext = createContext<OpenHelp>(() => undefined);

export const useHelp = (): OpenHelp => useContext(HelpContext);
