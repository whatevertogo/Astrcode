use std::env;

use astrcode_core::{CancelToken, ToolContext, DEFAULT_MAX_OUTPUT_SIZE};

#[test]
fn cancel_token_clone_observes_shared_cancellation() {
    let token = CancelToken::new();
    let cloned = token.clone();

    assert!(!token.is_cancelled());
    cloned.cancel();

    assert!(token.is_cancelled());
    assert!(cloned.is_cancelled());
}

#[test]
fn tool_context_preserves_explicit_execution_roots() {
    let working_dir = env::temp_dir().join("astrcode-working-dir");
    let ctx = ToolContext {
        session_id: "session-1".to_string(),
        working_dir: working_dir.clone(),
        cancel: CancelToken::new(),
        max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
    };

    assert_eq!(ctx.session_id, "session-1");
    assert_eq!(ctx.working_dir, working_dir);
    assert!(!ctx.cancel.is_cancelled());
}
