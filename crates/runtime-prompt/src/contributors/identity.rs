//! 身份贡献者。
//!
//! 从 `~/.astrcode/IDENTITY.md` 加载用户自定义身份定义，
//! 若文件不存在则使用内置默认值。

use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use log::{info, warn};

use super::shared::{cache_marker_for_path, user_astrcode_file_path};
use crate::{BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor};

/// 身份贡献者。
///
/// 负责生成 system prompt 中的身份定义部分。
/// 优先读取 `~/.astrcode/IDENTITY.md`，不存在时使用默认描述。
pub struct IdentityContributor;

const DEFAULT_IDENTITY: &str = "\
You are AstrCode, a local AI coding agent running on the user's machine. \
You help with coding tasks, file editing, and terminal commands. \
做任何事情之前请先确认用户的意图，必要时可以提出澄清问题.\
做任何删除、强推、数据库变更等高风险操作前必须先确认.\
请保证代码符合当前技术栈和当前代码库的最佳实践，包括但不限于代码架构和设计原则.\
动手前一定要先读相关文件建立上下文.\
改完要给出可运行的验证或测试命令形成闭环.\
最后总结的时候可以给下一步建议.";

/// Returns the path to the user-wide IDENTITY.md file.
pub fn user_identity_md_path() -> Option<PathBuf> {
    user_astrcode_file_path("IDENTITY.md")
}

/// Loads the identity definition from the given path.
/// Returns None if the file doesn't exist or can't be read.
/// Enforces a maximum size limit to prevent excessively large identity files
/// from bloating the system prompt.
const MAX_IDENTITY_SIZE: usize = 4096;

/// 按字符边界截断字符串，避免截断多字节 UTF-8 字符。
///
/// 如果 `max_bytes` 恰好落在多字节字符中间，
/// 会向前找到最后一个完整的字符边界。
fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // 从 max_bytes 向前找，直到找到有效的 UTF-8 字符边界
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn load_identity_md(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }

    match fs::read_to_string(path) {
        Ok(content) => {
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                return None;
            }
            // 超过限制时截断到 MAX_IDENTITY_SIZE 字节，并记录警告
            if trimmed.len() > MAX_IDENTITY_SIZE {
                warn!(
                    "identity file {} exceeds {} bytes ({} bytes), truncating to limit",
                    path.display(),
                    MAX_IDENTITY_SIZE,
                    trimmed.len()
                );
                // 按字符边界截断，避免截断多字节字符
                let truncated = truncate_to_char_boundary(&trimmed, MAX_IDENTITY_SIZE);
                info!("loaded custom identity from {} (truncated)", path.display());
                return Some(truncated.to_string());
            }
            info!("loaded custom identity from {}", path.display());
            Some(trimmed)
        }
        Err(error) => {
            warn!("failed to read {}: {}", path.display(), error);
            None
        }
    }
}

#[async_trait]
impl PromptContributor for IdentityContributor {
    fn contributor_id(&self) -> &'static str {
        "identity"
    }

    fn cache_version(&self) -> u64 {
        2
    }

    fn cache_fingerprint(&self, _ctx: &PromptContext) -> String {
        let user_marker = user_identity_md_path()
            .map(|path| format!("{}={}", path.display(), cache_marker_for_path(&path)))
            .unwrap_or_else(|| "user=<unresolved>".to_string());

        user_marker
    }

    async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        let identity = user_identity_md_path()
            .as_ref()
            .and_then(|path| load_identity_md(path))
            .unwrap_or_else(|| DEFAULT_IDENTITY.to_string());

        PromptContribution {
            blocks: vec![BlockSpec::system_text(
                "identity",
                BlockKind::Identity,
                "Identity",
                identity,
            )],
            ..PromptContribution::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::BlockContent;
    use astrcode_core::test_support::TestEnvGuard;

    fn context() -> PromptContext {
        PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string()],
            capability_descriptors: Vec::new(),
            prompt_declarations: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        }
    }

    #[tokio::test]
    async fn returns_default_identity_when_file_missing() {
        let _guard = TestEnvGuard::new();
        let contributor = IdentityContributor;

        let contribution = contributor.contribute(&context()).await;

        assert_eq!(contribution.blocks.len(), 1);
        assert_eq!(contribution.blocks[0].kind, BlockKind::Identity);
        if let BlockContent::Text(content) = &contribution.blocks[0].content {
            assert!(content.contains("AstrCode"));
        } else {
            panic!("Expected Text content");
        }
    }

    #[tokio::test]
    async fn returns_custom_identity_when_file_exists() {
        let guard = TestEnvGuard::new();
        let identity_path = guard.home_dir().join(".astrcode").join("IDENTITY.md");
        fs::create_dir_all(identity_path.parent().expect("parent should exist"))
            .expect("identity dir should be created");
        fs::write(&identity_path, "You are a custom AI assistant.")
            .expect("identity file should be written");
        let contributor = IdentityContributor;

        let contribution = contributor.contribute(&context()).await;

        assert_eq!(contribution.blocks.len(), 1);
        if let BlockContent::Text(content) = &contribution.blocks[0].content {
            assert!(content.contains("custom AI assistant"));
        } else {
            panic!("Expected Text content");
        }
    }
}
