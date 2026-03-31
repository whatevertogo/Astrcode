//! Connection testing utilities for provider profiles.

use std::time::Duration;

use astrcode_core::{AstrError, Result};
use serde_json::json;

use crate::constants::{
    ANTHROPIC_MESSAGES_API_URL, ANTHROPIC_VERSION, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI,
};
use crate::types::{Profile, TestResult};

/// Tests the connection to a provider using the given profile and model.
pub async fn test_connection(profile: &Profile, model: &str) -> Result<TestResult> {
    let provider = match profile.provider_kind.as_str() {
        ANTHROPIC_MESSAGES_API_URL => ANTHROPIC_MESSAGES_API_URL.to_string(),
        _ => profile.base_url.trim_end_matches('/').to_string(),
    };
    let api_key = match profile.resolve_api_key() {
        Ok(api_key) => api_key,
        Err(err) => {
            return Ok(TestResult {
                success: false,
                provider,
                model: model.to_string(),
                error: Some(err.to_string()),
            });
        }
    };

    match profile.provider_kind.as_str() {
        PROVIDER_KIND_OPENAI => {
            let endpoint = format!("{}/chat/completions", provider);
            let response = reqwest::Client::new()
                .post(endpoint)
                .bearer_auth(api_key)
                .timeout(Duration::from_secs(10))
                .json(&json!({
                    "model": model,
                    "messages": [
                        {
                            "role": "user",
                            "content": "hi"
                        }
                    ],
                    "max_tokens": 1,
                    "stream": false
                }))
                .send()
                .await;

            Ok(connection_result_from_response(response, provider, model))
        }
        PROVIDER_KIND_ANTHROPIC => {
            let response = reqwest::Client::new()
                .post(&provider)
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .timeout(Duration::from_secs(10))
                .json(&json!({
                    "model": model,
                    "max_tokens": 1,
                    "messages": [
                        {
                            "role": "user",
                            "content": "hi"
                        }
                    ]
                }))
                .send()
                .await;

            Ok(connection_result_from_response(response, provider, model))
        }
        other => Err(AstrError::UnsupportedProvider(other.to_string())),
    }
}

/// Converts an HTTP response to a TestResult.
fn connection_result_from_response(
    response: std::result::Result<reqwest::Response, reqwest::Error>,
    provider: String,
    model: &str,
) -> TestResult {
    match response {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                TestResult {
                    success: true,
                    provider,
                    model: model.to_string(),
                    error: None,
                }
            } else if status == reqwest::StatusCode::UNAUTHORIZED {
                TestResult {
                    success: false,
                    provider,
                    model: model.to_string(),
                    error: Some("API Key 无效或未授权".to_string()),
                }
            } else {
                TestResult {
                    success: false,
                    provider,
                    model: model.to_string(),
                    error: Some(format!("请求失败: {}", status)),
                }
            }
        }
        Err(err) if err.is_timeout() => TestResult {
            success: false,
            provider,
            model: model.to_string(),
            error: Some("连接超时".to_string()),
        },
        Err(err) => TestResult {
            success: false,
            provider,
            model: model.to_string(),
            error: Some(err.to_string()),
        },
    }
}
