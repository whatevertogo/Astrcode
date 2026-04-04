# Astrcode 工具系统 优化补充 TODO

> 基于「五大编程 AI 工具系统深度对比」文档与实际代码审查，补充 tools-enhancement.md 中未覆盖的缺失项

---

## 一、工具能力差距清单

以下差距来源于 5 AI 对比文档 + 实际代码审查，按优先级排列。

### Shell 工具

| 功能点 | Astrcode | Claude Code | Codex | Kimi CLI | 状态 |
|--------|:--------:|:-----------:|:-----:|:--------:|:----:|
| 一次性命令执行 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 流式输出 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 超时控制 | ✅ (120s 默认/600s 上限，无严格下限) | ✅ | ✅ (默认 10s) | ✅ (1-300s 严格) | ⚠️ 无超时下限校验 |
| PTY 后台进程 | ❌ | ✅ (持久 PTY 会话) | ✅ (PTY + write_stdin) | ❌ 每次新建 shell | **缺失**（仅 Claude/Codex 有） |
| stdin 写入 | ❌ | — | ✅ (write_stdin) | ❌ | 缺失 |
| 沙箱隔离 | ❌ | ❌ | ✅ | ❌ | 不足 |
| 环境状态持久 | ❌ | ✅ (通过 PTY) | ✅ (通过 PTY) | ❌ | 缺失（cd/export 不跨调用） |

**实际代码分析** (`shell.rs`):
- 当前 shell 是一次性非交互模式，每次调用产生新子进程
- `cd`、`export` 等环境变更**不会在调用间传递**
- 缺少 PTY 支持，无法处理交互式命令（`npm run dev`、`git rebase -i`）

---

### Edit 工具

| 功能点 | Astrcode | OpenCode | Claude Code | Kimi CLI | 状态 |
|--------|:--------:|:--------:|:-----------:|:--------:|:----:|
| 唯一匹配编辑 | ✅ | ✅ | ✅ | ✅ | ✅ |
| replaceAll | ✅ | ✅ | ✅ | ✅ | ✅ |
| 批量 edits 数组 | ✅ | ✅ (multiEdit) | ❌ 单次 | ✅ | ✅ |
| 重叠匹配检测 | ✅ | — | — | — | **独有** |
| LSP 诊断反馈 | ❌ | ✅ | — | — | 缺失 |
| 文件并发锁定 | ❌ | ✅ | — | — | 缺失 |

---

### Grep 工具

| 功能点 | Astrcode | Claude Code | Kimi CLI | Pi-mono | OpenCode | 状态 |
|--------|:--------:|:-----------:|:--------:|:-------:|:--------:|:----:|
| 正则匹配 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| glob 过滤 (`--glob`) | ✅ | ✅ | ✅ | — | ✅ (include) | ✅ |
| 文件类型过滤 (`--type`) | ✅ | ✅ | ✅ | ❌ | ❌ | **独有** |
| 上下文行 (-B/-A/-C) | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ |
| 3 种输出模式 | ✅ | ✅ | ✅ | ❌ | ❌ | ✅ |
| offset 分页 | ✅ | ❌ | ❌ | ❌ | ❌ | **独有** |
| match_text 提取 | ✅ | ❌ | ❌ | ❌ | ❌ | **独有** |
| 大结果溢出存盘 | ✅ | ❌ | ❌ | ❌ | ❌ | **独有** |
| multiline 多行匹配 | ❌ | ❌ | ✅ (`-U --multiline-dotall`) | ❌ | ❌ | 建议补充 |
| 反向匹配 (invert-match) | ❌ | ❌ | — | — | — | 建议补充 |
| literal 模式（固定字符串） | ❌ | — | — | ✅ | — | 建议补充 |
| 行宽截断 | ✅ (500 chars) | ✅ (500) | ✅ (2000) | ✅ (500) | ✅ (2000) | ✅ |

**注意**: grep 功能已相当完善，仅 multiline 和 literal 模式值得补充。

---

### Read 工具

| 功能点 | Astrcode | Claude Code | Kimi CLI | OpenCode | Pi-mono | 状态 |
|--------|:--------:|:-----------:|:--------:|:--------:|:-------:|:----:|
| UTF-8 文本读取 | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ |
| 行号显示 | ✅ | ✅ (cat -n, 6 位列) | ✅ (cat -n, 6 位列) | ✅ | ❌ | ✅ |
| 二进制检测 | ✅ (NUL 字节) | ✅ (UTF-8 解码) | ✅ (后缀/MIME + 512 字节魔数 + NUL) | ✅ (扩展名黑名单 + 4KB 采样) | ❌ | ⚠️ 检测层次不够 |
| 图片 base64 | ✅ | ✅ | ✅ (独立 ReadMediaFile 工具) | ✅ | ✅ | ✅ |
| PDF 支持 | ❌ | ❌ | ✅ | ✅ | ❌ | 缺失 |
| 超长行截断 | ✅ | ✅ | — | — | — | ✅ |

---

### Find/目录操作

| 功能点 | Astrcode | Claude Code | OpenCode | Pi-mono | Kimi CLI | 状态 |
|--------|:--------:|:-----------:|:--------:|:-------:|:--------:|:----:|
| glob 匹配 | ✅ | — (rg 内置) | — (rg 内置) | ✅ (fd) | ✅ | ✅ |
| .gitignore 感知 | ✅ | ✅ (rg 内置) | ✅ (rg 内置) | ✅ (fd 默认) | ❌ (Glob 不读取) | ✅ |
| 隐藏文件控制 | ✅ | — | — | — | 部分 | ✅ |
| 按修改时间排序 | ✅ | — | — | — | — | ✅ |
| listDir 元数据 | ✅ (name/type/size/mod/ext) | ✅ (name/type) | ❌ (目录树文本) | ✅ (name only) | ❌ (无独立工具) | ✅ |

**注意**: findFiles 和 listDir 功能已完善，无明显差距。

---

## 二、架构层面缺失

### 1. 权限/审批系统 ❌ — P1

**现状**: 仅有 `CapabilityEngine` 的 Allow/Deny/Ask 三阶段策略检查，无用户级审批交互机制。

**竞品实现**:
```python
# Kimi CLI 的 Approval 类（简化版）
async def request(self, sender, action, description):
    if self._state.yolo: return True
    if action in self._state.auto_approve_actions: return True
    # 等待用户响应
```

```typescript
// Claude Code 的 filterToolsForAgent
filterToolsForAgent({ tools, isBuiltIn, isAsync, capabilities, permissionOverrides })
```

**差距**:
- 无 YOLO 模式（全自动运行无需确认）
- 无会话级自动批准列表
- 权限粒度不够（无 per-tool 权限覆盖）

---

### 2. 条件工具加载/特性开关 ❌ — P2

**现状**: 内置工具在 `builtin_capabilities.rs` 中硬编码注册，不支持运行时动态启用/禁用。

**竞品实现**:
- **Claude Code**: 基于 `USER_TYPE`、特性开关、循环依赖动态加载/过滤工具
- **Codex**: `ToolsConfig` 结构体，条件编译控制工具可用性
- **OpenCode**: `Tool.define(id, init)` 工厂模式 + `{tools}/*.{js,ts}` 热加载自定义工具
- **Kimi CLI**: 动态导入 `"kimi_cli.tools.file:ReadFile"` 格式字符串

**差距**:
- 无构建时/运行时特性开关
- 不支持 MCP 远程工具动态加载
- 无自定义工具热加载机制

---

### 3. Shell 环境状态持久 ❌ — P1

**现状**: 每次 shell 调用产生全新子进程，`cd`、`export` 等环境变更不跨调用传递。

**影响**: AI Agent 执行以下工作流时受阻:
```
shell: cd my-project  →  下一调用: cargo build  # 不在 my-project 中
shell: export API_KEY=xxx  →  下一调用: curl $API_KEY  # 变量未定义
```

**竞品实现**:
| 工具 | 状态持久机制 |
|------|------------|
| Claude Code | 持久 PTY 会话 |
| Codex | PTY 会话 + write_stdin |
| Kimi CLI | ❌ 同样每次新建 |
| Astrcode | ❌ 同样每次新建 |

**建议方案**:
- **方案 A**: 引入 PTY 支持（`portable-pty` crate），维护长期会话
- **方案 B**: Shell wrapper 模式（在单 shell 实例中执行命令序列）
- **方案 C**: 环境状态管理（手动传递 env/cwd 到后续调用）

---

### 4. 工具结果压缩/清理策略 ❌ — P2

**现状**: `ToolCapabilityMetadata` 有 `compact_clearable` 标志，但缺少:
- 工具结果的分级压缩策略
- 对话历史溢出时的工具输出清理规则

**竞品实现**:
- **Kimi CLI**: `ToolResultBuilder` 50KB 默认输出限制
- **OpenCode**: 工具输出带字符级截断，可配置

---

## 三、建议的新增工具

### webFetch — P0

| 对比项 | OpenCode | Kimi CLI | Astrcode |
|--------|:--------:|:--------:|:--------:|
| URL 抓取 | ✅ | ✅ | ❌ |
| HTML → Markdown 转换 | ✅ | ✅ (trafilatura) | ❌ |
| 输出格式 | text/markdown/html | — | — |
| 5MB 上限 | ✅ | — | — |
| Cloudflare 重试 | ✅ | — | — |
| 服务端降级 | ❌ | ✅ | — |

### task (子代理) — P0

| 对比项 | OpenCode | Codex | Kimi CLI | Astrcode |
|--------|:--------:|:-----:|:--------:|:--------:|
| 可恢复 (task_id) | ✅ | — | — | ❌ |
| 权限继承/限制 | ✅ | — | — | ❌ |
| 批量 Agent Jobs | — | ✅ (CSV) | — | — |
| spawn/send_input/wait | — | ✅ | — | — |

### LSP 工具集 — P0

OpenCode 的 9 种 LSP 操作:
1. `goToDefinition`
2. `findReferences`
3. `hover`
4. `documentSymbol`
5. `workspaceSymbol`
6. `goToImplementation`
7. `prepareCallHierarchy`
8. `incomingCalls`
9. `outgoingCalls`

**联动价值**: 与 edit 联动提供诊断反馈（当前 editFile 缺少 LSP 诊断）

### apply_patch — P1

| 对比项 | OpenCode | Codex | Astrcode |
|--------|:--------:|:-----:|:--------:|
| 统一 patch 格式 | ✅ | ✅ | ❌ |
| 多文件操作 | ✅ | ✅ | — |
| add/update/delete/move | ✅ | ✅ | — |

### todo 管理 — P1

| 对比项 | OpenCode | Kimi CLI | Astrcode |
|--------|:--------:|:--------:|:--------:|
| 任务 CRUD | ✅ (todowrite + todoread) | ✅ (SetTodoList) | ❌ |
| Frontend 集成 | ✅ | ✅ | ✅ (已有前端组件) |

---

## 四、次要增强

### Shell 增强 — P3

- **后台运行模式**: 支持 `isBackground` 参数，命令在后台执行，返回进程 ID
- **stdin 写入**: 向交互式命令传递输入（如 `git rebase -i` 的编辑器输入）
- **命令 AST 解析**: 使用 `tree-sitter-bash` 解析命令结构，用于安全审计

### grep 增强 — P3

- **multiline 多行匹配**: 支持 `-U --multiline-dotall` 模式，`.` 匹配换行符
- **invert-match 反向匹配**: `-v` 标志，返回不匹配的行
- **literal 模式**: 固定字符串匹配，不解析正则表达式

### multiedit 多文件编辑 — P3

- 当前 edits 数组仅限单文件
- 支持跨文件的多个独立编辑（OpenCode 有此功能）

### 工具并发执行 — P2

- **batch 并行**: 一次调用并行执行最多 25 个工具（OpenCode experimental batch）
- **文件并发锁定**: `FileTime.withLock` 防止多工具并发编辑冲突

---

## 五、Astrcode 独有优势（继续保持）

1. **Plugin 系统**: 完整的插件生命周期管理
2. **Skill 工具**: 按需加载，节省 Token
3. **grep match_text 提取**: 精确提取匹配子串
4. **grep offset 分页**: 结果分页迭代
5. **工具结果溢出存盘**: 32KB 阈值自动存盘
6. **完整的 cancel 支持**: 所有关键节点检查
7. **editFile 重叠匹配检测**: 检测 "ababa" 中 "aba" 场景
8. **shell UTF-8 碎片处理**: 跨 read 边界正确拼接
9. **Rust 编译期安全**: 所有工具通过 trait 边界保证类型安全
10. **统一能力路由**: `CapabilityRouter` 统一调度和工具分发

---

## 六、已有 TODO 跟踪

| 位置 | 优先级 | 描述 |
|------|--------|------|
| `read_file.rs#L40` | 中 | SVG 是否应走多模态图片路径还是纯文本路径（目前走多模态） |
| `read_file.rs#L318` | 低 | 用懒加载/流式宽度策略替换第二次完整文件扫描 |
| `grep.rs#L628` | 中 | `session_dir()` 尚未注入 ToolContext，暂时用 `force_inline` 跳过存盘 |

---

## 七、实施路线图建议

### Phase 1 (P0) — 核心缺失

| 项目 | 预估工作量 | 依赖 |
|------|-----------|------|
| 审批系统/权限升级 | 2-3 周 | CapabilityEngine 改造 |
| shell 环境状态持久 | 2-3 周 | PTY/环境变量管理 |
| LSP 工具集 | 3-4 周 | LSP 通信层 |

### Phase 2 (P1) — 重要增强

| 项目 | 预估工作量 | 依赖 |
|------|-----------|------|
| 特性开关/条件加载 | 1-2 周 | 工具注册系统 |
| 编辑文件锁 | 1 周 | 文件系统工具 |

### Phase 3 (P2/P3) — 次要增强

| 项目 | 预估工作量 | 依赖 |
|------|-----------|------|
| grep multiline/literal | 0.5 周 | grep 核心 |
| shell 后台+stdin | 1-2 周 | PTY |
| multiedit 多文件 | 0.5 周 | editFile |

### 不实施项

- **模糊匹配策略** → 明确不要（安全风险 > 收益）
- **沙箱隔离** → 优先级低（Codex 有完整实现，但其他项目也多无）
