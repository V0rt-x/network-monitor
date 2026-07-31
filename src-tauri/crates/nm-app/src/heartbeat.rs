//! Emits the liveness event on the app's tokio runtime.
//!
//! It is the one signal that separates "nothing has been measured yet" from "the core is
//! gone", which is why it survives past the phase that introduced it. Like every other
//! stream it stops at the `WebView`'s door while the window is hidden: a beat nobody sees
//! is a wake-up nobody asked for.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Wry};
use tauri_specta::Event as _;

use crate::events::CoreHeartbeat;

/// How often the liveness event is emitted.
///
/// Well inside the ≤ 4 Hz IPC budget, and driven by a monotonic timer so a wall-clock
/// adjustment cannot make it burst.
pub(crate) const PERIOD: Duration = Duration::from_secs(1);

/// Spawns the heartbeat task; it ends when emitting fails, i.e. when the app shuts down.
pub fn spawn(app: AppHandle<Wry>, visible: Arc<AtomicBool>) {
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let mut ticker = tokio::time::interval(PERIOD);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut seq: u32 = 0;

        loop {
            ticker.tick().await;
            seq = seq.wrapping_add(1);
            if !visible.load(Ordering::Relaxed) {
                // The counter keeps running, so the uptime the window sees on its return
                // is the real one rather than one that paused with it.
                continue;
            }
            let event = CoreHeartbeat {
                seq,
                uptime_secs: nm_core::time::elapsed_secs(started.elapsed()),
            };
            if event.emit(&app).is_err() {
                break;
            }
        }
    });
}
