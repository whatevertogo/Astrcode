use std::{env, fs, path::PathBuf};

use astrcode_core::env::TAURI_ENV_TARGET_TRIPLE_ENV;

fn ensure_sidecar_placeholder() {
    let target_triple = env::var(TAURI_ENV_TARGET_TRIPLE_ENV)
        .or_else(|_| env::var("TARGET"))
        .expect("target triple should be available during build");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let file_name = if target_os == "windows" {
        format!("astrcode-server-{target_triple}.exe")
    } else {
        format!("astrcode-server-{target_triple}")
    };

    let binaries_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    let placeholder = binaries_dir.join(file_name);
    if placeholder.exists() {
        return;
    }

    fs::create_dir_all(&binaries_dir).expect("sidecar binaries directory should be creatable");
    fs::write(
        &placeholder,
        b"astrcode-server sidecar placeholder; scripts/tauri-frontend.js overwrites this before tauri dev/build\n",
    )
    .expect("sidecar placeholder should be writable");
}

fn main() {
    ensure_sidecar_placeholder();
    tauri_build::build()
}
