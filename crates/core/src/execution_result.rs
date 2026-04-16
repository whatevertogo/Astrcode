use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 执行结果中与调用类型无关的公共字段。
///
/// 该结构只承载通用执行元数据，避免工具结果与能力结果继续平行复制
/// `error / metadata / duration_ms / truncated` 四组字段。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResultCommon {
    /// 错误信息（仅在失败时设置）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 额外元数据（如 diff 信息、终端显示提示等）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// 执行耗时（毫秒）
    pub duration_ms: u64,
    /// 输出是否因大小限制被截断
    #[serde(default)]
    pub truncated: bool,
}

impl ExecutionResultCommon {
    pub fn success(metadata: Option<Value>, duration_ms: u64, truncated: bool) -> Self {
        Self {
            error: None,
            metadata,
            duration_ms,
            truncated,
        }
    }

    pub fn failure(
        error: impl Into<String>,
        metadata: Option<Value>,
        duration_ms: u64,
        truncated: bool,
    ) -> Self {
        Self {
            error: Some(error.into()),
            metadata,
            duration_ms,
            truncated,
        }
    }
}
