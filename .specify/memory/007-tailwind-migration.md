---
name: 前端 Tailwind v4 样式架构
description: 007 分支 — CSS Module 全面迁移至 Tailwind v4 utility class，@theme 令牌桥接 :root 变量、styles.ts 共享常量、ChatScreenContext 依赖注入
type: project
---

## 状态 (2026-04-11)
**007-tailwind-migration** 分支，已合并入主干。

**Why:** CSS Module 导致样式与组件强耦合、无法跨组件复用、调试时类名混淆。统一到 Tailwind v4 utility class 后样式直接可见于 TSX，共享常量通过 `styles.ts` 集中管理。

**How to apply:** 所有前端样式必须遵循以下三层架构，新增组件优先使用 `styles.ts` 常量，新增语义色必须在 `@theme` 注册令牌。

## 样式三层架构

1. **`:root` + `@theme`** (`index.css`) — 颜色、阴影、动画、布局变量的唯一事实源
   - `:root` 定义原始 CSS 变量值（含暗色/终端/JSON 语法色等）
   - `@theme` 将 `:root` 变量桥接为 Tailwind 可消费的令牌（`--color-text-primary` → `text-text-primary`）
   - 聊天布局变量（`--chat-content-max-width` 等）也在 `:root` 中定义，不放在 JSX inline style
2. **`styles.ts`** — 跨组件复用的 Tailwind class 组合常量
   - 弹窗/面板：`overlay`, `dialogSurface`, `fieldInput`
   - 按钮：`btnPrimary`, `btnSecondary`, `btnDanger`, `ghostIconButton`
   - 徽章：`pillBase`, `pillSuccess`, `pillWarning`, `pillDanger`, `pillInfo`
   - 卡片/代码/Composer：`errorSurface`, `codeBlockShell`, `composerShell` 等
3. **组件内联** — 仅组件特有的一次性样式直接写在 `className`

## 硬约束

- **禁止硬编码 hex 色**：所有语义色必须注册 `@theme` 令牌（如 `text-json-string` 而非 `text-[#1f6b45]`）
- **条件类名用 `cn()`**：来自 `lib/utils.ts`（clsx + tailwind-merge），禁止 `style={{ backgroundColor: ... }}`
- **共享工具函数放 `lib/utils.ts`**：如 `calculateCacheHitRatePercent`，禁止跨文件重复定义
- **CSS 变量集中管理**：所有 CSS 变量定义在 `index.css` 的 `:root` 中，不在 JSX inline style

## 新增的 Hook / Store 拆分

- `ChatScreenContext.tsx` — 聊天屏幕上下文接口 + Provider，替代 prop drilling
- `useComposerActions` — 提交/中断/删除等用户操作
- `useSessionCoordinator` — 会话加载/激活/刷新
- `useSubRunNavigation` — 子执行导航（打开/关闭/路径跳转）
- `reducerHelpers.ts` — 纯函数：消息查找、移动、upsert
- `reducerMessageProjection.ts` — 消息投影：delta 合并、tool call 更新、metrics upsert

## 关键文件

- `frontend/src/index.css` — `:root` 变量 + `@theme` 令牌桥接
- `frontend/src/lib/styles.ts` — 共享 Tailwind class 常量
- `frontend/src/lib/utils.ts` — `cn()` + 共享计算函数
- `frontend/src/components/Chat/ChatScreenContext.tsx` — 上下文定义
