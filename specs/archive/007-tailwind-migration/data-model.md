# Data Model: 前端统一样式技术栈迁移

## Entity: DesignToken

**Description**: 视觉语义的底层值，来源于 `frontend/src/index.css` 的根变量，并通过 Tailwind 主题机制暴露给组件消费。

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | 令牌名称，如 `app-bg`、`border-strong`、`shadow-soft` |
| `category` | enum(`color`,`spacing`,`radius`,`shadow`,`font`,`layout`,`motion`) | 令牌类别 |
| `sourceVar` | string | 根变量名，如 `--app-bg` |
| `tailwindAlias` | string | 在 Tailwind 中的消费别名或映射方式 |
| `usageScope` | enum(`global`,`component`,`both`) | 允许的消费范围 |
| `status` | enum(`existing`,`normalized`,`new`) | 令牌在迁移中的归一化状态 |

### Validation Rules

- 新增视觉值优先映射到现有令牌；只有复用价值明确时才允许新增 `DesignToken`
- 组件不得直接复制同一语义的硬编码字面量替代令牌
- `sourceVar` 和 `tailwindAlias` 必须一一对应，避免双重真相源

## Entity: StylingBoundaryRule

**Description**: 样式应落在哪一层的约束规则，用于指导实现和 code review。

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `ruleId` | string | 规则标识，如 `component-static-tailwind-only` |
| `layer` | enum(`component`,`theme`,`global`,`runtime-style`) | 规则所属层 |
| `allowedPatterns` | string[] | 允许的实现模式 |
| `forbiddenPatterns` | string[] | 禁止的实现模式 |
| `enforcementSignal` | string[] | 验证方式，如源码搜索、人工审查、构建校验 |

### Validation Rules

- `component` 层必须禁止 `.module.css` 导入和 `styles.xxx`
- `global` 层必须禁止组件名、深层后代结构和局部状态绑定规则
- `runtime-style` 层只允许动态值，不允许承载静态视觉方案

## Entity: MigrationUnit

**Description**: 一组需要从 CSS Module 迁移到 Tailwind 的组件单元，通常由一个 `.tsx` 文件和一个对应 `.module.css` 文件组成。

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `componentName` | string | 组件名称 |
| `tsxPath` | string | 组件实现路径 |
| `cssModulePath` | string | 旧样式文件路径 |
| `surface` | enum(`shell`,`dialog`,`sidebar`,`chat-render`,`chat-input`,`utility`) | 所属界面区域 |
| `stateComplexity` | enum(`low`,`medium`,`high`) | 状态样式复杂度 |
| `needsDynamicStyle` | boolean | 是否包含合理的运行时 `style` 场景 |
| `requiresVisualCheck` | boolean | 是否属于关键界面，必须做视觉回归检查 |
| `status` | enum(`pending`,`in-progress`,`migrated`,`verified`) | 迁移状态 |

### State Transitions

- `pending -> in-progress`: 开始迁移该组件
- `in-progress -> migrated`: TSX 改为 Tailwind 且旧 CSS Module 删除
- `migrated -> verified`: 通过源码搜索、构建和界面检查
- 任一阶段若发现边界违规，可退回 `in-progress`

## Entity: GlobalStyleAsset

**Description**: 迁移后仍允许存在于 `index.css` 中的共享样式资产。

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `assetName` | string | 资产名，如 `scrollbar`、`blink`、`spin` |
| `assetType` | enum(`reset`,`root-layout`,`token`,`scrollbar`,`keyframes`,`utility`,`third-party-override`) | 资产类型 |
| `sharedConsumers` | string[] | 消费该资产的组件或场景 |
| `justification` | string | 必须留在全局层的原因 |

### Validation Rules

- `GlobalStyleAsset` 必须服务于多个组件或全局行为，不能只为单一组件存在
- 若某条规则能自然落回 Tailwind 类名，则不得继续保留为 `GlobalStyleAsset`

## Appendix A: Migration Unit Inventory

| 组件 | TSX Path | CSS Module Path | Surface | State Complexity | Needs Dynamic Style | Requires Visual Check |
|------|----------|-----------------|---------|------------------|---------------------|-----------------------|
| ConfirmDialog | `frontend/src/components/ConfirmDialog.tsx` | `frontend/src/components/ConfirmDialog.module.css` | dialog | medium | false | true |
| NewProjectModal | `frontend/src/components/NewProjectModal.tsx` | `frontend/src/components/NewProjectModal.module.css` | dialog | medium | false | true |
| SettingsModal | `frontend/src/components/Settings/SettingsModal.tsx` | `frontend/src/components/Settings/SettingsModal.module.css` | dialog | high | false | true |
| Sidebar | `frontend/src/components/Sidebar/index.tsx` | `frontend/src/components/Sidebar/Sidebar.module.css` | sidebar | medium | false | true |
| ProjectItem | `frontend/src/components/Sidebar/ProjectItem.tsx` | `frontend/src/components/Sidebar/ProjectItem.module.css` | sidebar | high | false | true |
| SessionItem | `frontend/src/components/Sidebar/SessionItem.tsx` | `frontend/src/components/Sidebar/SessionItem.module.css` | sidebar | high | true | true |
| Chat | `frontend/src/components/Chat/index.tsx` | `frontend/src/components/Chat/Chat.module.css` | chat-input | medium | false | true |
| TopBar | `frontend/src/components/Chat/TopBar.tsx` | `frontend/src/components/Chat/TopBar.module.css` | chat-input | medium | false | true |
| InputBar | `frontend/src/components/Chat/InputBar.tsx` | `frontend/src/components/Chat/InputBar.module.css` | chat-input | high | true | true |
| CompactMessage | `frontend/src/components/Chat/CompactMessage.tsx` | `frontend/src/components/Chat/CompactMessage.module.css` | chat-input | low | false | true |
| PromptMetricsMessage | `frontend/src/components/Chat/PromptMetricsMessage.tsx` | `frontend/src/components/Chat/PromptMetricsMessage.module.css` | chat-input | low | false | true |
| AssistantMessage | `frontend/src/components/Chat/AssistantMessage.tsx` | `frontend/src/components/Chat/AssistantMessage.module.css` | chat-render | high | false | true |
| UserMessage | `frontend/src/components/Chat/UserMessage.tsx` | `frontend/src/components/Chat/UserMessage.module.css` | chat-render | low | false | true |
| MessageList | `frontend/src/components/Chat/MessageList.tsx` | `frontend/src/components/Chat/MessageList.module.css` | chat-render | high | false | true |
| ToolCallBlock | `frontend/src/components/Chat/ToolCallBlock.tsx` | `frontend/src/components/Chat/ToolCallBlock.module.css` | chat-render | high | false | true |
| ToolJsonView | `frontend/src/components/Chat/ToolJsonView.tsx` | `frontend/src/components/Chat/ToolJsonView.module.css` | chat-render | medium | false | true |
| SubRunBlock | `frontend/src/components/Chat/SubRunBlock.tsx` | `frontend/src/components/Chat/SubRunBlock.module.css` | chat-render | high | false | true |

## Appendix B: Design Token Mapping Inventory

| sourceVar | provisional tailwindAlias | category | usageScope | consumer example |
|-----------|----------------------------|----------|------------|------------------|
| `--app-bg` | `app-bg` | color | global | `frontend/src/App.tsx` |
| `--panel-bg` | `panel-bg` | color | component | 待补充 |
| `--sidebar-bg` | `sidebar-bg` | color | component | `frontend/src/components/Sidebar/index.tsx` |
| `--surface` | `surface` | color | both | 待补充 |
| `--surface-muted` | `surface-muted` | color | component | 待补充 |
| `--surface-soft` | `surface-soft` | color | both | 待补充 |
| `--border` | `border` | color | both | 待补充 |
| `--border-strong` | `border-strong` | color | both | 待补充 |
| `--text-primary` | `text-primary` | color | both | `frontend/src/App.tsx` |
| `--text-secondary` | `text-secondary` | color | both | 待补充 |
| `--text-muted` | `text-muted` | color | component | 待补充 |
| `--text-faint` | `text-faint` | color | component | 待补充 |
| `--accent-strong` | `accent-strong` | color | both | 待补充 |
| `--accent-soft` | `accent-soft` | color | component | 待补充 |
| `--success-soft` | `success-soft` | color | component | 待补充 |
| `--success` | `success` | color | both | 待补充 |
| `--danger-soft` | `danger-soft` | color | component | 待补充 |
| `--danger` | `danger` | color | both | 待补充 |
| `--shadow-soft` | `shadow-soft` | shadow | both | 待补充 |
| `--phase-idle` | `phase-idle` | color | component | 待补充 |
| `--phase-thinking` | `phase-thinking` | color | component | 待补充 |
| `--phase-calling-tool` | `phase-calling-tool` | color | component | 待补充 |
| `--phase-streaming` | `phase-streaming` | color | component | 待补充 |
| `--phase-interrupted` | `phase-interrupted` | color | component | 待补充 |
| `--phase-done` | `phase-done` | color | component | 待补充 |
| `--terminal-border` | `terminal-border` | color | component | 待补充 |
| `--terminal-bg-from` | `terminal-bg-from` | color | component | 待补充 |
| `--terminal-bg-to` | `terminal-bg-to` | color | component | 待补充 |
| `--terminal-text` | `terminal-text` | color | component | 待补充 |
| `--terminal-error` | `terminal-error` | color | component | 待补充 |
| `--rename-input-bg` | `rename-input-bg` | color | component | 待补充 |
| `--rename-input-border` | `rename-input-border` | color | component | 待补充 |
| `--rename-input-color` | `rename-input-color` | color | component | 待补充 |
