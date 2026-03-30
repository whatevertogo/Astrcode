use astrcode_core::ToolRegistry;

pub(crate) fn built_in_tool_registry() -> ToolRegistry {
    ToolRegistry::builder()
        .register(Box::new(astrcode_tools::tools::shell::ShellTool::default()))
        .register(Box::new(
            astrcode_tools::tools::list_dir::ListDirTool::default(),
        ))
        .register(Box::new(
            astrcode_tools::tools::read_file::ReadFileTool::default(),
        ))
        .register(Box::new(
            astrcode_tools::tools::write_file::WriteFileTool::default(),
        ))
        .register(Box::new(
            astrcode_tools::tools::edit_file::EditFileTool::default(),
        ))
        .register(Box::new(
            astrcode_tools::tools::find_files::FindFilesTool::default(),
        ))
        .register(Box::new(astrcode_tools::tools::grep::GrepTool::default()))
        .build()
}
