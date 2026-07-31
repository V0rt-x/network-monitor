//! Error type for [`crate`].

use thiserror::Error as ThisError;

/// Failures raised by the probe engine.
#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// The OS abstraction layer could not carry out a probe.
    #[error(transparent)]
    Platform(#[from] nm_platform::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_platform_failures_transparently() {
        let error = Error::from(nm_platform::Error::UnsupportedPlatform);

        assert!(matches!(
            error,
            Error::Platform(nm_platform::Error::UnsupportedPlatform)
        ));
        // `#[error(transparent)]` must forward the inner message verbatim: the probe
        // engine adds no wording of its own on top of an OS-layer failure.
        assert_eq!(
            error.to_string(),
            nm_platform::Error::UnsupportedPlatform.to_string()
        );
    }
}
