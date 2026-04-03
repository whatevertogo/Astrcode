const fs = require('fs');

const userMsgCSS = \
.wrapper {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-bottom: 24px;
  animation: messageEnter 280ms cubic-bezier(0.2, 1, 0.32, 1);
}

.avatar {
  display: none;
}

.body {
  max-width: 75%;
}

.text {
  display: inline-block;
  padding: 12px 18px;
  background: #f3f4f6;
  color: var(--text-primary);
  font-size: 16px;
  line-height: 1.5;
  border-radius: 20px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

@keyframes messageEnter {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

@media (max-width: 640px) {
  .body { max-width: 90%; }
}
\;

const assistantMsgCSS = \
.wrapper {
  display: flex;
  align-items: flex-start;
  gap: 16px;
  margin-bottom: 32px;
  animation: messageEnter 320ms cubic-bezier(0.2, 1, 0.32, 1);
}

.avatar {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-secondary);
  border-radius: 6px;
  flex-shrink: 0;
  margin-top: 2px;
}

.avatar svg {
  width: 16px;
  height: 16px;
}

.body {
  flex: 1;
  min-width: 0;
}

.content {
  position: relative;
  padding: 0;
  font-size: 16px;
  line-height: 1.6;
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.streamingText {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.fallbackText {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font: inherit;
  color: inherit;
}

.thinkingBlock {
  margin: 0 0 16px;
  border-left: 3px solid #e5e5e5;
  padding-left: 14px;
  overflow: hidden;
}

.thinkingSummary {
  cursor: pointer;
  padding: 4px 0;
  color: var(--text-secondary);
  font-size: 14px;
  font-weight: 500;
  user-select: none;
}

.thinkingContent {
  margin: 0;
  padding: 4px 0 12px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 14px;
  color: var(--text-secondary);
}

.content p { margin: 0 0 16px; }
.content p:last-child { margin-bottom: 0; }
.content ul, .content ol { margin: 8px 0 16px; padding-left: 24px; }
.content li { margin-bottom: 6px; }
.content strong { font-weight: 600; color: #000; }

.codeBlock {
  background: #f9f9f9;
  border: 1px solid #e5e5e5;
  border-radius: 8px;
  padding: 14px 16px;
  overflow-x: auto;
  margin: 16px 0;
  font-size: 14px;
  line-height: 1.5;
  font-family: ui-monospace, SFMono-Regular, Consolas, 'Liberation Mono', Menlo, monospace;
}

.inlineCode {
  background: #f3f4f6;
  border-radius: 4px;
  padding: 2px 6px;
  font-size: 14px;
  color: #333;
  font-family: ui-monospace, SFMono-Regular, Consolas, 'Liberation Mono', Menlo, monospace;
}

.cursor {
  display: inline-block;
  color: #000;
  animation: blink 1s step-end infinite;
  margin-left: 2px;
}

@keyframes messageEnter {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

@keyframes blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0; }
}
\;

const messageListCSS = \
.list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 16px 32px 48px;
  display: flex;
  flex-direction: column;
  background: transparent;
  scroll-behavior: smooth;
}

.empty {
  width: 100%;
  max-width: 900px;
  color: var(--text-secondary);
  font-size: 14px;
  text-align: center;
  margin: 60px auto 0;
  padding: 24px 28px;
}

.messageRow {
  width: 100%;
  max-width: 900px;
  margin: 0 auto;
  content-visibility: auto;
  contain-intrinsic-size: 124px;
}

.renderError {
  align-self: stretch;
  border: 1px solid var(--danger-soft);
  background: #fffdfd;
  color: var(--danger);
  border-radius: 8px;
  padding: 12px 16px;
  margin-bottom: 24px;
}

.renderErrorTitle { font-size: 14px; font-weight: 600; margin-bottom: 6px; }
.renderErrorMeta { font-size: 12px; color: #b88585; margin-bottom: 8px; }
.renderErrorBody { margin: 0; white-space: pre-wrap; font-size: 13px; line-height: 1.5; }
\;

fs.writeFileSync('src/components/Chat/UserMessage.module.css', userMsgCSS);
fs.writeFileSync('src/components/Chat/AssistantMessage.module.css', assistantMsgCSS);
fs.writeFileSync('src/components/Chat/MessageList.module.css', messageListCSS);
console.log('Update Complete');
