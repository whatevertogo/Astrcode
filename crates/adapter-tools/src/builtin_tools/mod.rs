//! # 工具模块
//!
//! 所有内置工具的具体实现，每个工具对应一个独立模块。
//!
//! 工具通过实现 `astrcode_core::Tool` trait 提供：
//! - `definition()`: 工具名称、描述、JSON Schema 参数定义
//! - `capability_metadata()`: 权限、副作用级别、Prompt 元数据
//! - `execute()`: 实际执行逻辑

/// 统一 diff 补丁应用工具：多文件 patch
pub mod apply_patch;
/// 文件编辑工具：唯一字符串替换
pub mod edit_file;
/// 文件查找工具：glob 模式匹配
pub mod find_files;
/// 文件系统公共工具：路径解析、取消检查、diff 生成
pub mod fs_common;
/// 内容搜索工具：正则匹配
pub mod grep;
/// 目录列表工具：浅层条目枚举
pub mod list_dir;
/// 文件读取工具：UTF-8 文本读取
pub mod read_file;
/// Shell 命令执行工具：流式 stdout/stderr
pub mod shell;
/// 技能工具：按需加载 skill 指令
pub mod skill_tool;
/// 外部工具搜索：按需展开 MCP/plugin 工具 schema
pub mod tool_search;
/// 文件写入工具：创建/覆盖文本文件
pub mod write_file;
