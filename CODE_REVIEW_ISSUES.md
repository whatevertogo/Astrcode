# Code Review — master (5e27af9)

## Summary
Files reviewed: 10 | New issues: 4 (0 critical, 1 high, 3 low) | Perspectives: 4/4

All changes are documentation-only (markdown). No code, no tests affected.

---

## 🔒 Security

*No security issues found. (Documentation-only change.)*

---

## 📝 Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| High | ADR-0007 status 降级为 `Proposed` 但实现代码仍存在 | `docs/adr/0007-layered-prompt-builder-for-kv-cache-optimization.md:3` | 读者会误以为该 ADR 仅有方案而未实现，实际 `crates/runtime-prompt/src/layered_builder.rs` 已存在 |

---

## ✅ Tests

*No tests affected. (Documentation-only change.)*

---

## 🏗️ Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| Low | Section 3.3 crate 列表遗漏 `runtime-agent-loader`、`runtime-registry`、`runtime-skill-loader`、`runtime-tool-loader` | `docs/architecture/architecture.md:62-71` |
| Low | Section 8 "推荐阅读顺序" 第 1 项是自身 `./architecture.md`，自引用无实际导航价值 | `docs/architecture/architecture.md:242` |
| Low | CLAUDE.md 依赖图仍列出 `runtime-tool-loader`，但 architecture.md 3.3 不再提及 | `CLAUDE.md` vs `docs/architecture/architecture.md` |

---

## 🚨 Must Fix Before Merge

1. **[ARCH-001]** ADR-0007 status 从"已实现但未投产"改为 `Proposed`，但 `layered_builder.rs` 仍存在
   - Impact: 状态描述不反映实际情况，可能误导后续决策
   - Fix: 将 status 改为 `Accepted` 或 `Implemented (not in production)`，或在 Consequences 中明确标注"实现已存在但未接入生产路径"

---

## 📎 Pre-Existing Issues (not blocking)

- `agent-loop-roadmap.md` 在 README.md 推荐阅读中列出，但 architecture.md 推荐阅读顺序中不再出现，可能是有意精简

---

## 🤔 Low-Confidence Observations

- architecture.md 3.3 把多个 crate 合并为一行 (`runtime-prompt / runtime-llm / runtime-config`)，这减少了篇幅但降低了导航性。如果目的是让 architecture.md 只讲主线而非列举全部 crate，这可以接受。
- ADR 0003/0004/0005/0006 移除了 `Amended` 日期和 `Current Implementation Status` 章节。作为"精简 ADR 到决策记录"的重构，这是合理的，但后续读者无法从 ADR 直接找到实现文件路径。
