//! Helpers for turning monotonic durations into the integer units the UI displays.
//!
//! All measurement timing uses [`std::time::Instant`]; wall-clock time is only ever used
//! for display and persistence. These conversions are deliberately saturating rather
//! than wrapping: an absurd duration must not become a small, plausible-looking number.
//!
//! Widths are chosen so the value survives the IPC boundary intact — JavaScript numbers
//! lose precision above 2^53, so nothing wider than [`u32`] crosses it.

use std::time::Duration;

/// Whole seconds in `elapsed`, saturating at [`u32::MAX`] (~136 years).
#[must_use]
pub fn elapsed_secs(elapsed: Duration) -> u32 {
    u32::try_from(elapsed.as_secs()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_whole_seconds() {
        assert_eq!(elapsed_secs(Duration::ZERO), 0);
        assert_eq!(elapsed_secs(Duration::from_secs(3_600)), 3_600);
    }

    #[test]
    fn truncates_sub_second_remainders() {
        assert_eq!(elapsed_secs(Duration::from_millis(999)), 0);
        assert_eq!(elapsed_secs(Duration::from_millis(1_999)), 1);
    }

    #[test]
    fn saturates_instead_of_wrapping() {
        assert_eq!(elapsed_secs(Duration::MAX), u32::MAX);
        // The first duration that no longer fits in u32 seconds.
        let overflowing = Duration::from_secs(u64::from(u32::MAX) + 1);
        assert_eq!(elapsed_secs(overflowing), u32::MAX);
    }
}
