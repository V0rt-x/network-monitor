import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import { platformKey, readinessKey } from './labels';
import { useCoreHeartbeat } from './useCoreHeartbeat';
import { useCoreStatus } from './useCoreStatus';

/**
 * Phase 0's proof that the whole pipeline works end to end: Rust computes the facts, the
 * typed IPC surface carries them, i18next renders them. The panel holds no logic of its
 * own beyond choosing which translated state to show.
 */
export const CoreStatusPanel = () => {
  const { t, i18n } = useTranslation();
  const status = useCoreStatus();
  const heartbeat = useCoreHeartbeat();

  const numberFormat = useMemo(() => new Intl.NumberFormat(i18n.language), [i18n.language]);

  return (
    <section className="nm-panel">
      <h2 className="nm-panel__heading">{t('core.heading')}</h2>

      {status.kind === 'loading' && <p className="nm-state--pending">{t('core.loading')}</p>}

      {status.kind === 'unreachable' && (
        <p className="nm-state--degraded" role="alert">
          {t('core.unreachable')}
        </p>
      )}

      {status.kind === 'ready' && (
        <dl className="nm-facts">
          <dt>{t('core.versionLabel')}</dt>
          <dd>{status.status.coreVersion}</dd>

          <dt>{t('core.platformLabel')}</dt>
          <dd
            className={status.status.platform === 'unsupported' ? 'nm-state--degraded' : undefined}
          >
            {t(platformKey(status.status.platform))} — {t(readinessKey(status.status.readiness))}
          </dd>

          <dt>{t('heartbeat.label')}</dt>
          <dd>
            {heartbeat.kind === 'beating' && (
              <>
                {t('heartbeat.uptime', {
                  seconds: numberFormat.format(heartbeat.beat.uptimeSecs),
                })}{' '}
                <span className="nm-state--pending">
                  {t('heartbeat.tickLabel', { seq: numberFormat.format(heartbeat.beat.seq) })}
                </span>
              </>
            )}
            {heartbeat.kind === 'waiting' && (
              <span className="nm-state--pending">{t('heartbeat.waiting')}</span>
            )}
            {heartbeat.kind === 'unavailable' && (
              <span className="nm-state--degraded">{t('heartbeat.unavailable')}</span>
            )}
          </dd>
        </dl>
      )}
    </section>
  );
};
