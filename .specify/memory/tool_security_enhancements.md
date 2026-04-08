---
name: tool_security_enhancements
description: builtin_tools 安全加固和 Agent 友好性改进（从 Claude Code 学习）
type: reference
---

## 文件操作安全检查

### 设备文件黑名单
**Location**: `runtime-tool-loader/src/builtin_tools/read_file.rs`

**规则**：禁止读取设备文件（防止无限读取或阻塞）

```rust
const DEVICE_FILE_BLACKLIST: &[&str] = &[
    "/dev/zero", "/dev/random", "/dev/urandom",
    "/dev/stdin", "/dev/stdout", "/dev/stderr",
    "/dev/null", "/dev/full",
];
```

### UNC 路径检查
**Location**: `runtime-tool-loader/src/builtin_tools/fs_common.rs`

**规则**：禁止写入 UNC 路径（防止 Windows NTLM 泄漏）

```rust
fn is_unc_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        path.to_string_lossy().starts_with(r"\\")
    }
    #[cfg(not(windows))]
    {
        false
    }
}
```

**Why**：访问 `\\attacker.com\share\file` 会触发 NTLM 认证，泄漏凭据

### 文件大小限制
**Location**: `runtime-tool-loader/src/builtin_tools/edit_file.rs`

**规则**：编辑文件不超过 1 GiB（防止 OOM）

```rust
const MAX_EDIT_FILE_SIZE: u64 = 1024 * 1024 * 1024; // 1 GiB
```

### 符号链接检测
**Location**: `runtime-tool-loader/src/builtin_tools/fs_common.rs`

**规则**：禁止写入符号链接（防止绕过路径沙箱）

```rust
fn is_symlink(path: &Path) -> Result<bool> {
    Ok(path.symlink_metadata()?.is_symlink())
}
```

**Why**：符号链接可能指向工作目录外的文件

## 用户体验改进

### 引号规范化
**Location**: `runtime-tool-loader/src/builtin_tools/edit_file.rs`

**规则**：自动转换智能引号为标准 ASCII 引号

```rust
fn normalize_quotes(s: &str) -> String {
    s.replace('\u{201C}', "\"")  // " → "
        .replace('\u{201D}', "\"")  // " → "
        .replace('\u{2018}', "'")   // ' → '
        .replace('\u{2019}', "'")   // ' → '
}
```

**Why**：LLM 生成的代码常包含智能引号，导致匹配失败

### Grep 默认限制
**Location**: `runtime-tool-loader/src/builtin_tools/grep.rs`

**规则**：默认返回 250 条结果（防止 context window 爆炸）

```rust
const DEFAULT_MAX_MATCHES: usize = 250;
```

**Why**：与 Claude Code 保持一致，避免大量结果占满上下文

### Shell 超时上限
**Location**: `runtime-tool-loader/src/builtin_tools/shell.rs`

**规则**：超时上限 600 秒（10 分钟），默认 120 秒

```rust
let timeout_secs = args.timeout.unwrap_or(120).min(600);
```

**Why**：平衡资源限制和长时间任务需求

## Agent 友好性

### ListDir 按大小排序
**Location**: `runtime-tool-loader/src/builtin_tools/list_dir.rs`

**规则**：支持 `sortBy: "size"`（降序，最大文件优先）

```rust
SortBy::Size => {
    entries.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => Ordering::Less,  // 目录优先
            (false, true) => Ordering::Greater,
            _ => b.size.cmp(&a.size),  // 降序
        }
    });
}
```

**Why**：帮助 Agent 快速定位大文件（日志、数据库等）

## 从 Claude Code 学习的其他模式

### 1. Bash Zsh 危险命令检测
**未实现**：检测 `zmodload`、`emulate`、`zpty` 等可能改变 shell 行为的命令

### 2. FileRead 图片大小限制
**已有**：20 MB 限制（与 Claude Code 一致）

### 3. macOS 截图路径兼容性
**未实现**：处理窄空格（U+202F）在截图文件名中的使用

### 4. Dry-run 模式
**未实现**：所有工具缺少 dry-run 模式（预览操作但不执行）

## 测试验证

```bash
# 运行所有工具测试
cargo test -p astrcode-runtime-tool-loader --lib

# 预期：92 个测试全部通过
```

## 安全检查清单

- [x] 设备文件黑名单（read_file.rs）
- [x] UNC 路径检查（edit_file.rs, write_file.rs, apply_patch.rs）
- [x] 文件大小限制（edit_file.rs）
- [x] 符号链接检测（所有写入工具）
- [x] 引号规范化（edit_file.rs）
- [x] Grep 默认限制（grep.rs）
- [x] ListDir 按大小排序（list_dir.rs）
- [ ] Zsh 危险命令检测（未实现）
- [ ] Dry-run 模式（未实现）
