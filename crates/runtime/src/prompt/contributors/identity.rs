use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;
use log::{info, warn};

use crate::prompt::{BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor};

pub struct IdentityContributor;

const DEFAULT_IDENTITY: &str = "\
You are AstrCode, a local AI coding agent running on the user's machine. \
You help with coding tasks, file editing, and terminal commands. \
Be concise and accurate. Prefer editing files directly over explaining how to do it.";

/// Returns the path to the user-wide IDENTITY.md file.
/// Respects ASTRCODE_HOME_DIR if set, otherwise falls back to ~/.astrcode/IDENTITY.md
pub fn user_identity_md_path() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("ASTRCODE_HOME_DIR") {
        if !home.is_empty() {
            return Some(PathBuf::from(home).join(".astrcode").join("IDENTITY.md"));
        }
    }

    #[cfg(test)]
    if let Some(home) = crate::test_support::test_home_dir() {
        return Some(home.join(".astrcode").join("IDENTITY.md"));
    }

    match dirs::home_dir() {
        Some(home) => Some(home.join(".astrcode").join("IDENTITY.md")),
        None => {
            warn!("failed to resolve home dir for IDENTITY.md");
            None
        }
    }
}

/// Loads the identity definition from the given path.
/// Returns None if the file doesn't exist or can't be read.
/// Enforces a maximum size limit to prevent excessively large identity files
/// from bloating the system prompt.
const MAX_IDENTITY_SIZE: usize = 4096;

pub fn load_identity_md(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }

    match fs::read_to_string(path) {
        Ok(content) => {
            if content.len() > MAX_IDENTITY_SIZE {
                warn!(
                    "identity file {} exceeds {} bytes ({} bytes), truncating",
                    path.display(),
                    MAX_IDENTITY_SIZE,
                    content.len()
                );
            }
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                info!("loaded custom identity from {}", path.display());
                Some(trimmed)
            }
        }
        Err(error) => {
            warn!("failed to read {}: {}", path.display(), error);
            None
        }
    }
}

/// Returns a cache marker for the given path, used for cache invalidation.
fn cache_marker_for_path(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(metadata) => {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();

            format!("present:{}:{modified}", metadata.len())
        }
        Err(_) => "missing".to_string(),
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
    use crate::prompt::BlockContent;
    use crate::test_support::TestEnvGuard;

    fn context() -> PromptContext {
        PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string()],
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
        assert_eq!(contribution.blocks[0].title, "Identity");
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

    #[tokio::test]
    async fn cache_fingerprint_contains_path() {
        let guard = TestEnvGuard::new();
        let identity_path = guard.home_dir().join(".astrcode").join("IDENTITY.md");
        fs::create_dir_all(identity_path.parent().expect("parent should exist"))
            .expect("identity dir should be created");
        fs::write(&identity_path, "content").expect("identity file should be written");
        let contributor = IdentityContributor;
        let ctx = context();

        let fingerprint = contributor.cache_fingerprint(&ctx);
        assert!(fingerprint.contains("IDENTITY.md"));
        assert!(fingerprint.contains("present:"));
    }
}
