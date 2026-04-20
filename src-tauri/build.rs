#[path = "src/desktop_frontend_mode.rs"]
mod desktop_frontend_mode;

use std::{env, fs, path::PathBuf};

use desktop_frontend_mode::{
    DesktopFrontendMode, FRONTEND_MODE_ENV, tauri_cli_invoked_from_env, tauri_is_dev_from_env,
};

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
    // 桌面端的前端来源必须在编译期就固定下来，不能留给运行期靠 cfg 或路径猜。
    // 这里显式声明 build script 依赖的环境变量和共享契约文件，避免 cargo 复用
    // 过时的 build.rs 输出，让 plain cargo / tauri dev / packaged 串线。
    println!("cargo:rerun-if-changed=src/desktop_frontend_mode.rs");
    println!(
        "cargo:rerun-if-env-changed={}",
        desktop_frontend_mode::TAURI_CLI_VERBOSITY_ENV
    );
    println!(
        "cargo:rerun-if-env-changed={}",
        desktop_frontend_mode::DEP_TAURI_DEV_ENV
    );
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=TARGET");

    let target_triple = env::var("TARGET").expect("target triple should be available during build");
    let frontend_mode =
        DesktopFrontendMode::resolve(tauri_cli_invoked_from_env(), tauri_is_dev_from_env());
    // 运行期需要知道 cargo tauri build/dev 选择的目标 triple，
    // 这样未安装的开发产物也能回退定位到 sidecar 的真实输出目录。
    println!("cargo:rustc-env=ASTRCODE_DESKTOP_TARGET_TRIPLE={target_triple}");
    println!(
        "cargo:rustc-env={FRONTEND_MODE_ENV}={}",
        frontend_mode.as_env_value()
    );
    ensure_sidecar_placeholder(&target_triple);
    tauri_build::build()
}
