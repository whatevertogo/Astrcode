use std::path::PathBuf;

use crate::paths::default_config_path;

#[tauri::command]
pub fn minimize_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn maximize_window(window: tauri::WebviewWindow) -> Result<(), String> {
    let is_maximized = window.is_maximized().map_err(|error| error.to_string())?;
    if is_maximized {
        window.unmaximize().map_err(|error| error.to_string())
    } else {
        window.maximize().map_err(|error| error.to_string())
    }
}

#[tauri::command]
pub fn close_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn select_directory() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("选择工作目录")
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_config_in_editor(path: Option<String>) -> Result<(), String> {
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => default_config_path().map_err(|e| e.to_string())?,
    };

    // 安全验证：只允许打开 ~/.astrcode/ 下的文件
    let allowed_root = crate::paths::astrcode_root_dir().map_err(|e| e.to_string())?;

    // 规范化路径并验证是否在允许的根目录下
    let canonical_path =
        std::fs::canonicalize(&path).map_err(|e| format!("无法访问路径: {}", e))?;
    let canonical_root = std::fs::canonicalize(&allowed_root).unwrap_or(allowed_root);

    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "安全限制：只能打开 ~/.astrcode/ 目录下的文件\n请求的路径: {}",
            path.display()
        ));
    }

    open::that(path).map_err(|error| error.to_string())
}
