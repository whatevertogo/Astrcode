# Quickstart: 前端统一样式技术栈迁移

## 1. 实施前准备

1. 进入前端目录：

```powershell
cd D:\GitObjectsOwn\Astrcode\frontend
```

2. 记录迁移前基线：

```powershell
rg --files src -g '*.module.css'
npm run build
```

3. 对 FR-009 中全部组件保留迁移前截图或等效可对比环境，并记录到下面的矩阵中。

## 2. 迁移前视觉基线矩阵

| 组件组 | 组件 | 基线证据 | 迁移后对比结果 |
|--------|------|----------|----------------|
| dialogs | ConfirmDialog | 待记录 | 待记录 |
| dialogs | NewProjectModal | 待记录 | 待记录 |
| dialogs | SettingsModal | 待记录 | 待记录 |
| sidebar | Sidebar | 待记录 | 待记录 |
| sidebar | ProjectItem | 待记录 | 待记录 |
| sidebar | SessionItem | 待记录 | 待记录 |
| chat-shell | Chat 主界面 | 待记录 | 待记录 |
| chat-shell | TopBar | 待记录 | 待记录 |
| chat-shell | InputBar | 待记录 | 待记录 |
| chat-shell | CompactMessage | 待记录 | 待记录 |
| chat-shell | PromptMetricsMessage | 待记录 | 待记录 |
| chat-render | AssistantMessage | 待记录 | 待记录 |
| chat-render | UserMessage | 待记录 | 待记录 |
| chat-render | MessageList | 待记录 | 待记录 |
| chat-render | ToolCallBlock | 待记录 | 待记录 |
| chat-render | ToolJsonView | 待记录 | 待记录 |
| chat-render | SubRunBlock | 待记录 | 待记录 |

## 3. 设计令牌映射清单

| sourceVar | provisional tailwindAlias | category | usageScope | consumer example |
|-----------|----------------------------|----------|------------|------------------|
| `--app-bg` | `app-bg` | color | global | 待记录 |
| `--panel-bg` | `panel-bg` | color | component | 待记录 |
| `--sidebar-bg` | `sidebar-bg` | color | component | 待记录 |
| `--surface` | `surface` | color | both | 待记录 |
| `--surface-muted` | `surface-muted` | color | component | 待记录 |
| `--surface-soft` | `surface-soft` | color | both | 待记录 |
| `--border` | `border` | color | both | 待记录 |
| `--border-strong` | `border-strong` | color | both | 待记录 |
| `--text-primary` | `text-primary` | color | both | 待记录 |
| `--text-secondary` | `text-secondary` | color | both | 待记录 |
| `--text-muted` | `text-muted` | color | component | 待记录 |
| `--text-faint` | `text-faint` | color | component | 待记录 |
| `--accent-strong` | `accent-strong` | color | both | 待记录 |
| `--accent-soft` | `accent-soft` | color | component | 待记录 |
| `--success-soft` | `success-soft` | color | component | 待记录 |
| `--success` | `success` | color | both | 待记录 |
| `--danger-soft` | `danger-soft` | color | component | 待记录 |
| `--danger` | `danger` | color | both | 待记录 |
| `--shadow-soft` | `shadow-soft` | shadow | both | 待记录 |
| `--phase-idle` | `phase-idle` | color | component | 待记录 |
| `--phase-thinking` | `phase-thinking` | color | component | 待记录 |
| `--phase-calling-tool` | `phase-calling-tool` | color | component | 待记录 |
| `--phase-streaming` | `phase-streaming` | color | component | 待记录 |
| `--phase-interrupted` | `phase-interrupted` | color | component | 待记录 |
| `--phase-done` | `phase-done` | color | component | 待记录 |
| `--terminal-border` | `terminal-border` | color | component | 待记录 |
| `--terminal-bg-from` | `terminal-bg-from` | color | component | 待记录 |
| `--terminal-bg-to` | `terminal-bg-to` | color | component | 待记录 |
| `--terminal-text` | `terminal-text` | color | component | 待记录 |
| `--terminal-error` | `terminal-error` | color | component | 待记录 |
| `--rename-input-bg` | `rename-input-bg` | color | component | 待记录 |
| `--rename-input-border` | `rename-input-border` | color | component | 待记录 |
| `--rename-input-color` | `rename-input-color` | color | component | 待记录 |

## 4. 推荐实施顺序

1. 整理 `src/index.css`
   - 保留 reset、根布局、滚动条、共享 keyframes、共享 utility
   - 补齐 Tailwind 主题令牌入口
   - 删除组件专属规则
2. 迁移弹窗与壳层组件
3. 迁移 Sidebar 组件组
4. 迁移 Chat 壳层与输入区
5. 迁移 Chat 富文本与复杂展示组件
6. 删除残留的 CSS Module 类型声明和未使用样式

## 5. 组件迁移检查清单

- 删除对应 `.module.css`
- 移除 `.module.css` 导入
- 用 Tailwind 类名替代所有 `styles.xxx`
- 条件类名统一使用 `cn(...)`
- 只在动态尺寸/坐标/变量注入时使用 `style`
- 确认 hover、focus-visible、disabled、expanded 等状态没有丢失
- 确认移动端断点和桌面端滚动行为未回归

## 6. 组件-状态验收矩阵

| 组件 | 必验状态 |
|------|----------|
| ConfirmDialog | default, hover, focus-visible, danger |
| NewProjectModal | default, focus-visible, disabled |
| SettingsModal | default, loading, success, error, disabled |
| Sidebar | default, resize, hover |
| ProjectItem | default, hover, expanded, context menu |
| SessionItem | default, active, hover, context menu |
| Chat 主界面 | default, sidebar open/closed |
| TopBar | default, hover, breadcrumb navigation |
| InputBar | default, focus-visible, disabled, submit-ready |
| CompactMessage | default |
| PromptMetricsMessage | default |
| AssistantMessage | default, streaming, expanded thinking, collapsed thinking |
| UserMessage | default |
| MessageList | default, empty, sticky scroll |
| ToolCallBlock | running, completed, error, expanded, collapsed |
| ToolJsonView | default, expanded, collapsed |
| SubRunBlock | running, completed, failed, expanded, collapsed |

## 7. 完成后验证

```powershell
cd D:\GitObjectsOwn\Astrcode
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cd D:\GitObjectsOwn\Astrcode\frontend
rg --files src -g '*.module.css'
rg -n '\.module\.css|styles\.' src -g '*.ts' -g '*.tsx'
npm run typecheck
npm run build
npm run lint
npm test
```

## 8. Review 时重点关注

- `index.css` 有没有重新长出组件专属样式
- `style` 是否被滥用为静态视觉实现
- 是否出现“为了省事”保留的桥接层或临时 class 映射
- 设计令牌映射清单是否完整，且每类至少有一个真实消费组件
- FR-009 全部组件是否都有视觉对比记录
- FR-010 组件-状态矩阵是否逐项勾完
- 富文本、代码块、弹窗和侧边栏滚动是否与迁移前一致
