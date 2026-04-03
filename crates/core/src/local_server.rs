use serde::{Deserialize, Serialize};

/// Desktop shell 和 sidecar server 之间用于发现和引导的共享载荷。
///
/// 这份结构同时用于：
/// - `astrcode-server` 写入 `~/.astrcode/run.json`，供浏览器开发桥接和诊断读取
/// - `astrcode-server` 通过 stdout 发出 ready 事件，供桌面端进程同步等待
///
/// 统一 DTO 的目的是避免两个进程各自维护一套近似字段，随着时间漂移后
/// 出现“run.json 能读、ready 行却解析失败”之类的隐蔽兼容问题。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalServerInfo {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: String,
    pub expires_at_ms: i64,
}

/// Sidecar stdout ready 事件的行前缀。
///
/// 桌面端只会消费带这个前缀的结构化行，保留普通 `println!` 日志给人工调试。
pub const LOCAL_SERVER_READY_PREFIX: &str = "ASTRCODE_SERVER_READY ";

impl LocalServerInfo {
    /// 将 ready 载荷编码成单行 stdout 协议。
    ///
    /// 单行文本比额外建一条 stdio 子协议简单，且保留了 shell/日志工具的可观测性。
    pub fn to_ready_line(&self) -> Result<String, serde_json::Error> {
        Ok(format!(
            "{LOCAL_SERVER_READY_PREFIX}{}",
            serde_json::to_string(self)?
        ))
    }

    /// 解析 sidecar stdout 中的结构化 ready 行。
    ///
    /// 返回 `Ok(None)` 表示这只是一条普通日志，不属于 ready 协议。
    pub fn parse_ready_line(line: &str) -> Result<Option<Self>, serde_json::Error> {
        let trimmed = line.trim();
        let Some(payload) = trimmed.strip_prefix(LOCAL_SERVER_READY_PREFIX) else {
            return Ok(None);
        };

        serde_json::from_str(payload).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::LocalServerInfo;

    #[test]
    fn ready_line_round_trips() {
        let info = LocalServerInfo {
            port: 62000,
            token: "bootstrap-token".to_string(),
            pid: 42,
            started_at: "2026-04-03T00:00:00Z".to_string(),
            expires_at_ms: 9_999_999,
        };

        let line = info
            .to_ready_line()
            .expect("ready payload should serialize");
        let parsed = LocalServerInfo::parse_ready_line(&line)
            .expect("ready payload should parse")
            .expect("ready line should be recognized");

        assert_eq!(parsed, info);
    }

    #[test]
    fn parse_ready_line_ignores_regular_logs() {
        assert!(
            LocalServerInfo::parse_ready_line("Ready: http://localhost:62000/")
                .expect("regular logs should not fail to parse")
                .is_none()
        );
    }
}
