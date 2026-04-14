use std::{path::PathBuf, sync::mpsc::sync_channel, time::Duration};

use tauri::{Emitter, Manager};

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

#[cfg(debug_assertions)]
#[tauri::command]
pub fn open_debug_workbench(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::ServerState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("debug-workbench") {
        if let Some(next_session_id) = session_id
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            window
                .emit("debug-workbench:set-session", next_session_id.to_string())
                .map_err(|error| error.to_string())?;
        }
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app_handle.clone();
    let bootstrap_script = state.bootstrap_script.clone();
    let next_session_id = session_id
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let (result_tx, result_rx) = sync_channel(1);

    std::thread::spawn(move || {
        let open_result = (|| -> Result<(), String> {
            let window = crate::create_debug_workbench_window(
                &app_handle,
                &bootstrap_script,
                next_session_id.as_deref(),
            )
            .map_err(|error| format!("{error:#}"))?;

            window
                .show()
                .map_err(|error| format!("failed to show debug workbench: {error}"))?;
            window
                .set_focus()
                .map_err(|error| format!("failed to focus debug workbench: {error}"))?;

            Ok(())
        })();

        if let Err(error) = result_tx.send(open_result) {
            eprintln!("[astrcode-debug-workbench] failed to report open result: {error}");
        }
    });

    result_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|error| format!("等待 Debug Workbench 启动结果超时: {error}"))?
}

#[cfg(not(debug_assertions))]
#[tauri::command]
pub fn open_debug_workbench(
    _app_handle: tauri::AppHandle,
    _session_id: Option<String>,
) -> Result<(), String> {
    Err("Debug Workbench 仅在 debug 构建可用".to_string())
}
