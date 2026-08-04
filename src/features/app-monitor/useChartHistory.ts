import { useCallback, useEffect, useState } from 'react';

import { fetchChartHistory, type ChartHistoryView } from '../../shared/ipc';

const NOTHING: ChartHistoryView = { elapsedSecs: [], endpoints: [] };

/**
 * The hour behind the two minutes the event pushes.
 *
 * **Fetched, never pushed**, and that is the whole shape of the deeper chart. The
 * `AppEndpoints` event carries every endpoint's last forty slots on every emission; twenty
 * minutes at the enforced ceiling would be four hundred slots × two series × sixteen
 * endpoints × five applications, once a second, into a `WebView` with a 1 % CPU budget. So the
 * depth is asked for instead — three times in a typical session:
 *
 * * when the card mounts, because that is when a reader first has a chart to scroll;
 * * when the window is shown again after being hidden, because **a window that was hidden
 *   must leave no gap**: Rust kept measuring and emitted nothing, and a hole the UI created by
 *   not listening would read as packets that did not come back, which is the exact failure the
 *   three-second slot was introduced to prevent;
 * * and when the reader asks for it, which is what `refresh` is.
 *
 * A failure is silence rather than an error state: the chart falls back to the live window,
 * which is what it drew before any of this existed, and the reader loses depth rather than
 * their measurements.
 */
export const useChartHistory = (
  app: number,
): { readonly history: ChartHistoryView; readonly refresh: () => void } => {
  const [history, setHistory] = useState<ChartHistoryView>(NOTHING);

  const load = useCallback(() => {
    let active = true;
    void fetchChartHistory(app).then(
      (fetched) => {
        if (active) setHistory(fetched);
      },
      () => {
        // Nothing to say to the reader: the live window is still on screen and still true.
      },
    );
    return () => {
      active = false;
    };
  }, [app]);

  useEffect(() => load(), [load]);

  // The window coming back is the one moment the backfill is not optional, and it is the one
  // moment nothing else would trigger it: an emission arrives immediately on show, but it
  // carries the same forty slots it always does.
  useEffect(() => {
    const onVisible = () => {
      if (!document.hidden) load();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [load]);

  return { history, refresh: load };
};
