use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::action::{LlmMessage, LlmResponse, ToolCallRequest, ToolDefinition};
use crate::llm::LlmProvider;

#[derive(Clone, Debug)]
pub struct OpenAiProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiProvider {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            api_key,
            model,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        cancel: CancellationToken,
    ) -> Result<LlmResponse> {
        let request = OpenAiChatRequest {
            model: &self.model,
            messages: messages.iter().map(to_openai_message).collect(),
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.iter().map(to_openai_tool).collect())
            },
            tool_choice: if tools.is_empty() { None } else { Some("auto") },
            stream: false,
        };

        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let send_future = self
            .client
            .post(endpoint)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send();

        let response = select! {
            _ = cancel.cancelled() => {
                return Err(anyhow!("llm request interrupted"));
            }
            result = send_future => result.context("failed to call openai-compatible endpoint")?
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("openai-compatible request failed with {}: {}", status, body));
        }

        let parsed: OpenAiChatResponse = response
            .json()
            .await
            .context("failed to parse openai-compatible response")?;
        let first_choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("openai-compatible response did not include choices"))?;

        let message = first_choice.message;
        let content = message.content.unwrap_or_default();
        let tool_calls = message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|call| ToolCallRequest {
                id: call.id,
                name: call.function.name,
                args: serde_json::from_str::<Value>(&call.function.arguments)
                    .unwrap_or_else(|_| Value::String(call.function.arguments)),
            })
            .collect();

        Ok(LlmResponse { content, tool_calls })
    }
}

fn to_openai_tool(def: &ToolDefinition) -> OpenAiTool {
    OpenAiTool {
        tool_type: "function".to_string(),
        function: OpenAiToolFunction {
            name: def.name.clone(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
        },
    }
}

fn to_openai_message(message: &LlmMessage) -> OpenAiRequestMessage {
    match message {
        LlmMessage::User { content } => OpenAiRequestMessage {
            role: "user".to_string(),
            content: Some(content.clone()),
            tool_call_id: None,
            tool_calls: None,
        },
        LlmMessage::Assistant { content, tool_calls } => OpenAiRequestMessage {
            role: "assistant".to_string(),
            content: if content.is_empty() {
                None
            } else {
                Some(content.clone())
            },
            tool_call_id: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(
                    tool_calls
                        .iter()
                        .map(|call| OpenAiToolCall {
                            id: call.id.clone(),
                            tool_type: "function".to_string(),
                            function: OpenAiToolCallFunction {
                                name: call.name.clone(),
                                arguments: call.args.to_string(),
                            },
                        })
                        .collect(),
                )
            },
        },
        LlmMessage::Tool {
            tool_call_id,
            content,
        } => OpenAiRequestMessage {
            role: "tool".to_string(),
            content: Some(content.clone()),
            tool_call_id: Some(tool_call_id.clone()),
            tool_calls: None,
        },
    }
}

#[derive(Debug, Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiRequestMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiRequestMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Serialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolCallFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseToolCall {
    id: String,
    function: OpenAiResponseToolFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseToolFunction {
    name: String,
    arguments: String,
}
