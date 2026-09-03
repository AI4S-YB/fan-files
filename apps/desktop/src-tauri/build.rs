fn main() {
    // Copy workspace release binaries for sidecar bundling.
    // Must run BEFORE tauri_build::build(), which validates that the
    // externalBin resources exist (resolved as `{entry}-{target_triple}{extension}`).
    let out = std::path::Path::new("binaries");
    let _ = std::fs::create_dir_all(out);
    // build.rs lives at apps/desktop/src-tauri, workspace root is three levels up
    let workspace_target = std::path::Path::new("../../../target/release");
    let target_triple = std::env::var("TARGET").unwrap_or_default();
    // CARGO_CFG_TARGET_OS reflects the build *target*, unlike cfg!(windows) which
    // reflects the host and would silently skip the copy when cross-compiling.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let extension = if target_os == "windows" { ".exe" } else { "" };
    for name in ["fan-files", "fan-files-share"] {
        let src = workspace_target.join(format!("{name}{extension}"));
        // Re-run this build script whenever the workspace sidecar binary changes,
        // otherwise cargo caches the copy and the bundle ships a stale sidecar.
        println!("cargo:rerun-if-changed={}", src.display());
        // tauri-utils resolves externalBin entries as `{entry}-{target_triple}{extension}`
        // (extension comes AFTER the triple), so the copy must match that layout.
        let dst = out.join(format!("{name}-{target_triple}{extension}"));
        if src.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }

    tauri_build::build();
}
