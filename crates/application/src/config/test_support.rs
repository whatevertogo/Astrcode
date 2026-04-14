use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use astrcode_core::{
    Config, ConfigOverlay, Result,
    ports::{ConfigStore, McpConfigFileScope},
};
use serde_json::Value;

#[derive(Default)]
pub(crate) struct TestConfigStore {
    pub(crate) config: Mutex<Config>,
    pub(crate) overlay: Mutex<Option<ConfigOverlay>>,
    pub(crate) user_mcp: Mutex<Option<Value>>,
    pub(crate) local_mcp: Mutex<Option<Value>>,
    pub(crate) project_mcp: Mutex<Option<Value>>,
}

impl ConfigStore for TestConfigStore {
    fn load(&self) -> Result<Config> {
        Ok(self.config.lock().expect("config mutex").clone())
    }

    fn save(&self, config: &Config) -> Result<()> {
        *self.config.lock().expect("config mutex") = config.clone();
        Ok(())
    }

    fn path(&self) -> PathBuf {
        PathBuf::from("test-config.json")
    }

    fn load_overlay(&self, _working_dir: &Path) -> Result<Option<ConfigOverlay>> {
        Ok(self.overlay.lock().expect("overlay mutex").clone())
    }

    fn save_overlay(&self, _working_dir: &Path, overlay: &ConfigOverlay) -> Result<()> {
        *self.overlay.lock().expect("overlay mutex") = Some(overlay.clone());
        Ok(())
    }

    fn load_mcp(
        &self,
        scope: McpConfigFileScope,
        _working_dir: Option<&Path>,
    ) -> Result<Option<Value>> {
        let value = match scope {
            McpConfigFileScope::User => self.user_mcp.lock().expect("user mcp mutex").clone(),
            McpConfigFileScope::Local => self.local_mcp.lock().expect("local mcp mutex").clone(),
            McpConfigFileScope::Project => {
                self.project_mcp.lock().expect("project mcp mutex").clone()
            },
        };
        Ok(value)
    }

    fn save_mcp(
        &self,
        scope: McpConfigFileScope,
        _working_dir: Option<&Path>,
        mcp: Option<&Value>,
    ) -> Result<()> {
        match scope {
            McpConfigFileScope::User => {
                *self.user_mcp.lock().expect("user mcp mutex") = mcp.cloned();
            },
            McpConfigFileScope::Local => {
                *self.local_mcp.lock().expect("local mcp mutex") = mcp.cloned();
            },
            McpConfigFileScope::Project => {
                *self.project_mcp.lock().expect("project mcp mutex") = mcp.cloned();
            },
        }
        Ok(())
    }
}
