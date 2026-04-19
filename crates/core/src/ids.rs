//! # 强类型标识定义
//!
//! 通过宏批量生成 newtype 包装，将 SessionId / TurnId / AgentId / SubRunId /
//! DeliveryId / CapabilityName 从裸字符串中剥离，避免跨层 API 依赖脆弱的字符串约定。
//!
//! 每个 ID 类型实现了 Display、Deref、From<String/&str>、Serialize、Deserialize
//! 等标准 trait，可直接用于格式化、比较和序列化。

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! typed_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            pub fn into_string(self) -> String {
                self.0
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self(String::new())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.into_string()
            }
        }
    };
}

typed_id!(SessionId);
typed_id!(TurnId);
typed_id!(AgentId);
typed_id!(SubRunId);
typed_id!(DeliveryId);
typed_id!(CapabilityName);
