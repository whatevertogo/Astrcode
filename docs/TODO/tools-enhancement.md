# 工具增强 TODO

> 对比项目：Claude Code、OpenCode、Codex、pi-mono、kimi-cli

---

## 已有工具能力完善度

### readFile

| 功能点 | Astrcode | OpenCode | Codex | kimi-cli | 状态 |
|--------|:--------:|:--------:|:-----:|:--------:|:----:|
| UTF-8 文本读取 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 图片 base64 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 行号显示 | ✅ | ✅ | - | - | ✅ |
| offset/limit 分页 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 二进制检测 | ✅ | ✅ | ✅ | ✅ | ✅ |
| PDF 支持 | ❌ | ✅ | - | - | 缺失 |

### editFile

| 功能点 | Astrcode | OpenCode | Codex | kimi-cli | 状态 |
|--------|:--------:|:--------:|:-----:|:--------:|:----:|
| 唯一匹配编辑 | ✅ | ✅ | - | ✅ | ✅ |
| replaceAll | ✅ | ✅ | - | ✅ | ✅ |
| 批量编辑 (edits 数组) | ✅ | ✅ | - | ✅ | ✅ |
| diff 生成 | ✅ | ✅ | - | ✅ | ✅ |
| 模糊匹配策略 | ❌ 0层 | ✅ 9层 | - | ❌ | **不足** |
| LSP 诊断反馈 | ❌ | ✅ | - | - | 缺失 |
| 文件并发锁定 | ❌ | ✅ | - | - | 缺失 |

**OpenCode 的 9 层模糊匹配降级策略**:
1. SimpleReplacer - 精确匹配
2. LineTrimmedReplacer - 逐行 trim
3. BlockAnchorReplacer - 首尾锚定 + Levenshtein 相似度
4. WhitespaceNormalizedReplacer - 空白归一化
5. IndentationFlexibleReplacer - 忽略缩进差异
6. EscapeNormalizedReplacer - 转义字符归一化
7. TrimmedBoundaryReplacer - 边界 trim
8. ContextAwareReplacer - 上下文感知匹配 (50% 行相似度)
9. MultiOccurrenceReplacer - 全部精确匹配位置

### shell

| 功能点 | Astrcode | OpenCode | Codex | kimi-cli | 状态 |
|--------|:--------:|:--------:|:-----:|:--------:|:----:|
| 命令执行 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 流式输出 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 超时控制 | ✅ | ✅ | ✅ | ✅ | ✅ |
| UTF-8 碎片处理 | ✅ | - | - | - | **独有** |
| 跨平台 shell 检测 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 沙箱隔离 | ❌ | ❌ | ✅ | ❌ | 不足 |
| 命令 AST 解析 | ❌ | ✅ | - | - | 缺失 |
| stdin 写入 | ❌ | ❌ | ✅ | ❌ | 缺失 |

### grep

| 功能点 | Astrcode | OpenCode | Codex | kimi-cli | 状态 |
|--------|:--------:|:--------:|:-----:|:--------:|:----:|
| 正则匹配 | ✅ | ✅ | ✅ | ✅ | ✅ |
| glob 过滤 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 文件类型过滤 | ✅ | - | - | - | **独有** |
| 上下文行 (B/A) | ✅ | - | - | ✅ | ✅ |
| 3种输出模式 | ✅ | - | - | ✅ | ✅ |
| offset 分页 | ✅ | - | - | - | **独有** |
| .gitignore 感知 | ✅ | ✅ | ✅ | ✅ | ✅ |
| match_text 提取 | ✅ | - | - | - | **独有** |
| 大结果溢出存盘 | ✅ | - | - | - | **独有** |

### findFiles

| 功能点 | Astrcode | OpenCode | Codex | kimi-cli | 状态 |
|--------|:--------:|:--------:|:-----:|:--------:|:----:|
| glob 匹配 | ✅ | ✅ | - | ✅ | ✅ |
| .gitignore 感知 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 隐藏文件控制 | ✅ | - | - | - | ✅ |
| 按修改时间排序 | ✅ | ✅ | - | - | ✅ |

### listDir

| 功能点 | Astrcode | OpenCode | Codex | kimi-cli | 状态 |
|--------|:--------:|:--------:|:-----:|:--------:|:----:|
| name/isDir/isFile | ✅ | - | ✅ | - | ✅ |
| size/modified/extension | ✅ | - | - | ✅ | ✅ |
| 排序支持 (name/modified) | ✅ | - | - | - | ✅ |
| 目录优先 | ✅ | - | - | - | ✅ |

---

## 已有工具总结

### ✅ 已完善（与竞品持平或领先）
- **readFile**: 完整（缺 PDF）
- **grep**: 完整且有独有优势（match_text、offset 分页、溢出存盘）
- **findFiles**: 完整
- **listDir**: 完整

### ⚠️ 有差距
- **editFile**: 缺模糊匹配策略（0层 vs OpenCode 9层）
- **shell**: 缺沙箱隔离（Codex 有完整实现）

---

## P0 - 核心缺失工具

### webFetch
- **实现方**: OpenCode, kimi-cli
- **功能**: URL 抓取，HTML→Markdown 转换，图片附件
- **OpenCode 特性**:
  - 支持 text/markdown/html 三种输出格式
  - 5MB 上限
  - Cloudflare 重试机制
- **kimi-cli 特性**:
  - 服务端 + 本地双层降级（trafilatura）

### task (子代理)
- **实现方**: OpenCode, Codex, kimi-cli
- **功能**: 创建独立会话的子代理执行子任务
- **OpenCode 特性**:
  - 可恢复 (task_id)
  - 权限继承/限制
- **Codex 特性**:
  - agent_jobs (CSV 批量)
  - multi_agents (spawn/send_input/wait/close_agent)

### LSP 工具
- **实现方**: OpenCode
- **功能**: 9 种 LSP 操作
  1. goToDefinition
  2. findReferences
  3. hover
  4. documentSymbol
  5. workspaceSymbol
  6. goToImplementation
  7. prepareCallHierarchy
  8. incomingCalls
  9. outgoingCalls
- **联动**: 与 edit 联动提供诊断反馈

---

## P1 - 重要增强

### webSearch
- **实现方**: OpenCode, kimi-cli
- **OpenCode**: Exa API (MCP 协议), fast/auto/deep 搜索类型
- **kimi-cli**: Moonshot 搜索服务

### apply_patch
- **实现方**: OpenCode, Codex
- **功能**: 统一 patch 格式，支持多文件 add/update/delete/move

### todo 管理
- **实现方**: OpenCode (todowrite + todoread), kimi-cli (SetTodoList)
- **功能**: 任务进度追踪

---

## P2 - 次要增强

### batch 并行执行
- **实现方**: OpenCode
- **功能**: 一次调用并行执行最多 25 个工具

### PDF/Jupyter 支持
- **实现方**: OpenCode
- **PDF**: mime=application/pdf，按页读取
- **Jupyter**: .ipynb 解析

### 文件并发锁定
- **实现方**: OpenCode
- **功能**: FileTime.withLock 防止多工具并发编辑冲突

---

## P3 - 可选增强

### shell 增强
- stdin 写入 (交互式命令)
- 后台运行模式
- 命令 AST 解析 (tree-sitter-bash)

### grep 增强
- multiline 多行匹配
- invert-match 反向匹配

### multiedit 多文件编辑
- **实现方**: OpenCode
- **功能**: 不同文件的多个编辑（当前 edits 数组仅限单文件）

---

## Astrcode 独有优势（保持）

1. **Plugin 系统**: 完整的插件生命周期管理
2. **Skill 工具**: 按需加载，节省 Token
3. **grep match_text 提取**: 精确提取匹配子串
4. **grep offset 分页**: 结果分页迭代
5. **工具结果溢出存盘**: 32KB 阈值自动存盘
6. **完整的 cancel 支持**: 所有关键节点检查
7. **editFile 重叠匹配检测**: 检测 "ababa" 中 "aba" 场景
8. **shell UTF-8 碎片处理**: 跨 read 边界正确拼接
