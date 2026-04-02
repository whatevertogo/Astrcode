//! # Astrcode 内置工具
//!
//! 本库实现 Astrcode 编码代理（agent）的内置工具集，所有工具均实现 `astrcode_core::Tool` trait。
//!
//! ## 架构约束
//!
//! - 本 crate 仅依赖 `astrcode_core`，不依赖 `runtime` 或其他业务 crate
//! - 所有工具通过 `Tool` trait 统一接口暴露，由 `runtime` 层统一调度
//! - 工具执行结果包含结构化 metadata，供前端渲染（如终端视图、diff 视图）
//!
//! ## 工具列表
//!
//! | 工具名 | 功能 | 副作用级别 |
//! |--------|------|------------|
//! | `readFile` | 读取 UTF-8 文本文件，支持 `maxBytes` 截断 | None（只读） |
//! | `writeFile` | 写入/覆盖文件，自动生成 diff 报告 | Workspace |
//! | `editFile` | 唯一字符串替换，要求 `oldStr` 在文件中仅出现一次 | Workspace |
//! | `listDir` | 浅层列出目录条目，返回名称和类型 | None（只读） |
//! | `findFiles` | 基于 glob 模式查找文件，支持递归 | None（只读） |
//! | `grep` | 正则搜索文件内容，返回匹配行及行号 | None（只读） |
//! | `shell` | 执行一次性非交互式 shell 命令，流式输出 stdout/stderr | External |
//!
//! ## 沙箱机制
//!
//! 所有文件系统工具通过 `resolve_path` 进行路径沙箱检查：
//! 1. 相对路径基于工作目录解析为绝对路径
//! 2. 拒绝任何逃逸出工作目录的路径（如 `../outside.txt`）
//! 3. 支持尚不存在的路径（用于 writeFile 创建新文件）

pub mod tools;

#[cfg(test)]
pub(crate) mod test_support;
