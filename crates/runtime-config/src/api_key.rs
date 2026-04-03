//! Profile API Key 解析逻辑。
//!
//! 本模块为 [`Profile`] 实现 `resolve_api_key` 方法，将配置中的 API key
//! 引用解析为实际的密钥字符串。
//!
//! # 解析规则
//!
//! 1. `literal:<value>` → 直接返回 `<value>`
//! 2. `env:<NAME>` → 从环境变量 `NAME` 读取，缺失时报错
//! 3. 裸值（如 `MY_API_KEY`）→ 若符合环境变量命名规范，尝试读取环境变量；
//!    若环境变量不存在则作为字面值回退（兼容旧版配置行为）
//!
//! # 安全考虑
//!
//! API key 不会写入配置文件。`resolve_api_key` 仅在运行时从内存配置和环境变量中
//! 读取，确保密钥始终只存在于进程内存中。

use astrcode_core::{AstrError, Result};

use crate::{env_resolver::resolve_env_value, types::Profile};

impl Profile {
    /// 解析 Profile 的 API key。
    ///
    /// 支持三种格式：
    /// - `literal:<value>`：直接返回 `<value>`，跳过环境变量解析
    /// - `env:<name>`：从环境变量 `<name>` 读取，缺失时返回错误
    /// - 裸值：若看起来像环境变量名（大写字母+数字+下划线且含下划线），
    ///   尝试读取环境变量；缺失时回退为字面值（兼容旧版配置）
    ///
    /// # 错误
    ///
    /// - `api_key` 为 `None` 或空字符串时返回 `MissingApiKey` 错误
    /// - `env:<name>` 格式但环境变量不存在时返回 `Validation` 错误
    /// - 解析后的值为空时返回 `MissingApiKey` 错误
    pub fn resolve_api_key(&self) -> Result<String> {
        let val = match &self.api_key {
            None => {
                return Err(AstrError::MissingApiKey(format!(
                    "profile '{}' 未配置 apiKey",
                    self.name
                )));
            },
            Some(s) => s.trim().to_string(),
        };

        if val.is_empty() {
            return Err(AstrError::MissingApiKey(format!(
                "profile '{}' 的 apiKey 不能为空",
                self.name
            )));
        }

        let resolved = resolve_env_value(&val).map_err(|error| match error {
            // Preserve profile context here so callers keep seeing the same actionable error.
            AstrError::Validation(message) => {
                AstrError::Validation(format!("profile '{}' 的 apiKey {}", self.name, message))
            },
            other => other,
        })?;
        if resolved.is_empty() {
            return Err(AstrError::MissingApiKey(format!(
                "profile '{}' 的 apiKey 不能为空",
                self.name
            )));
        }

        Ok(resolved)
    }
}
