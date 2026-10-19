//! Build script for Universal MCP Manager.
//!
//! On Windows this script owns the application manifest for *every* target of
//! the package: the GUI binary and all test harnesses (unit tests of the lib
//! target plus the integration tests in `tests/`).
//!
//! Why: `cargo test` links the full GUI stack (`tauri` -> `tao`/`muda` and
//! `rfd`), which statically imports `TaskDialogIndirect`. That export only
//! exists in Common Controls v6; without a manifest the Windows loader binds
//! the legacy `comctl32` v5.82 and every test process dies at startup with
//! 0xC0000139 STATUS_ENTRYPOINT_NOT_FOUND.
//!
//! `tauri-build` embeds its manifest only into *binary* targets (it emits
//! `cargo:rustc-link-arg-bins` via `embed-resource`), and Cargo cannot attach
//! link arguments to the unit-test harness of the `[lib]` target at all. The
//! only reliable approach is to disable `tauri-build`'s manifest and pass our
//! own to the MSVC linker for the whole package with plain
//! `cargo:rustc-link-arg` (which covers bins, tests, benches, examples and the
//! lib test harness). Embedding it exactly once per binary also avoids the
//! CVT1100 duplicate-resource error that occurred when both `tauri-build` and
//! this script injected an RT_MANIFEST resource into the app binary.

fn main() {
    // `tauri-build` still compiles the Windows resource section (icon, version
    // info, product metadata); we only ask it to leave the manifest out.
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attributes).expect("failed to run tauri-build");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("windows")
            .join("app.manifest");
        assert!(
            manifest.is_file(),
            "missing Windows manifest: {}",
            manifest.display()
        );
        println!("cargo:rerun-if-changed=windows/app.manifest");
        // `/MANIFEST:EMBED` puts the manifest in the linked image; the path
        // must be absolute because the linker runs with the target directory
        // as its working directory.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
