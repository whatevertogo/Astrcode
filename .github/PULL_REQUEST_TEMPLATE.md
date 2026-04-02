## 一、PR 概述
<!-- 简要描述此 PR 的变更内容 -->

---

## 二、背景 / 动机
- **问题现象**：
- **影响范围**：
- **复现方式**：

---

## 三、改动内容
<!-- 列出主要的变更点 -->

-

**改动类型：**
- [ ] 新功能
- [ ] 缺陷修复
- [ ] 重构/整理
- [ ] 性能优化
- [ ] 文档更新
- [ ] 测试补充
- [ ] 依赖/配置变更

---

## 四、实现说明
- **核心思路**：
- **关键设计/权衡**：
- **主要涉及文件/模块**：

---

## 五、行为变化（对外影响）
- **之前是**：
- **现在是**：
- **兼容性影响**：

---

## 六、测试与验证
**自测结果：**
- [ ] 已本地验证通过
- [ ] 已跑单测
- [ ] 已跑集成测试/端到端测试

**测试命令：**
```bash
# Rust 全量检查
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode

# 前端完整检查
cd frontend && npm run typecheck && npm run lint && npm run format:check
```

---

## 七、Checklist
- [ ] 代码已通过 `cargo fmt --all -- --check`
- [ ] 代码已通过 `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] 测试已通过 `cargo test --workspace --exclude astrcode`
- [ ] 前端类型检查已通过 `npm run typecheck`
- [ ] 前端 lint 已通过 `npm run lint`
- [ ] 前端格式检查已通过 `npm run format:check`
- [ ] 已添加必要的注释
- [ ] 已更新相关文档
- [ ] 无 Breaking Changes（或已明确说明）
- [ ] 已关联相关 Issue

---

## 八、关联 Issue
<!-- 例如: Closes #123 -->
