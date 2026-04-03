
const fs = require('fs');

const messageListCSS = \
.list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 24px max(24px, calc(50vw - 400px)) 24px;
  display: flex;
  flex-direction: column;
  gap: 24px;
  background: transparent;
  scroll-behavior: smooth;
}

.empty {
  width: 100%;
  max-width: 800px;
  color: var(--text-secondary);
  font-size: 14px;
  text-align: center;
  margin: 60px auto 0;
  padding: 24px 28px;
}

.messageRow {
  width: 100%;
  max-width: 800px;
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
}

.renderErrorTitle {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 6px;
}

.renderErrorMeta {
  font-size: 12px;
  color: #b88585;
  margin-bottom: 8px;
}

.renderErrorBody {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 12px;
  line-height: 1.5;
}

@media (max-width: 640px) {
  .list {
    padding: 16px 16px 8px;
    gap: 16px;
  }
}
\;

fs.writeFileSync('frontend/src/components/Chat/MessageList.module.css', messageListCSS);
console.log('updated MessageList');

