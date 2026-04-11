# Feature Specification: 前端统一样式技术栈迁移

**Feature Branch**: `007-tailwind-migration`
**Created**: 2026-04-10
**Status**: Draft
**Input**: User description: "前端需要重构为tailwind技术栈而不是混合cssmodules和tailwind，项目保持良好和统一"
**Decision**: 本特性以“单一 Tailwind 技术栈”作为完成线（见 FR-003），不保留 CSS Module 兼容层、过渡层或双轨实现；允许为了获得更干净的 Tailwind 实现调整非公开 DOM 结构，但不得改变用户可感知的视觉、交互、可访问性语义和桌面端滚动行为。

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 样式技术栈统一且可审计 (Priority: P1)

开发者或审查者在检查任意前端组件时，只看到 Tailwind 类名和必要的 `cn(...)` 组合，不再看到 `.module.css` 导入、`styles.xxx` 引用或任何“为了兼容旧样式保留的中间层”。

**Why this priority**: 这是本特性的核心目标。如果仓库里仍然同时存在 CSS Module 和 Tailwind，两套心智模型、两套排障路径、两套维护方式就仍然存在，迁移的主要收益不会兑现。

**Independent Test**: 在 `frontend` 下执行源码检查，确认 `frontend/src/**/*.module.css` 为 0，`rg -n '\.module\.css|styles\.' frontend/src -g '*.ts' -g '*.tsx'` 无结果，且关键页面的视觉和交互与迁移前保持一致。

**Acceptance Scenarios**:

1. **Given** 任意组件源码文件, **When** 开发者查看实现, **Then** 静态样式仅通过 Tailwind 类名和 `cn(...)` 表达
2. **Given** 代码仓库, **When** 执行源码搜索, **Then** 不存在任何 `.module.css` 文件、`.module.css` 导入或 `styles.xxx` 引用
3. **Given** 关键页面和弹窗, **When** 与迁移前进行对比, **Then** 不出现肉眼可见的视觉回归和状态丢失
4. **Given** 迁移完成后的仓库, **When** 审查实现方式, **Then** 不存在 CSS Module 到 Tailwind 的兼容映射、桥接组件或临时双写样式

---

### User Story 2 - 新组件开发只需一种样式心智模型 (Priority: P2)

开发者创建新组件时，只需要使用 Tailwind v4、`cn(...)` 和统一的设计令牌，不需要在 CSS Module、全局 CSS、行内样式之间来回判断“该写在哪”。

**Why this priority**: P1 解决的是历史包袱，P2 解决的是未来的持续收益。只有新代码也被约束到同一轨道上，统一技术栈才不会在几周后重新退化。

**Independent Test**: 新建一个包含按钮、输入框、状态标签和弹层的测试组件，全程不创建组件级 CSS 文件，仅依赖 Tailwind 类名、主题令牌和必要的动态 `style`，即可完成样式实现。

**Acceptance Scenarios**:

1. **Given** 一个新组件, **When** 开发者实现其样式, **Then** 不需要新建任何组件级 CSS 文件
2. **Given** 颜色、阴影、间距、圆角等设计令牌, **When** 开发者编写 Tailwind 类名, **Then** 可以通过 Tailwind 主题系统访问这些令牌
3. **Given** 条件样式需求, **When** 开发者组合类名, **Then** 通过 `cn(...)` 完成，不引入额外样式抽象层

---

### User Story 3 - 样式维护边界清晰且修改路径单一 (Priority: P3)

开发者修改组件样式时，能明确判断应该修改组件 TSX 中的 Tailwind 类名，还是修改全局设计令牌，而不是在多个文件和多个层级之间碰运气。

**Why this priority**: 统一技术栈不只是“删文件”，还包括建立稳定边界。没有清晰边界，CSS Module 会消失，但新的混乱会以“少量全局 CSS”或“大量行内样式”的形式回来。

**Independent Test**: 选取 `InputBar`、`SettingsModal`、`Sidebar` 三类不同复杂度组件，分别做一次局部样式调整和一次主题令牌调整，确认不需要恢复组件级 CSS 文件，也不需要把组件特定规则塞回 `index.css`。

**Acceptance Scenarios**:

1. **Given** 组件局部样式调整需求, **When** 开发者修改样式, **Then** 修改发生在对应组件 TSX 中
2. **Given** 全局视觉令牌变更需求, **When** 修改 Tailwind 主题或根变量, **Then** 相关组件自动反映变化
3. **Given** 审查 `index.css`, **When** 检查其中规则, **Then** 不应出现组件专属选择器、深层后代选择器或“补 Tailwind 漏洞”的临时规则

---

### Edge Cases

- 原 CSS Module 中使用了 `:hover`、`:focus-visible`、`:focus-within`、`:disabled`、`:nth-child`、`::before`、`::after` 等伪类和伪元素，迁移后需确保状态和语义不丢失
- 原样式依赖 `position: sticky`、`overflow`、`min-height: 0`、`flex-shrink`、滚动容器层级等布局细节，迁移后不能破坏桌面端滚动与吸底行为
- 弹窗、下拉菜单、上下文菜单等浮层组件依赖层级、遮罩、焦点管理和 portal 行为，迁移后不能出现点击穿透、滚动穿透或遮挡错误
- Markdown、代码块、JSON 展示、终端输出等富文本区域包含大量嵌套节点，迁移后需保持可读性、滚动体验和等宽字体表现
- 动态宽高、运行时计算位置、颜色预览等少数无法稳定静态类化的场景，需要定义哪些情况允许使用 `style`
- `index.css` 中当前存在设计令牌、基础 reset、滚动条样式和动画定义，迁移时需明确哪些保留为全局能力，哪些必须下沉到 Tailwind 主题或组件类名
- 迁移后构建产物中的 CSS 体积不应因类名堆叠或全局兜底样式而明显膨胀

## Out of Scope

- 不引入新的样式方案（如 CSS-in-JS、styled-components、UnoCSS 等）
- 不为了“兼容旧样式”保留 `.module.css` 文件、`styles` 映射对象或任何桥接层
- 不修改后端、协议层、运行时逻辑；仅允许做保持视觉和交互一致所必需的前端结构调整
- 不把组件专属样式迁移到新的全局 CSS 文件中，以“换地方继续写旧模式”

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 以 `frontend/src/**/*.module.css` 为统计口径，当前基线中的 17 个 `.module.css` 文件必须全部删除，迁移完成后数量为 0
- **FR-002**: 所有组件文件中的 `.module.css` 导入和 `styles.xxx` 引用必须移除；若 `frontend/src/vite-env.d.ts` 中仅剩 CSS Module 类型声明且无使用方，必须一并删除
- **FR-003**: 不允许保留 CSS Module 兼容层、Tailwind 包装层、旧类名到新类名的映射对象、双写样式或任何过渡性实现
- **FR-004**: 组件的静态样式必须直接写在 TSX 中，并通过 Tailwind 类名与 `cn(...)` 组合表达；`style` 仅允许用于运行时计算值、动态坐标/尺寸、动态 CSS 变量赋值，或当前 Tailwind 无法稳定表达的值
- **FR-005**: 现有设计令牌（颜色、阴影、圆角、间距、字体等）必须通过 Tailwind v4 的主题机制暴露，并形成可审计的令牌映射清单；该清单至少记录 `sourceVar`、`tailwindAlias`、`usageScope` 和一个实际消费组件，新增视觉值默认应先沉淀为令牌，而不是散落为硬编码字面量
- **FR-006**: `frontend/src/index.css` 必须收敛为全局边界文件，只允许保留以下内容：reset、根节点布局、设计令牌定义、滚动条样式、共享 keyframes、共享 utility、第三方控件必要覆盖；不得保留与具体组件绑定的选择器和结构样式
- **FR-007**: 原 CSS Module 中的伪类、伪元素、层级选择器和状态样式，优先使用 Tailwind v4 的变体、任意变体、`group`、`peer`、`data-*`、`aria-*` 等能力表达；只有在 Tailwind 无法稳定表达时，才允许抽成命名清晰的全局 utility
- **FR-008**: 动画效果必须迁移到共享的 Tailwind 动画能力中；若保留全局 `@keyframes`，其用途必须是共享动画基础能力，而不是某个单独组件的隐藏样式容器
- **FR-009**: 迁移后以下关键界面必须保持视觉和交互一致：Sidebar、ProjectItem、SessionItem、Chat 主界面、AssistantMessage、UserMessage、InputBar、CompactMessage、MessageList、ToolCallBlock、ToolJsonView、SubRunBlock、TopBar、PromptMetricsMessage、SettingsModal、NewProjectModal、ConfirmDialog
- **FR-010**: 所有交互状态必须保持一致，包括 hover、active、focus-visible、disabled、selected、loading、error、streaming、展开/收起和上下文菜单状态；验收时必须按组件-状态矩阵逐项记录
- **FR-011**: 迁移不得破坏桌面端窗口内的滚动、吸底、粘性头部/底部、遮罩滚动锁定、文本选择和输入体验；不得引入依赖宿主容器 hack 的样式修复
- **FR-012**: 允许为了更干净的 Tailwind 实现调整非公开 DOM 结构，但不得改变组件对外行为、语义标签职责、可访问性关系和现有交互契约
- **FR-013**: 迁移完成后必须通过仓库级 Rust 校验（`cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`）以及前端校验（`cd frontend && npm run typecheck && npm run build && npm run lint && npm test`）
- **FR-014**: 迁移后的生产构建 CSS 体积不得相较迁移前基线出现显著增长；若超过 10%，必须在实施说明中给出具体原因与收益说明
- **FR-015**: 已经是 Tailwind 实现的组件不得在迁移过程中被回退为新的全局 CSS 或行内样式堆砌写法，统一方向必须是“更多 Tailwind、不是更多例外”

### Key Entities

- **样式实现边界**: 组件 TSX、Tailwind 主题、`index.css` 三者的职责划分，是本次迁移最重要的架构约束
- **设计令牌体系**: 根变量和 Tailwind 主题中承载的颜色、间距、阴影、圆角、字体等语义值，是统一视觉语言的基础
- **组件样式实现**: 现有使用 CSS Module 的组件及其状态样式，是主要迁移对象
- **全局能力样式**: reset、滚动条、共享动画、共享 utility、第三方必要覆盖，是迁移后仍允许存在的全局 CSS
- **验收基线**: `.module.css` 文件数量、源码搜索结果、关键界面截图、构建结果和 CSS 产物大小，共同构成迁移完成的判断依据

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: `rg --files frontend/src -g '*.module.css'` 返回 0 条结果
- **SC-002**: `rg -n '\.module\.css|styles\.' frontend/src -g '*.ts' -g '*.tsx'` 返回 0 条结果，且不存在继续为 CSS Module 服务的残留类型声明
- **SC-003**: `frontend/src/index.css` 中不再包含任何与具体组件名称、组件层级结构或组件局部状态绑定的规则
- **SC-004**: `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test` 与 `cd frontend && npm run typecheck && npm run build && npm run lint && npm test` 全部通过
- **SC-005**: FR-009 列出的全部关键界面和组件在同一环境下对比迁移前截图，无肉眼可见回归
- **SC-006**: 对代表性组件（至少 `InputBar` 或 `AssistantMessage`）进行一次样式微调时，不需要新建或修改任何组件级 CSS 文件
- **SC-007**: 生产构建的主 CSS 产物大小相较迁移前基线增长不超过 10%，或已附带可审计的收益说明
- **SC-008**: 设计令牌映射清单完整存在，覆盖 `frontend/src/index.css` 中当前共享颜色、阴影、布局与终端相关变量，并且每类令牌至少有一个消费组件被记录

## Assumptions

- Tailwind CSS v4、`clsx`、`tailwind-merge` 和现有 `cn(...)` 工具足以覆盖当前组件的绝大部分样式表达需求
- 当前 `frontend/src/index.css` 中的设计令牌将继续作为底层视觉语义来源，但其消费入口应收敛到 Tailwind 主题和少量全局能力
- 本项目不需要为 CSS Module 保留向后兼容能力，因此旧文件、旧导入和旧类型声明可以在同一特性交付中一次性删除
- 实施过程可以按组件分批迁移，但该特性的“完成”标准是仓库内 `.module.css` 清零，而不是局部完成
- 少量全局 CSS 是允许的，但它们必须是共享能力，不得重新演化为“换了文件名的 CSS Module”
