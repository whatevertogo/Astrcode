#!/bin/bash
# Pre-push check script
# Runs frontend lint before allowing git push

echo '{"systemMessage": "⏳ Running pre-push checks...", "continue": true}'

# Run frontend checks
if cd frontend && npm run lint; then
    echo '{"systemMessage": "✅ Pre-push checks passed!", "continue": true}'
else
    echo '{"systemMessage": "❌ Pre-push checks failed. Fix the issues before pushing.", "continue": false, "stopReason": "Pre-push checks failed. Please fix the errors and try again."}'
fi
