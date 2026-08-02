import type { ReactNode } from 'react';

import { HelpContext } from './helpContext';
import type { OpenHelp } from './helpContext';

/** Supplies the door from every metric's ⓘ to the help page. */
export const HelpProvider = ({
  openHelp,
  children,
}: {
  readonly openHelp: OpenHelp;
  readonly children: ReactNode;
}) => <HelpContext.Provider value={openHelp}>{children}</HelpContext.Provider>;
