# Research: 前端统一样式技术栈迁移

## Decision 1: 设计令牌继续以 `index.css` 根变量为底层真相，并通过 Tailwind v4 主题暴露

**Rationale**:
- 当前项目已经在 `frontend/src/index.css` 中集中维护颜色、阴影和布局变量，直接废弃会制造大量无业务价值的重命名噪音。
- Tailwind v4 已在项目中通过 `@tailwindcss/vite` 启用，适合把现有变量继续作为底层设计令牌，再在主题层提供一致的消费入口。
- 这种做法可以同时满足“组件只写 Tailwind 类名”和“主题值有单一事实源”。

**Alternatives considered**:
- 直接在 TSX 中使用任意值字面量：实现快，但会把同一视觉语义散落到多个组件，后续无法统一治理。
- 新建独立 `tailwind.config.*` 重新定义整套主题：在 v4 项目里收益有限，还会引入新的配置分叉。

## Decision 2: 组件静态样式全部进入 TSX，条件类名统一通过 `cn(...)` 组合

**Rationale**:
- 仓库已经存在 `frontend/src/lib/utils.ts` 中的 `cn(...)`，这是现成且一致的条件类名入口。
- 统一到 TSX 后，组件维护路径会变成“改结构与样式都在一个地方”，符合此次迁移的核心收益。
- 这也天然防止“删掉 CSS Module 后又长出一套字符串拼接私货”。

**Alternatives considered**:
- 保留少量 CSS Module 作为“复杂组件例外”：会直接破坏统一目标，且例外会越来越多。
- 把状态样式抽成新的 helper CSS 文件：只是换文件名延续旧模式，不解决边界问题。

## Decision 3: `style` 只允许承载运行时动态值，不承担静态视觉表达

**Rationale**:
- 当前代码中确有少量合理的动态样式，例如侧边栏宽度和局部定位，这类值本来就来自运行时状态。
- 如果不明确限制，迁移后最容易退化成“Tailwind + 大量 inline style”的新混合栈。
- 把 `style` 约束为动态值，可以同时保留灵活性和审查可读性。

**Alternatives considered**:
- 完全禁止 `style`：会迫使动态尺寸和运行时坐标走不自然的绕路实现。
- 对 `style` 不设边界：短期方便，长期会重新制造不可搜索、不可复用的样式碎片。

## Decision 4: 全局 CSS 仅保留共享能力，不承载组件专属规则

**Rationale**:
- 当前 `frontend/src/index.css` 已经天然承担 reset、根布局、滚动条样式和共享 keyframes，这些是合理的全局能力。
- Chat 富文本和复杂内容区确实可能需要共享 utility，但这些 utility 必须是语义清晰、跨组件可复用的能力，而不是把局部样式“藏”回全局。
- 明确边界后，review 时可以快速判断某条规则是否越界。

**Alternatives considered**:
- 追求“零全局 CSS”：会让 reset、滚动条和共享动画无处安放，也不符合当前项目结构。
- 容许把复杂组件样式迁到 `index.css`：这是最容易发生的伪迁移，必须明确禁止。

## Decision 5: 迁移顺序采用“先边界、后组件、最后清理”的分批策略

**Rationale**:
- 先固化主题和全局边界，再迁组件，可以减少每个组件单独发明模式的概率。
- 壳层、弹窗、Sidebar、Chat 渲染区复杂度递增，按这个顺序更利于提炼共用模式。
- 最终以源码搜索和测试收尾，能确保没有漏网残留。

**Alternatives considered**:
- 一次性大爆改全部组件：上下文切换少，但 review 成本高、回归定位困难。
- 按文件随意迁移：容易导致不同组件采用不同风格，最后又需要二次统一。
