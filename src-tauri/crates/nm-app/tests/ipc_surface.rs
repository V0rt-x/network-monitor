//! Tests for the IPC contract between the Rust core and the UI.

use nm_app::{CoreReadiness, CoreStatus, PlatformKind};
use nm_platform::HostPlatform;

/// Where the generated TypeScript IPC bindings live.
///
/// Anchored to the crate directory rather than the working directory so the output
/// lands in the checkout however the test was launched.
const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../src/bindings.ts");

/// Regenerates `src/bindings.ts` from the live IPC surface.
///
/// Generation lives in a test rather than in `run()` so the shipped app never writes to
/// the source tree, and so CI can prove the committed bindings still match the Rust
/// types: it runs the suite and then fails if the working tree became dirty.
#[test]
fn exports_typescript_bindings() {
    nm_app::ipc_builder()
        .export(specta_typescript::Typescript::default(), BINDINGS_PATH)
        .expect("exporting TypeScript bindings must succeed");
}

#[test]
fn reports_ready_on_a_supported_platform() {
    assert_eq!(
        CoreStatus::describe("0.4.2", Ok(HostPlatform::Windows)),
        CoreStatus {
            core_version: "0.4.2".to_owned(),
            platform: PlatformKind::Windows,
            readiness: CoreReadiness::Ready,
        }
    );
}

#[test]
fn maps_every_supported_platform() {
    for (host, expected) in [
        (HostPlatform::Windows, PlatformKind::Windows),
        (HostPlatform::Linux, PlatformKind::Linux),
        (HostPlatform::MacOs, PlatformKind::MacOs),
    ] {
        let status = CoreStatus::describe("0.1.0", Ok(host));
        assert_eq!(status.platform, expected);
        assert_eq!(status.readiness, CoreReadiness::Ready);
    }
}

#[test]
fn degrades_honestly_on_an_unsupported_platform() {
    let status = CoreStatus::describe("0.4.2", Err(nm_platform::Error::UnsupportedPlatform));
    assert_eq!(status.platform, PlatformKind::Unsupported);
    assert_eq!(status.readiness, CoreReadiness::UnsupportedPlatform);
    // Facts we do know must survive a degraded state instead of being blanked out.
    assert_eq!(status.core_version, "0.4.2");
}

#[test]
fn the_live_command_reports_this_hosts_platform() {
    let status = nm_app::core_status();
    assert_eq!(status.core_version, nm_core::VERSION);
    assert_eq!(status.readiness, CoreReadiness::Ready);
    assert_ne!(status.platform, PlatformKind::Unsupported);
}
