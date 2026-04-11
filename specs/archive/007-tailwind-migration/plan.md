# Implementation Plan: 前端统一样式技术栈迁移

**Branch**: `007-tailwind-migration` | **Date**: 2026-04-10 | **Spec**: [spec.md](D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/spec.md)
**Input**: Feature specification from `D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/spec.md`

## Summary

将 `frontend/src` 下现存的 17 个 CSS Module 全量迁移为 Tailwind v4 实现，并把前端样式架构收敛为三层边界：组件 TSX 负责静态样式与状态组合，`index.css` 只保留全局能力和设计令牌，运行时 `style` 仅用于动态值。迁移过程中允许为获得更干净的 Tailwind 结构调整非公开 DOM，但不保留兼容层，也不接受“把局部样式转存到全局 CSS”的伪迁移方案。

## Technical Context

**Language/Version**: TypeScript 5、React 18、Tailwind CSS 4、Vite 5、Tauri 2 桌面前端  
**Primary Dependencies**: `react`、`@vitejs/plugin-react`、`tailwindcss`、`@tailwindcss/vite`、`clsx`、`tailwind-merge`、`vitest`、`eslint`  
**Storage**: N/A；本特性不引入新持久化，仅调整前端样式实现  
**Testing**: `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`、`npm run typecheck`、`npm run build`、`npm run lint`、`npm test`，辅以源码搜索、令牌映射审计和关键界面对比  
**Target Platform**: Tauri 桌面应用前端，开发态运行于 Vite 浏览器环境，需保持桌面端滚动与布局行为稳定  
**Project Type**: 桌面应用前端子系统  
**Performance Goals**: 保持现有交互与滚动体验，无肉眼可见视觉回归；生产构建主 CSS 体积相较迁移前增长不超过 10%  
**Constraints**: 不需要向后兼容；不得保留 CSS Module 兼容层；不得把组件样式重新塞回全局 CSS；不得破坏桌面端滚动/吸底/遮罩锁定；注释和文档必须使用中文  
**Scale/Scope**: `frontend/src/components` 下 17 个 `.module.css` 文件，覆盖 Sidebar、Chat 富文本渲染、消息列表、弹窗、上下文菜单与设置面板等核心界面

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Durable Truth First**: 通过。本特性仅调整前端样式投影层，不改变 durable 事件、协议 DTO 或历史语义。
- **One Boundary, One Owner**: 通过。计划将样式职责明确拆分为组件 TSX、Tailwind 主题令牌和 `index.css` 全局能力三层，并显式删除 CSS Module 这一重复边界。
- **Protocol Purity, Projection Fidelity**: 通过。本特性不改协议映射、不改 `/history` 或 `/events` 投影。
- **Ownership Over Storage Mode**: 不适用。本特性不涉及 subrun ownership、session mode 或 storage mode。
- **Explicit Migrations, Verifiable Refactors**: 通过。计划包含组件清单、迁移顺序、禁止兼容层原则和具体验证命令。
- **Runtime Robustness**: 通过。本特性不触达 Rust runtime；前端仅调整样式与少量非公开 DOM 结构，不引入新的异步生命周期风险。
- **Observability & Error Visibility**: 通过。本特性不新增错误吞没点；若迁移过程中调整渲染结构，必须保留现有错误显示与加载状态表现。

**Post-Design Re-check**: 通过。Phase 1 产物已经把“样式实现边界”“全局能力边界”“验证合同”文档化，未引入新的跨边界职责混淆，也不需要三层架构迁移文档。

## Project Structure

### Documentation (this feature)

```text
specs/007-tailwind-migration/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── styling-boundary-contract.md
└── tasks.md
```

本特性不触发 `findings.md` / `design-*.md` / `migration.md` 三层文档要求，因为它不改变 durable 事件、公共 runtime surface、跨边界依赖方向，也不删除对外公共接口。

### Source Code (repository root)

```text
frontend/
├── src/
│   ├── App.tsx
│   ├── index.css
│   ├── components/
│   │   ├── Chat/
│   │   ├── Settings/
│   │   ├── Sidebar/
│   │   ├── ConfirmDialog.tsx
│   │   └── NewProjectModal.tsx
│   ├── hooks/
│   ├── lib/
│   │   ├── utils.ts
│   │   └── styles.ts
│   └── store/
├── vite.config.ts
└── package.json

src-tauri/
└── [桌面宿主，作为消费方存在，但本特性不计划修改]
```

**Structure Decision**: 仅在 `frontend/` 内实施迁移，不改后端和 Tauri 宿主。组件样式变更集中在 `frontend/src/components/**`，主题与全局能力收敛到 `frontend/src/index.css`，条件类拼装统一复用 `frontend/src/lib/utils.ts` 中的 `cn(...)`。

## Phase 0: Research Summary

> **Phase 映射**: 本文档的 Phase 0（Research）和 Phase 1（Design）对应 `tasks.md` 的 Phase 1（Setup）和 Phase 2（Foundational）；tasks.md 的 Phase 3-6 是本计划 Design Decision 的具体实施阶段。

1. 设计令牌继续以 `frontend/src/index.css` 中的 `:root` 变量为底层事实源，并通过 Tailwind v4 的 `@theme` 暴露为可消费令牌，避免在 TSX 中散落重复字面量；同时维护可审计的 `sourceVar -> tailwindAlias -> consumer` 映射清单。
2. 复杂状态样式优先使用 Tailwind 变体、任意变体、`group`、`peer`、`data-*`、`aria-*` 以及语义化 class 组合完成；仅当 Tailwind 无法稳定表达时，允许新增共享 utility，而不是回退到组件级 CSS。
3. `index.css` 保留全局 reset、根布局、滚动条样式、共享动画和少量 utility，不承载任何组件专属结构选择器。
4. 迁移顺序按“先边界、后组件”执行：先固化令牌和全局能力，再处理壳层与弹窗，随后处理 Sidebar，最后处理 Chat 区域中最复杂的富文本和交互组件。
5. 验收以源码搜索、仓库级 Rust 校验、前端构建测试、令牌映射审计、组件-状态矩阵和关键界面对比共同判定，避免只看“能跑”而忽略视觉与维护边界退化。

## Phase 1: Design Plan

### Design Decisions

1. **令牌层**: 以 `index.css` 中现有 CSS 变量为视觉真相层，通过 Tailwind 主题别名暴露给组件消费；只有复用价值明确的新值才能新增令牌，并且每个共享令牌都必须进入映射清单。
2. **组件层**: 所有静态样式进入 TSX，条件类名统一走 `cn(...)`；`style` 仅用于动态宽度、运行时坐标、动态 CSS 变量注入等值。
3. **全局层**: `index.css` 只保留 reset、根布局、滚动条、共享 keyframes、共享 utility 和第三方必要覆盖；不得出现组件名、组件层级或局部状态绑定规则。
4. **实现方式**: 对复杂富文本区域，优先抽取可复用的 Tailwind 类常量或共享 utility，而不是在多个元素上重复长串类名，也不是把深层选择器迁回全局 CSS。
5. **迁移策略**: 每个组件迁移时同时删除对应 `.module.css` 和导入；最终提交不保留任何桥接文件或“待清理”残留，并以 FR-009 组件清单和 FR-010 状态矩阵作为验收边界。

### Migration Order

1. 建立主题令牌与全局能力边界：整理 `index.css`，补齐 `@theme` 和共享 utility。
2. 迁移壳层与弹窗：`ConfirmDialog`、`NewProjectModal`、`SettingsModal`。
3. 迁移 Sidebar 体系：`Sidebar`、`ProjectItem`、`SessionItem`。
4. 迁移 Chat 壳层与输入区：`Chat`、`TopBar`、`InputBar`、`CompactMessage`、`PromptMetricsMessage`。
5. 迁移消息与渲染区：`AssistantMessage`、`UserMessage`、`MessageList`、`ToolCallBlock`、`ToolJsonView`、`SubRunBlock`。
6. 清理残留：删除 `vite-env.d.ts` 中未使用的 CSS Module 声明，运行搜索、测试和 CSS 体积核对。

### Validation Strategy

- 源码验证：`rg --files frontend/src -g '*.module.css'` 与 `rg -n '\.module\.css|styles\.' frontend/src -g '*.ts' -g '*.tsx'`
- 仓库级 Rust 校验（宪法要求）：`cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
- 前端类型/构建/测试：`cd frontend && npm run typecheck && npm run build && npm run lint && npm test`
- 视觉验证：按 FR-009 列出的全部组件逐项对比迁移前截图或等效基线
- 状态验证：按组件-状态矩阵逐项检查 hover、active、focus-visible、disabled、selected、loading、error、streaming、展开/收起和上下文菜单状态
- 结构验证：审查 `index.css` 是否只剩全局能力，审查 `cn(...)` 和 `style` 的使用是否符合合同
- 令牌映射验证：审查 `sourceVar -> tailwindAlias -> usageScope -> consumer` 清单，确认共享变量均有落点
- CSS 体积验证：对比迁移前后生产构建的主 CSS 产物（`dist/assets/index-*.css`）大小，增长不超过 10%

## Complexity Tracking

无已知宪法违规项，无需额外复杂度豁免。
