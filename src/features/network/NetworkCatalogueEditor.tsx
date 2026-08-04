import { useTranslation } from 'react-i18next';

import type { Section } from '../../shared/ipc';
import { useSettings } from '../settings/useSettings';
import { sectionKey } from './labels';
import { useNetworkCatalogue } from './useNetworkCatalogue';

interface NetworkCatalogueEditorProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}

/** The three sections this chooser may offer, in the order the page shows their tiles. */
const EDITABLE_SECTIONS: readonly Section[] = ['gamingPlatform', 'infrastructure', 'other'];

/**
 * The page is edited, not fixed: a checklist over the **bundled catalogue**, nothing more.
 *
 * **No free-text address field exists here or anywhere in this product.** An address a user
 * typed would be a target this app then probed on their behalf, and the bundled lists are
 * auditable precisely because they are the only thing that is ever probed — restricting the
 * chooser to ticking bundled entries is a privacy decision as much as a scope one.
 *
 * The selection persists with the settings, debounced exactly as every other setting is —
 * this component only ever calls the same `change` every field on the Settings page calls,
 * and Rust is what coalesces a burst of clicks into one write.
 *
 * **Unticking an entry really stops it being probed**, which is what lets ticking fewer buy
 * the rest a shorter cadence: `Domestic` and `Foreign` never appear here at all, because they
 * are the verdict's own evidence and stay measured regardless of what this chooser says.
 */
export const NetworkCatalogueEditor = ({ open, onOpenChange }: NetworkCatalogueEditorProps) => {
  const { t } = useTranslation();
  const catalogue = useNetworkCatalogue();
  const { state, change } = useSettings();

  if (!open) {
    return (
      <button
        type="button"
        className="nm-button nm-button--quiet"
        onClick={() => {
          onOpenChange(true);
        }}
      >
        {t('network.edit.open')}
      </button>
    );
  }

  if (catalogue.kind === 'loading' || state.kind === 'loading') {
    return <p className="nm-state--pending">{t('network.edit.loading')}</p>;
  }
  if (catalogue.kind === 'unavailable' || state.kind === 'unavailable') {
    return (
      <p className="nm-state--degraded" role="alert">
        {t('network.edit.unavailable')}
      </p>
    );
  }

  const { entries } = catalogue.catalogue;
  const { networkSelection } = state.view.settings;
  const isSelected = (key: string) => networkSelection === null || networkSelection.includes(key);

  const toggle = (key: string) => {
    const every = entries.map((entry) => entry.key);
    const current = new Set(networkSelection ?? every);
    if (current.has(key)) current.delete(key);
    else current.add(key);
    // Collapsing back to `null` once everything is ticked again is what keeps a later
    // release's new entry visible with no settings migration — the same reason `null` is
    // the default rather than a listed-out array of today's keys.
    const next = every.every((candidate) => current.has(candidate)) ? null : [...current];
    change({ networkSelection: next });
  };

  return (
    <section className="nm-editcatalogue">
      <header className="nm-editcatalogue__header">
        <h3 className="nm-editcatalogue__title">{t('network.edit.heading')}</h3>
        <button
          type="button"
          className="nm-button"
          onClick={() => {
            onOpenChange(false);
          }}
        >
          {t('network.edit.done')}
        </button>
      </header>
      <p className="nm-editcatalogue__hint">{t('network.edit.hint')}</p>

      {EDITABLE_SECTIONS.map((section) => {
        const members = entries.filter((entry) => entry.section === section);
        // For item 4's reason, reused: an empty group heading is a finding about the page,
        // not about the catalogue.
        if (members.length === 0) return null;
        return (
          <fieldset key={section} className="nm-editcatalogue__group">
            <legend className="nm-editcatalogue__legend">{t(sectionKey(section))}</legend>
            <ul className="nm-editcatalogue__list">
              {members.map((entry) => (
                <li key={entry.key} className="nm-editcatalogue__entry">
                  <label>
                    <input
                      type="checkbox"
                      checked={isSelected(entry.key)}
                      onChange={() => {
                        toggle(entry.key);
                      }}
                    />
                    {entry.label}
                  </label>
                </li>
              ))}
            </ul>
          </fieldset>
        );
      })}
    </section>
  );
};
