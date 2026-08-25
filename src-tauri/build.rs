fn main() {
    #[cfg(target_os = "windows")]
    {
        let attributes = tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        tauri_build::try_build(attributes).expect("failed to run Tauri build script");
        embed_windows_manifest_for_all_targets();
    }

    #[cfg(not(target_os = "windows"))]
    tauri_build::build()
}

/// Tauri's default resource arguments attach the Common Controls v6 manifest
/// only to the application binary. Unit-test executables also link dialog code,
/// so they must receive the same manifest or Windows exits before the Rust test
/// harness starts with STATUS_ENTRYPOINT_NOT_FOUND.
#[cfg(target_os = "windows")]
fn embed_windows_manifest_for_all_targets() {
    let manifest = std::env::current_dir()
        .expect("current source directory")
        .join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    println!("cargo:rustc-link-arg=/WX");
}
