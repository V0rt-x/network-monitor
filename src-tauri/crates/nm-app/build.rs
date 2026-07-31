fn main() {
    tauri_build::build();

    // See `tests.manifest`: test harnesses link the same Windows libraries as the app
    // but do not get the manifest `tauri-build` embeds into the binary target.
    #[cfg(windows)]
    {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests.manifest");
        println!("cargo:rerun-if-changed=tests.manifest");
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
