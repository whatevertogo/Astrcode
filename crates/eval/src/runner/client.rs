use std::time::Duration;

use astrcode_protocol::http::{CreateSessionRequest, PromptAcceptedResponse, PromptRequest};
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::{EvalError, EvalResult};

const AUTH_HEADER_NAME: &str = "x-astrcode-token";

#[derive(Debug, Clone)]
pub struct ServerControlClient {
    http: Client,
    base_url: String,
    auth_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionCreateAck {
    pub session_id: String,
    pub working_dir: String,
}

impl ServerControlClient {
    pub fn new(
        base_url: impl Into<String>,
        auth_token: Option<String>,
        timeout: Duration,
    ) -> EvalResult<Self> {
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| EvalError::http("创建 HTTP client 失败", error))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            auth_token,
        })
    }

    pub async fn probe(&self) -> EvalResult<()> {
        let response = self
            .send(self.http.get(format!("{}/", self.base_url)))
            .await
            .map_err(|error| EvalError::http("探测 server 失败", error))?;
        if response.status().is_success() {
            return Ok(());
        }

        let response = self
            .send(
                self.http
                    .get(format!("{}/__astrcode__/run-info", self.base_url))
                    .header("origin", "http://127.0.0.1:5173"),
            )
            .await
            .map_err(|error| EvalError::http("探测 server 失败", error))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(EvalError::validation(format!(
                "server 探测失败，HTTP 状态 {}",
                response.status()
            )))
        }
    }

    pub async fn create_session(&self, working_dir: &str) -> EvalResult<SessionCreateAck> {
        let request = self
            .http
            .post(format!("{}/api/sessions", self.base_url))
            .json(&CreateSessionRequest {
                working_dir: working_dir.to_string(),
            });
        let response = self
            .send(request)
            .await
            .map_err(|error| EvalError::http("创建 session 失败", error))?;
        if !response.status().is_success() {
            return Err(status_error("创建 session", response.status()));
        }
        response
            .json::<SessionCreateAck>()
            .await
            .map_err(|error| EvalError::http("解析 create_session 响应失败", error))
    }

    pub async fn submit_turn(
        &self,
        session_id: &str,
        prompt: &str,
    ) -> EvalResult<PromptAcceptedResponse> {
        let request = self
            .http
            .post(format!(
                "{}/api/sessions/{session_id}/prompts",
                self.base_url
            ))
            .json(&PromptRequest {
                text: prompt.to_string(),
                skill_invocation: None,
                control: None,
            });
        let response = self
            .send(request)
            .await
            .map_err(|error| EvalError::http("提交评测 turn 失败", error))?;
        if !response.status().is_success() && response.status() != StatusCode::ACCEPTED {
            return Err(status_error("提交评测 turn", response.status()));
        }
        response
            .json::<PromptAcceptedResponse>()
            .await
            .map_err(|error| EvalError::http("解析 submit_turn 响应失败", error))
    }

    async fn send(
        &self,
        mut request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, reqwest::Error> {
        if let Some(token) = &self.auth_token {
            request = request.header(AUTH_HEADER_NAME, token);
        }
        request.send().await
    }
}

fn status_error(action: &str, status: StatusCode) -> EvalError {
    EvalError::validation(format!("{action} 失败，HTTP 状态 {status}"))
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use axum::{
        Json, Router,
        routing::{get, post},
    };
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::ServerControlClient;

    async fn test_server() -> SocketAddr {
        let app = Router::new()
            .route(
                "/__astrcode__/run-info",
                get(|| async { Json(json!({"ok": true})) }),
            )
            .route(
                "/api/sessions",
                post(|| async {
                    Json(json!({
                        "sessionId": "session-1",
                        "workingDir": "D:/workspace",
                    }))
                }),
            )
            .route(
                "/api/sessions/{id}/prompts",
                post(|| async {
                    (
                        reqwest::StatusCode::ACCEPTED,
                        Json(json!({
                            "accepted": true,
                            "message": "accepted",
                            "turnId": "turn-1",
                            "sessionId": "session-1",
                            "branchedFromSessionId": null,
                            "acceptedControl": null,
                        })),
                    )
                }),
            );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("local addr should resolve");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        addr
    }

    #[tokio::test]
    async fn client_creates_session_and_submits_turn() {
        let addr = test_server().await;
        let client =
            ServerControlClient::new(format!("http://{addr}"), None, Duration::from_secs(3))
                .expect("client should build");

        client.probe().await.expect("probe should succeed");
        let session = client
            .create_session("D:/workspace")
            .await
            .expect("session should create");
        assert_eq!(session.session_id, "session-1");
        let accepted = client
            .submit_turn("session-1", "hello")
            .await
            .expect("turn should submit");
        assert!(accepted.accepted);
        assert_eq!(accepted.turn_id.as_deref(), Some("turn-1"));
    }
}
