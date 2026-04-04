#!/bin/bash
# Pre-push check script
# Runs cargo check, cargo test, and npm typecheck before allowing git push

echo '{"systemMessage": "⏳ Running pre-push checks...", "continue": true}'

# Run checks
if cargo check --workspace && \
   cargo test --workspace --exclude astrcode --lib && \
   cd frontend && npm run typecheck; then
    echo '{"systemMessage": "✅ Pre-push checks passed!", "continue": true}'
else
    echo '{"systemMessage": "❌ Pre-push checks failed. Fix the issues before pushing.", "continue": false, "stopReason": "Pre-push checks failed. Please fix the errors and try again."}'
fi
