use astrcode_core::{Result, SessionId, TurnId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// plugin-host owner 的资源读取请求上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRequestContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// plugin-host owner 的资源读取结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceReadResult {
    pub uri: String,
    pub content: Value,
    #[serde(default)]
    pub metadata: Value,
}

#[async_trait]
pub trait ResourceProvider: Send + Sync {
    async fn read_resource(
        &self,
        uri: &str,
        context: &ResourceRequestContext,
    ) -> Result<ResourceReadResult>;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ResourceReadResult, ResourceRequestContext};

    #[test]
    fn resource_context_defaults_to_empty_metadata() {
        let context = ResourceRequestContext::default();

        assert!(context.session_id.is_none());
        assert_eq!(context.metadata, serde_json::Value::Null);
    }

    #[test]
    fn resource_read_result_preserves_uri_and_content() {
        let result = ResourceReadResult {
            uri: "skill://review".to_string(),
            content: json!({"name": "review"}),
            metadata: serde_json::Value::Null,
        };

        assert_eq!(result.uri, "skill://review");
        assert_eq!(result.content["name"], "review");
    }
}
