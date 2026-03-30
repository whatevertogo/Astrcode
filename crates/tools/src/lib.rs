//! # Astrcode 内置工具
//!
//! 本库包含 Astrcode 内置的工具实现：
//!
//! - **read_file**: 读取文件内容
//! - **write_file**: 写入文件
//! - **edit_file**: 编辑文件（替换内容）
//! - **list_dir**: 列出目录
//! - **find_files**: 查找文件
//! - **grep**: 搜索文件内容
//! - **shell**: 执行 Shell 命令

pub mod tools;

#[cfg(test)]
pub(crate) mod test_support;
