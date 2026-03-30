use std::sync::Arc;

use astrcode_core::{CapabilityInvoker, ToolCapabilityInvoker};

pub(crate) fn built_in_capability_invokers() -> Vec<Arc<dyn CapabilityInvoker>> {
    vec![
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::shell::ShellTool::default())),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_tools::tools::list_dir::ListDirTool::default(),
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_tools::tools::read_file::ReadFileTool::default(),
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_tools::tools::write_file::WriteFileTool::default(),
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_tools::tools::edit_file::EditFileTool::default(),
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_tools::tools::find_files::FindFilesTool::default(),
        )),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::grep::GrepTool::default())),
    ]
}
