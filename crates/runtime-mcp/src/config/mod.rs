//! # MCP 配置管理
//!
//! 负责从多个作用域加载 MCP 服务器配置，支持去重和环境变量展开。

mod types;

pub use types::*;
