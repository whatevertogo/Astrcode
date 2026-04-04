//! Provider 连接测试工具。
//!
//! 本模块提供 [`test_connection`] 函数，用于验证 Profile 配置是否可以正常连接到
//! LLM Provider API。
//!
//! # 测试策略
//!
//! 向 Provider 发送一个最小化的请求（`max_tokens: 1`，内容为 `"hi"`），
//! 根据响应状态判断连接状态：
/// - 2xx：连接成功
/// - 401：API Key 无效
/// - 其他：HTTP 错误
/// - 超时：网络不可达
///
/// # 返回值设计
///
/// 无论测试成功或失败都返回 `Ok(TestResult)`，HTTP 错误被封装在 `error` 字段中
/// 而非作为 `Result::Err` 传播。这样调用方可以统一处理成功和失败两种情况，
/// 便于前端展示具体的错误原因。
use std::time::Duration;

use astrcode_core::{AstrError, Result};
use serde_json::json;

use crate::{
    constants::{
        ANTHROPIC_VERSION, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI,
        resolve_anthropic_messages_api_url,
    },
    types::{Profile, TestResult},
};

/// 测试指定 Profile 和模型的 Provider 连接。
///
/// 发送一个最小化请求验证 API key 和网络连通性。
/// 超时设置为 10 秒，避免长时间阻塞。
///
/// # 返回值
///
/// 始终返回 `Ok(TestResult)`，连接失败信息封装在 `TestResult.error` 中。
/// 仅在不支持的 `provider_kind` 时返回 `Err`。
pub async fn test_connection(profile: &Profile, model: &str) -> Result<TestResult> {
    // 统一在这里解析最终请求地址，避免连接测试与运行时使用不同的 URL 规则。
    let provider = if profile.provider_kind == PROVIDER_KIND_ANTHROPIC {
        resolve_anthropic_messages_api_url(&profile.base_url)
    } else {
        profile.base_url.trim().trim_end_matches('/').to_string()
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
        },
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
        },
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
        },
        other => Err(AstrError::UnsupportedProvider(other.to_string())),
    }
}

/// 将 HTTP 响应转换为 [`TestResult`]。
///
/// 根据响应状态码分类：
/// - 2xx：成功
/// - 401：API Key 无效
/// - 其他状态码：请求失败
/// - 超时错误：连接超时
/// - 其他错误：网络异常
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
        },
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
