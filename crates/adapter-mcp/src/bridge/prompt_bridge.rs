//! # MCP Prompt 桥接
//!
//! 将 MCP 服务器握手响应中的 `instructions`
//! 转换为 `PromptDeclaration`，注入到 Astrcode 的 prompt 组装管线。

use astrcode_prompt_contract::{
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, SystemPromptLayer,
};

/// 将 MCP 服务器的 instructions 转换为 PromptDeclaration。
///
/// instructions 是握手时服务器提供的全局指令文本，
/// 映射为 ExtensionInstruction 类型的 prompt block。
pub fn instructions_to_prompt_declaration(
    server_name: &str,
    instructions: &str,
) -> PromptDeclaration {
    PromptDeclaration {
        block_id: format!("mcp__{}__instructions", server_name),
        title: format!("MCP Server: {}", server_name),
        content: instructions.to_string(),
        render_target: PromptDeclarationRenderTarget::System,
        layer: SystemPromptLayer::default(),
        kind: PromptDeclarationKind::ExtensionInstruction,
        priority_hint: None,
        always_include: false,
        source: PromptDeclarationSource::Mcp,
        origin: Some(server_name.to_string()),
        capability_name: None,
    }
}

/// 从 instructions 生成 PromptDeclaration 列表。
///
/// `prompts/list` 返回的模板不再被常驻注入 system prompt，而是通过
/// `prompt_tool` 暴露为可调用 capability；这里仅保留真正的 server
/// instructions。
pub fn collect_prompt_declarations(
    server_name: &str,
    instructions: Option<&str>,
) -> Vec<PromptDeclaration> {
    let mut declarations = Vec::new();

    // 添加 instructions
    if let Some(instructions) = instructions {
        if !instructions.is_empty() {
            declarations.push(instructions_to_prompt_declaration(
                server_name,
                instructions,
            ));
        }
    }

    declarations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instructions_to_prompt_declaration() {
        let decl = instructions_to_prompt_declaration("my-server", "Be helpful and concise");
        assert_eq!(decl.block_id, "mcp__my-server__instructions");
        assert_eq!(decl.title, "MCP Server: my-server");
        assert_eq!(decl.content, "Be helpful and concise");
        assert_eq!(decl.source, PromptDeclarationSource::Mcp);
        assert_eq!(decl.origin.as_deref(), Some("my-server"));
        assert_eq!(decl.kind, PromptDeclarationKind::ExtensionInstruction);
    }

    #[test]
    fn test_collect_prompt_declarations_with_instructions() {
        let decls = collect_prompt_declarations("srv", Some("instructions text"));
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].block_id, "mcp__srv__instructions");
    }

    #[test]
    fn test_collect_prompt_declarations_no_instructions() {
        let decls = collect_prompt_declarations("srv", None);
        assert!(decls.is_empty());
    }

    #[test]
    fn test_collect_prompt_declarations_empty_instructions() {
        let decls = collect_prompt_declarations("srv", Some(""));
        assert!(decls.is_empty());
    }
}
