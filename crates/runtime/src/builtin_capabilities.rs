use std::sync::Arc;

use astrcode_core::{CapabilityInvoker, Result, ToolCapabilityInvoker};

pub(crate) fn built_in_capability_invokers() -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    vec![
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::shell::ShellTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::list_dir::ListDirTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::read_file::ReadFileTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::write_file::WriteFileTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::edit_file::EditFileTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::find_files::FindFilesTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::grep::GrepTool)),
    ]
    .into_iter()
    .collect()
}
