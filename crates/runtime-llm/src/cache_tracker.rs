//! Prompt cache break detection and tracking
//!
//! This module tracks changes that invalidate Anthropic's prompt cache:
//! - System prompt changes (identity, capabilities, rules)
//! - Tool definition changes (additions, removals, modifications)
//! - Model changes
//! - Provider changes
//!
//! Inspired by Claude Code's promptCacheBreakDetection.ts

use serde::{Deserialize, Serialize};

/// Tracks cache-breaking changes across requests
#[derive(Debug, Clone, Default)]
pub struct CacheTracker {
    /// Hash of the current system prompt
    system_prompt_hash: Option<String>,
    /// Hash of the current tool definitions
    tools_hash: Option<String>,
    /// Current model name
    model: Option<String>,
    /// Current provider
    provider: Option<String>,
    /// Reasons for cache breaks in the current session
    break_reasons: Vec<CacheBreakReason>,
}

/// Reasons why cache might break
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CacheBreakReason {
    /// System prompt changed
    SystemPromptChanged,
    /// Tool definitions changed
    ToolsChanged,
    /// Model changed
    ModelChanged,
    /// Provider changed
    ProviderChanged,
    /// First request (no cache exists)
    FirstRequest,
}

impl CacheTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the request will break cache and update state
    pub fn check_and_update(
        &mut self,
        system_prompt: &str,
        tools: &[String],
        model: &str,
        provider: &str,
    ) -> Vec<CacheBreakReason> {
        let mut reasons = Vec::new();

        // Hash the inputs
        let new_system_hash = Self::hash_string(system_prompt);
        let new_tools_hash = Self::hash_strings(tools);

        // Check for changes
        if self.system_prompt_hash.is_none() {
            reasons.push(CacheBreakReason::FirstRequest);
        } else {
            if self.system_prompt_hash.as_ref() != Some(&new_system_hash) {
                reasons.push(CacheBreakReason::SystemPromptChanged);
            }
            if self.tools_hash.as_ref() != Some(&new_tools_hash) {
                reasons.push(CacheBreakReason::ToolsChanged);
            }
            if self.model.as_deref() != Some(model) {
                reasons.push(CacheBreakReason::ModelChanged);
            }
            if self.provider.as_deref() != Some(provider) {
                reasons.push(CacheBreakReason::ProviderChanged);
            }
        }

        // Update state
        self.system_prompt_hash = Some(new_system_hash);
        self.tools_hash = Some(new_tools_hash);
        if self.model.as_deref() != Some(model) {
            self.model = Some(model.to_string());
        }
        if self.provider.as_deref() != Some(provider) {
            self.provider = Some(provider.to_string());
        }
        self.break_reasons.extend(reasons.clone());

        reasons
    }

    /// Get all cache break reasons in this session
    pub fn get_break_reasons(&self) -> &[CacheBreakReason] {
        &self.break_reasons
    }

    /// Reset the tracker (e.g., for new session)
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Simple hash function for strings
    fn hash_string(s: &str) -> String {
        use std::{
            collections::hash_map::DefaultHasher,
            hash::{Hash, Hasher},
        };

        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Hash multiple strings together
    fn hash_strings(strings: &[String]) -> String {
        use std::{
            collections::hash_map::DefaultHasher,
            hash::{Hash, Hasher},
        };

        let mut hasher = DefaultHasher::new();
        for s in strings {
            s.hash(&mut hasher);
        }
        format!("{:x}", hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_request_is_cache_break() {
        let mut tracker = CacheTracker::new();
        let reasons = tracker.check_and_update(
            "system prompt",
            &["tool1".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        assert_eq!(reasons.len(), 1);
        assert!(matches!(reasons[0], CacheBreakReason::FirstRequest));
    }

    #[test]
    fn test_no_change_no_break() {
        let mut tracker = CacheTracker::new();

        // First request
        tracker.check_and_update(
            "system prompt",
            &["tool1".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        // Second request with same inputs
        let reasons = tracker.check_and_update(
            "system prompt",
            &["tool1".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        assert_eq!(reasons.len(), 0);
    }

    #[test]
    fn test_system_prompt_change_breaks_cache() {
        let mut tracker = CacheTracker::new();

        tracker.check_and_update(
            "system prompt v1",
            &["tool1".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        let reasons = tracker.check_and_update(
            "system prompt v2",
            &["tool1".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        assert_eq!(reasons.len(), 1);
        assert!(matches!(reasons[0], CacheBreakReason::SystemPromptChanged));
    }

    #[test]
    fn test_tools_change_breaks_cache() {
        let mut tracker = CacheTracker::new();

        tracker.check_and_update(
            "system prompt",
            &["tool1".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        let reasons = tracker.check_and_update(
            "system prompt",
            &["tool1".to_string(), "tool2".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        assert_eq!(reasons.len(), 1);
        assert!(matches!(reasons[0], CacheBreakReason::ToolsChanged));
    }

    #[test]
    fn test_model_change_breaks_cache() {
        let mut tracker = CacheTracker::new();

        tracker.check_and_update(
            "system prompt",
            &["tool1".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        let reasons = tracker.check_and_update(
            "system prompt",
            &["tool1".to_string()],
            "claude-sonnet-4",
            "anthropic",
        );

        assert_eq!(reasons.len(), 1);
        assert!(matches!(reasons[0], CacheBreakReason::ModelChanged));
    }

    #[test]
    fn test_multiple_changes() {
        let mut tracker = CacheTracker::new();

        tracker.check_and_update(
            "system prompt v1",
            &["tool1".to_string()],
            "claude-opus-4",
            "anthropic",
        );

        let reasons = tracker.check_and_update(
            "system prompt v2",
            &["tool2".to_string()],
            "claude-sonnet-4",
            "openai",
        );

        assert_eq!(reasons.len(), 4);
    }
}
