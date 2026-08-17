fn main() {
    // Copy workspace release binaries for sidecar bundling.
    // Must run BEFORE tauri_build::build(), which validates that the
    // externalBin resources exist (resolved as `{entry}-{target_triple}`).
    let out = std::path::Path::new("binaries");
    let _ = std::fs::create_dir_all(out);
    // build.rs lives at apps/desktop/src-tauri, workspace root is three levels up
    let workspace_target = std::path::Path::new("../../../target/release");
    let target_triple = std::env::var("TARGET").unwrap_or_default();
    for name in ["fan-files", "fan-files-share"] {
        let bin = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
        let src = workspace_target.join(&bin);
        let dst = out.join(format!("{bin}-{target_triple}"));
        if src.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }

    tauri_build::build();
}
