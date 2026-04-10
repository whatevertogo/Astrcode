# Tasks: 前端统一样式技术栈迁移

**Input**: Design documents from `D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/`
**Prerequisites**: [plan.md](D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/plan.md), [spec.md](D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/spec.md), [research.md](D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/research.md), [data-model.md](D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/data-model.md), [contracts/styling-boundary-contract.md](D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/contracts/styling-boundary-contract.md), [quickstart.md](D:/GitObjectsOwn/Astrcode/specs/007-tailwind-migration/quickstart.md)

**Tests**: 本特性未要求先写新的自动化测试；每个用户故事都必须完成源码搜索、前端构建验证和关键界面人工对比。

**Organization**: Tasks are grouped by user story to enable independent implementation and validation.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: 固化迁移基线、盘点影响范围、准备最终验收记录位置

- [x] T001 Capture pre-migration screenshot targets, the full FR-009 component verification matrix, and CSS bundle baseline notes in `specs/007-tailwind-migration/quickstart.md`
- [x] T002 [P] Record the 17-file migration inventory and the design-token mapping inventory in `specs/007-tailwind-migration/data-model.md`
- [x] T003 [P] Record the final source-audit, component-state matrix, and validation checklist in `specs/007-tailwind-migration/contracts/styling-boundary-contract.md`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: 建立所有用户故事共用的样式边界和共享能力

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [x] T004 Establish Tailwind v4 theme aliases, shared keyframes, and allowed global utility boundaries in `frontend/src/index.css`
- [x] T005 Create reusable Tailwind class-composition helpers for shared surfaces, badges, and prose blocks in `frontend/src/lib/styles.ts`

**Checkpoint**: Foundation ready - component migration can now begin in parallel

---

## Phase 3: User Story 1 - 样式技术栈统一且可审计 (Priority: P1) 🎯 MVP

**Goal**: 删除全部 CSS Module，实现现有关键界面的 Tailwind 化，并保持视觉与交互一致

**Independent Test**: `rg --files frontend/src -g '*.module.css'` 返回 0 条结果；`rg -n '\.module\.css|styles\.' frontend/src -g '*.ts' -g '*.tsx'` 返回 0 条结果；FR-009 列出的全部组件均完成迁移前后视觉对比且无肉眼可见回归

### Implementation for User Story 1

- [x] T006 [P] [US1] Migrate dialog components to Tailwind in `frontend/src/components/ConfirmDialog.tsx`, `frontend/src/components/NewProjectModal.tsx`, and `frontend/src/components/Settings/SettingsModal.tsx`
- [x] T007 [US1] Delete dialog CSS Modules in `frontend/src/components/ConfirmDialog.module.css`, `frontend/src/components/NewProjectModal.module.css`, and `frontend/src/components/Settings/SettingsModal.module.css`
- [x] T008 [P] [US1] Migrate sidebar components to Tailwind in `frontend/src/components/Sidebar/index.tsx`, `frontend/src/components/Sidebar/ProjectItem.tsx`, and `frontend/src/components/Sidebar/SessionItem.tsx`
- [x] T009 [US1] Delete sidebar CSS Modules in `frontend/src/components/Sidebar/Sidebar.module.css`, `frontend/src/components/Sidebar/ProjectItem.module.css`, and `frontend/src/components/Sidebar/SessionItem.module.css`
- [x] T010 [P] [US1] Migrate chat shell and input components to Tailwind in `frontend/src/components/Chat/index.tsx`, `frontend/src/components/Chat/TopBar.tsx`, `frontend/src/components/Chat/InputBar.tsx`, `frontend/src/components/Chat/CompactMessage.tsx`, and `frontend/src/components/Chat/PromptMetricsMessage.tsx`
- [x] T011 [US1] Delete chat shell/input CSS Modules in `frontend/src/components/Chat/Chat.module.css`, `frontend/src/components/Chat/TopBar.module.css`, `frontend/src/components/Chat/InputBar.module.css`, `frontend/src/components/Chat/CompactMessage.module.css`, and `frontend/src/components/Chat/PromptMetricsMessage.module.css`
- [x] T012 [P] [US1] Migrate chat render and tool display components to Tailwind in `frontend/src/components/Chat/AssistantMessage.tsx`, `frontend/src/components/Chat/UserMessage.tsx`, `frontend/src/components/Chat/MessageList.tsx`, `frontend/src/components/Chat/ToolCallBlock.tsx`, `frontend/src/components/Chat/ToolJsonView.tsx`, and `frontend/src/components/Chat/SubRunBlock.tsx`
- [x] T013 [US1] Delete chat render/display CSS Modules in `frontend/src/components/Chat/AssistantMessage.module.css`, `frontend/src/components/Chat/UserMessage.module.css`, `frontend/src/components/Chat/MessageList.module.css`, `frontend/src/components/Chat/ToolCallBlock.module.css`, `frontend/src/components/Chat/ToolJsonView.module.css`, and `frontend/src/components/Chat/SubRunBlock.module.css`
- [x] T014 [US1] Remove the CSS Module type declaration from `frontend/src/vite-env.d.ts`
- [ ] T015 [US1] Record the Tailwind-only source audit and full component-by-component visual verification results for all FR-009 entries in `specs/007-tailwind-migration/quickstart.md`

**Checkpoint**: At this point, User Story 1 should be fully functional and independently reviewable as the MVP

---

## Phase 4: User Story 2 - 新组件开发只需一种样式心智模型 (Priority: P2)

**Goal**: 让新组件开发只需要 Tailwind、主题令牌和 `cn(...)`，不再需要 CSS Module 或额外样式方案

**Independent Test**: 现有 Tailwind 原生组件和应用壳层都改为消费共享令牌与样式 helper，令牌映射清单完整记录 `sourceVar -> tailwindAlias -> usageScope -> consumer`，`quickstart.md` 提供无 CSS 文件的新组件编写路径

### Implementation for User Story 2

- [x] T016 [US2] Normalize semantic token exposure for shared colors, spacing, shadows, and motion in `frontend/src/index.css` and update the token mapping inventory in `specs/007-tailwind-migration/data-model.md`
- [x] T017 [P] [US2] Align shared token usage in the application shell at `frontend/src/App.tsx`
- [x] T018 [P] [US2] Align Tailwind-native reference components with shared token helpers in `frontend/src/components/Chat/CommandSelector.tsx` and `frontend/src/components/Chat/ModelSelector.tsx`
- [ ] T019 [US2] Add Tailwind-only component authoring examples, token lookup guidance, and `sourceVar -> tailwindAlias -> consumer` examples in `specs/007-tailwind-migration/quickstart.md`

**Checkpoint**: At this point, a new component can be authored using only Tailwind classes, shared helpers, and documented token references

---

## Phase 5: User Story 3 - 样式维护边界清晰且修改路径单一 (Priority: P3)

**Goal**: 把重复样式收敛为可维护的 shared helper，并清理全局越界规则，让日常样式修改只落在 TSX 或主题层

**Independent Test**: 修改代表性组件样式时只需要改组件 TSX 或 `frontend/src/index.css` 令牌；`index.css` 中不再出现组件专属规则

### Implementation for User Story 3

- [x] T020 [US3] Consolidate shared rich-text, code-block, and surface helper definitions in `frontend/src/lib/styles.ts`
- [x] T021 [P] [US3] Apply shared style helpers to rich-text and tool display components in `frontend/src/components/Chat/AssistantMessage.tsx`, `frontend/src/components/Chat/ToolCallBlock.tsx`, and `frontend/src/components/Chat/ToolJsonView.tsx`
- [x] T022 [P] [US3] Apply shared style helpers to sub-run and settings surfaces in `frontend/src/components/Chat/SubRunBlock.tsx` and `frontend/src/components/Settings/SettingsModal.tsx`
- [x] T023 [US3] Remove dead global and style-boundary leftovers in `frontend/src/index.css`, `frontend/src/main.tsx`, and `frontend/src/App.tsx`
- [ ] T024 [US3] Document one-file style edits and theme-wide update workflow in `specs/007-tailwind-migration/quickstart.md`

**Checkpoint**: All user stories are now independently functional, and style maintenance follows a single predictable path

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: 记录最终结果并完成整体验证

- [x] T025 [P] Update final migration notes, CSS size delta, and visual review outcomes in `specs/007-tailwind-migration/plan.md` and `specs/007-tailwind-migration/quickstart.md`
- [x] T026 Run final validation. (A) Repository-level Rust validation per constitution: `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`. (B) Frontend validation: `cd frontend && npm run typecheck && npm run build && npm run lint && npm test`. (C) Search for compat layer residue: `rg -n 'bridge\|wrapper\|shim\|compat\|adapter.*css\|module\.css' frontend/src -g '*.ts' -g '*.tsx'` must return 0 results. (D) Manually verify the full component-state matrix for all FR-009 components in `specs/007-tailwind-migration/contracts/styling-boundary-contract.md`. (E) Manually verify desktop scrolling behavior: sticky header/footer, scroll lock on modal overlay, text selection in chat area, and input focus experience. (F) Confirm the token mapping inventory in `specs/007-tailwind-migration/data-model.md` is complete for all shared root variables

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies - can start immediately
- **Phase 2 (Foundational)**: Depends on Phase 1 - blocks all user stories
- **Phase 3 (US1)**: Depends on Phase 2 - delivers the MVP and unblocks all cleanup work
- **Phase 4 (US2)**: Depends on Phase 2 and should be executed after the US1 migration groups stabilize
- **Phase 5 (US3)**: Depends on US1 and the shared token/helper direction established in US2
- **Phase 6 (Polish)**: Depends on all desired user stories being complete

### User Story Dependencies

- **US1**: No dependency on other user stories; this is the MVP
- **US2**: Builds on the foundational style boundary and benefits from US1’s migrated components, but remains independently testable through token/helper usage and authoring guidance
- **US3**: Depends on US1’s completed migration and US2’s shared token/helper conventions to make maintenance boundaries durable

### Within Each User Story

- Shared boundary work before component migration
- Component Tailwind migration before deleting the matching `.module.css`
- Source audit and visual verification before marking a story complete
- Shared helper extraction before component adoption
- Final validation after all cleanup and documentation updates

### Parallel Opportunities

- `T002` and `T003` can run in parallel during setup
- `T006`, `T008`, `T010`, and `T012` can run in parallel after foundational work completes
- `T017` and `T018` can run in parallel after `T016`
- `T021` and `T022` can run in parallel after `T020`
- `T025` can be prepared while `T026` validation is being coordinated

---

## Parallel Example: User Story 1

```bash
Task: "Migrate dialog components to Tailwind in frontend/src/components/ConfirmDialog.tsx, frontend/src/components/NewProjectModal.tsx, and frontend/src/components/Settings/SettingsModal.tsx"
Task: "Migrate sidebar components to Tailwind in frontend/src/components/Sidebar/index.tsx, frontend/src/components/Sidebar/ProjectItem.tsx, and frontend/src/components/Sidebar/SessionItem.tsx"
Task: "Migrate chat shell and input components to Tailwind in frontend/src/components/Chat/index.tsx, frontend/src/components/Chat/TopBar.tsx, frontend/src/components/Chat/InputBar.tsx, frontend/src/components/Chat/CompactMessage.tsx, and frontend/src/components/Chat/PromptMetricsMessage.tsx"
Task: "Migrate chat render and tool display components to Tailwind in frontend/src/components/Chat/AssistantMessage.tsx, frontend/src/components/Chat/UserMessage.tsx, frontend/src/components/Chat/MessageList.tsx, frontend/src/components/Chat/ToolCallBlock.tsx, frontend/src/components/Chat/ToolJsonView.tsx, and frontend/src/components/Chat/SubRunBlock.tsx"
```

---

## Parallel Example: User Story 2

```bash
Task: "Align shared token usage in the application shell at frontend/src/App.tsx"
Task: "Align Tailwind-native reference components with shared token helpers in frontend/src/components/Chat/CommandSelector.tsx and frontend/src/components/Chat/ModelSelector.tsx"
```

---

## Parallel Example: User Story 3

```bash
Task: "Apply shared style helpers to rich-text and tool display components in frontend/src/components/Chat/AssistantMessage.tsx, frontend/src/components/Chat/ToolCallBlock.tsx, and frontend/src/components/Chat/ToolJsonView.tsx"
Task: "Apply shared style helpers to sub-run and settings surfaces in frontend/src/components/Chat/SubRunBlock.tsx and frontend/src/components/Settings/SettingsModal.tsx"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational
3. Complete Phase 3: User Story 1
4. Stop and validate the Tailwind-only audit plus key-surface visual parity
5. Use this as the first mergeable increment

### Incremental Delivery

1. Finish Setup + Foundational to lock the style boundary
2. Deliver US1 as the MVP migration slice
3. Add US2 to make new component authoring and token usage durable
4. Add US3 to reduce maintenance cost and remove boundary regressions
5. Finish with Phase 6 validation and documentation updates

### Parallel Team Strategy

1. One person prepares `frontend/src/index.css` and `frontend/src/lib/styles.ts`
2. After Phase 2, different people can own dialog, sidebar, chat shell, and chat render migration groups in parallel
3. Once US1 is stable, one person can normalize tokens while another updates Tailwind-native reference components
4. Shared helper adoption for US3 can then be split by component group

---

## Notes

- [P] tasks operate on different files after their prerequisites are in place
- Every user story remains independently reviewable through explicit source checks and UI verification
- The suggested MVP scope is **User Story 1 only**
- Final implementation must avoid reintroducing CSS Modules, component-specific global CSS, or static inline-style drift
