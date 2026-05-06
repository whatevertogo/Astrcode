use serde::{Deserialize, Serialize};

/// 系统提示词块所属层级。
///
/// 该类型出现在 durable prompt metrics 中，因此保留在 `core` 的事件语义层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SystemPromptLayer {
    Stable,
    SemiStable,
    Inherited,
    Dynamic,
    #[default]
    Unspecified,
}
