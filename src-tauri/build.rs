use std::{env, fs, path::PathBuf};

use astrcode_core::env::TAURI_ENV_TARGET_TRIPLE_ENV;

fn resolve_target_triple() -> String {
    env::var(TAURI_ENV_TARGET_TRIPLE_ENV)
        .or_else(|_| env::var("TARGET"))
        .expect("target triple should be available during build")
}

fn ensure_sidecar_placeholder(target_triple: &str) {
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
    let target_triple = resolve_target_triple();
    // 运行期需要知道 cargo tauri build/dev 选择的目标 triple，
    // 这样未安装的开发产物也能回退定位到 sidecar 的真实输出目录。
    println!("cargo:rustc-env=ASTRCODE_DESKTOP_TARGET_TRIPLE={target_triple}");
    ensure_sidecar_placeholder(&target_triple);
    tauri_build::build()
}
