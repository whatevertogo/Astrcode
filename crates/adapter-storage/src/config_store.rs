//! 配置存储的文件系统实现。
//!
//! 提供 JSON 格式的配置文件读写、原子保存和项目 overlay 加载。
//! 实现 `application` 层定义的 `ConfigStore` 端口。

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use astrcode_core::{AstrError, Config, ConfigOverlay, Result, ports::ConfigStore};
use serde_json::Value;

/// 配置文件存储的文件系统实现。
///
/// 路径约定：
/// - 用户配置：`<base>/config.json`
/// - 项目 overlay：`<project>/.astrcode/config.json`
pub struct FileConfigStore {
    config_path: PathBuf,
}

impl FileConfigStore {
    pub fn new(config_path: PathBuf) -> Self {
        Self { config_path }
    }

    /// 默认路径 `~/.astrcode/config.json`。
    pub fn default_path() -> Result<Self> {
        let home = astrcode_core::home::resolve_home_dir()?;
        Ok(Self {
            config_path: home.join(".astrcode").join("config.json"),
        })
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    fn user_mcp_path(&self) -> PathBuf {
        self.config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("mcp.json")
    }

    fn local_mcp_path(&self, working_dir: &Path) -> PathBuf {
        working_dir.join(".astrcode").join("mcp.json")
    }

    fn project_mcp_path(&self, working_dir: &Path) -> PathBuf {
        working_dir.join(".mcp.json")
    }

    /// 从磁盘加载配置。文件不存在时创建默认配置。
    pub fn load(&self) -> Result<Config> {
        if !self.config_path.exists() {
            return self.init_default();
        }
        let config = self.read_json::<Config>(&self.config_path)?;
        Ok(config)
    }

    /// 原子保存配置到磁盘（先写临时文件再重命名）。
    pub fn save(&self, config: &Config) -> Result<()> {
        self.ensure_parent()?;
        self.write_json_atomic(&self.config_path, config)
    }

    /// 加载项目 overlay（文件存在时）。
    pub fn load_overlay(&self, working_dir: &Path) -> Result<Option<ConfigOverlay>> {
        let overlay_path = working_dir.join(".astrcode").join("config.json");
        if !overlay_path.exists() {
            return Ok(None);
        }
        self.read_json(&overlay_path).map(Some)
    }

    /// 保存项目 overlay；空 overlay 会删除文件，避免残留无意义配置。
    pub fn save_overlay(&self, working_dir: &Path, overlay: &ConfigOverlay) -> Result<()> {
        let overlay_path = working_dir.join(".astrcode").join("config.json");
        if overlay == &ConfigOverlay::default() {
            if overlay_path.exists() {
                fs::remove_file(&overlay_path).map_err(|e| {
                    AstrError::io(
                        format!("failed to remove overlay config {}", overlay_path.display()),
                        e,
                    )
                })?;
            }
            return Ok(());
        }
        if let Some(parent) = overlay_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AstrError::io(
                    format!("failed to create overlay dir '{}'", parent.display()),
                    e,
                )
            })?;
        }
        self.write_json_atomic(&overlay_path, overlay)
    }

    /// 读取用户级 `~/.astrcode/mcp.json` 原始值。
    pub fn load_user_mcp(&self) -> Result<Option<Value>> {
        let user_mcp_path = self.user_mcp_path();
        if !user_mcp_path.exists() {
            return Ok(None);
        }
        self.read_json(&user_mcp_path).map(Some)
    }

    /// 保存用户级 `~/.astrcode/mcp.json`；空值会删除文件，保持目录干净。
    pub fn save_user_mcp(&self, mcp: Option<&Value>) -> Result<()> {
        let user_mcp_path = self.user_mcp_path();
        match mcp {
            Some(value) => {
                if let Some(parent) = user_mcp_path.parent() {
                    ensure_parent_dir(parent)?;
                }
                self.write_json_atomic(&user_mcp_path, value)
            },
            None => {
                if user_mcp_path.exists() {
                    fs::remove_file(&user_mcp_path).map_err(|e| {
                        AstrError::io(
                            format!(
                                "failed to remove user MCP config {}",
                                user_mcp_path.display()
                            ),
                            e,
                        )
                    })?;
                }
                Ok(())
            },
        }
    }

    /// 读取项目级 `.mcp.json` 原始值。
    pub fn load_project_mcp(&self, working_dir: &Path) -> Result<Option<Value>> {
        let project_path = self.project_mcp_path(working_dir);
        if !project_path.exists() {
            return Ok(None);
        }
        self.read_json(&project_path).map(Some)
    }

    /// 保存项目级 `.mcp.json`；空值会删除文件，保持工作区干净。
    pub fn save_project_mcp(&self, working_dir: &Path, mcp: Option<&Value>) -> Result<()> {
        let project_path = self.project_mcp_path(working_dir);
        match mcp {
            Some(value) => {
                if let Some(parent) = project_path.parent() {
                    ensure_parent_dir(parent)?;
                }
                self.write_json_atomic(&project_path, value)
            },
            None => {
                if project_path.exists() {
                    fs::remove_file(&project_path).map_err(|e| {
                        AstrError::io(
                            format!(
                                "failed to remove project MCP config {}",
                                project_path.display()
                            ),
                            e,
                        )
                    })?;
                }
                Ok(())
            },
        }
    }

    /// 读取项目本地 `.astrcode/mcp.json` 原始值。
    pub fn load_local_mcp(&self, working_dir: &Path) -> Result<Option<Value>> {
        let local_path = self.local_mcp_path(working_dir);
        if !local_path.exists() {
            return Ok(None);
        }
        self.read_json(&local_path).map(Some)
    }

    /// 保存项目本地 `.astrcode/mcp.json`；空值会删除文件，保持工作区干净。
    pub fn save_local_mcp(&self, working_dir: &Path, mcp: Option<&Value>) -> Result<()> {
        let local_path = self.local_mcp_path(working_dir);
        match mcp {
            Some(value) => {
                if let Some(parent) = local_path.parent() {
                    ensure_parent_dir(parent)?;
                }
                self.write_json_atomic(&local_path, value)
            },
            None => {
                if local_path.exists() {
                    fs::remove_file(&local_path).map_err(|e| {
                        AstrError::io(
                            format!("failed to remove local MCP config {}", local_path.display()),
                            e,
                        )
                    })?;
                }
                Ok(())
            },
        }
    }

    fn init_default(&self) -> Result<Config> {
        self.ensure_parent()?;
        let default_cfg = Config::default();
        self.write_json_atomic(&self.config_path, &default_cfg)?;
        log::warn!(
            "Config created at {}，请填写 apiKey",
            self.config_path.display()
        );
        Ok(default_cfg)
    }

    fn ensure_parent(&self) -> Result<()> {
        if let Some(parent) = self.config_path.parent() {
            ensure_parent_dir(parent)?;
        }
        Ok(())
    }

    fn read_json<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<T> {
        let raw = fs::read_to_string(path).map_err(|e| {
            AstrError::io(format!("failed to read config at {}", path.display()), e)
        })?;
        serde_json::from_str::<T>(&raw).map_err(|e| {
            AstrError::parse(format!("failed to parse config at {}", path.display()), e)
        })
    }

    /// 原子写入：先写 .json.tmp → fsync → 重命名。Windows 需三步替换。
    fn write_json_atomic<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let json = serde_json::to_vec_pretty(value)
            .map_err(|e| AstrError::parse("failed to serialize config", e))?;

        let tmp_path = path.with_extension("json.tmp");
        let mut tmp_file = fs::File::create(&tmp_path).map_err(|e| {
            AstrError::io(
                format!("failed to create temp file {}", tmp_path.display()),
                e,
            )
        })?;
        tmp_file.write_all(&json).map_err(|e| {
            AstrError::io(
                format!("failed to write temp file {}", tmp_path.display()),
                e,
            )
        })?;
        tmp_file
            .flush()
            .map_err(|e| AstrError::io("failed to flush temp config".to_string(), e))?;
        tmp_file
            .sync_all()
            .map_err(|e| AstrError::io("failed to fsync temp config".to_string(), e))?;
        drop(tmp_file);

        if let Err(err) = fs::rename(&tmp_path, path) {
            #[cfg(windows)]
            {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    let backup_path = path.with_extension("json.bak");
                    let _ = fs::remove_file(&backup_path);
                    if let Err(e) = fs::rename(path, &backup_path) {
                        let _ = fs::remove_file(&tmp_path);
                        return Err(AstrError::Internal(format!(
                            "failed to backup config before replace: {}",
                            e
                        )));
                    }
                    if let Err(e) = fs::rename(&tmp_path, path) {
                        let _ = fs::rename(&backup_path, path);
                        return Err(AstrError::Internal(format!(
                            "failed to replace config: {}",
                            e
                        )));
                    }
                    let _ = fs::remove_file(&backup_path);
                    return Ok(());
                }
            }
            let _ = fs::remove_file(&tmp_path);
            return Err(AstrError::Internal(format!(
                "failed to replace config {}: {}",
                path.display(),
                err
            )));
        }
        Ok(())
    }
}

fn ensure_parent_dir(parent: &Path) -> Result<()> {
    fs::create_dir_all(parent).map_err(|e| {
        AstrError::io(
            format!("failed to create config dir '{}'", parent.display()),
            e,
        )
    })
}

impl std::fmt::Debug for FileConfigStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileConfigStore")
            .field("config_path", &self.config_path)
            .finish()
    }
}

impl ConfigStore for FileConfigStore {
    fn load(&self) -> Result<Config> {
        FileConfigStore::load(self)
    }

    fn save(&self, config: &Config) -> Result<()> {
        FileConfigStore::save(self, config)
    }

    fn path(&self) -> std::path::PathBuf {
        self.config_path.clone()
    }

    fn load_overlay(&self, working_dir: &std::path::Path) -> Result<Option<ConfigOverlay>> {
        FileConfigStore::load_overlay(self, working_dir)
    }

    fn save_overlay(&self, working_dir: &std::path::Path, overlay: &ConfigOverlay) -> Result<()> {
        FileConfigStore::save_overlay(self, working_dir, overlay)
    }

    fn load_project_mcp(&self, working_dir: &std::path::Path) -> Result<Option<Value>> {
        FileConfigStore::load_project_mcp(self, working_dir)
    }

    fn save_project_mcp(&self, working_dir: &std::path::Path, mcp: Option<&Value>) -> Result<()> {
        FileConfigStore::save_project_mcp(self, working_dir, mcp)
    }

    fn load_user_mcp(&self) -> Result<Option<Value>> {
        FileConfigStore::load_user_mcp(self)
    }

    fn save_user_mcp(&self, mcp: Option<&Value>) -> Result<()> {
        FileConfigStore::save_user_mcp(self, mcp)
    }

    fn load_local_mcp(&self, working_dir: &std::path::Path) -> Result<Option<Value>> {
        FileConfigStore::load_local_mcp(self, working_dir)
    }

    fn save_local_mcp(&self, working_dir: &std::path::Path, mcp: Option<&Value>) -> Result<()> {
        FileConfigStore::save_local_mcp(self, working_dir, mcp)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::FileConfigStore;

    #[test]
    fn user_mcp_roundtrip_uses_home_sidecar_file() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let store = FileConfigStore::new(temp.path().join(".astrcode").join("config.json"));
        let value = json!({
            "mcpServers": {
                "demo": {
                    "command": "npx"
                }
            }
        });

        store
            .save_user_mcp(Some(&value))
            .expect("user mcp should save");

        let loaded = store
            .load_user_mcp()
            .expect("user mcp should load")
            .expect("user mcp file should exist");
        assert_eq!(loaded, value);
        assert!(temp.path().join(".astrcode").join("mcp.json").exists());
    }

    #[test]
    fn local_mcp_roundtrip_uses_project_astrcode_sidecar_file() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let project = tempfile::tempdir().expect("project tempdir should be created");
        let store = FileConfigStore::new(temp.path().join(".astrcode").join("config.json"));
        let value = json!({
            "mcpServers": {
                "demo": {
                    "type": "http",
                    "url": "http://localhost:8080/mcp"
                }
            }
        });

        store
            .save_local_mcp(project.path(), Some(&value))
            .expect("local mcp should save");

        let loaded = store
            .load_local_mcp(project.path())
            .expect("local mcp should load")
            .expect("local mcp file should exist");
        assert_eq!(loaded, value);
        assert!(project.path().join(".astrcode").join("mcp.json").exists());
    }
}
