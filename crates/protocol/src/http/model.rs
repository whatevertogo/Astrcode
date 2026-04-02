//! 模型信息相关 DTO
//!
//! 定义模型查询接口的请求/响应结构，包括当前活跃模型信息和可用模型列表。

use serde::{Deserialize, Serialize};

/// GET /api/models/current 响应体——当前活跃的模型信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentModelInfoDto {
    /// 配置文件中的 profile 名称
    pub profile_name: String,
    /// 当前使用的模型 ID（如 "claude-3-5-sonnet"）
    pub model: String,
    /// 提供商类型（"anthropic" 或 "openai"）
    pub provider_kind: String,
}

/// GET /api/models 响应体中的单个模型选项。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelOptionDto {
    /// 此模型所属的 profile 名称
    pub profile_name: String,
    /// 模型 ID
    pub model: String,
    /// 提供商类型
    pub provider_kind: String,
}
