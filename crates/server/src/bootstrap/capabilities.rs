//! # 能力装配
//!
//! 负责把内置工具适配为 `CapabilityRouter`，
//! 并在外部 surface（如 MCP）变化时同步刷新 kernel 能力面。

use std::sync::Arc;

use astrcode_adapter_tools::builtin_tools::{
    apply_patch::ApplyPatchTool, edit_file::EditFileTool, find_files::FindFilesTool,
    grep::GrepTool, list_dir::ListDirTool, read_file::ReadFileTool, shell::ShellTool,
    write_file::WriteFileTool,
};
use astrcode_core::{CapabilityInvoker, Result, Tool};
use astrcode_kernel::{CapabilityRouter, Kernel, ToolCapabilityInvoker};

pub(crate) fn build_builtin_capability_invokers() -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ReadFileTool),
        Arc::new(WriteFileTool),
        Arc::new(EditFileTool),
        Arc::new(ApplyPatchTool),
        Arc::new(ListDirTool),
        Arc::new(FindFilesTool),
        Arc::new(GrepTool),
        Arc::new(ShellTool),
    ];

    let invokers = tools
        .into_iter()
        .filter_map(|tool| match ToolCapabilityInvoker::new(tool) {
            Ok(invoker) => Some(Arc::new(invoker) as Arc<dyn CapabilityInvoker>),
            Err(error) => {
                log::error!("注册工具失败: {error}");
                None
            },
        })
        .collect();

    Ok(invokers)
}

pub(crate) fn build_server_capability_router(
    invokers: Vec<Arc<dyn CapabilityInvoker>>,
) -> Result<CapabilityRouter> {
    let router = CapabilityRouter::empty();
    router.register_invokers(invokers)?;
    Ok(router)
}

#[derive(Clone)]
pub(crate) struct CapabilitySurfaceSync {
    builtin_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    router: CapabilityRouter,
    kernel: Arc<Kernel>,
}

impl CapabilitySurfaceSync {
    pub(crate) fn new(
        kernel: Arc<Kernel>,
        builtin_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    ) -> Self {
        Self {
            router: kernel.gateway().capabilities().clone(),
            kernel,
            builtin_invokers,
        }
    }

    pub(crate) fn apply_external_invokers(
        &self,
        external_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    ) -> Result<()> {
        let mut invokers = self.builtin_invokers.clone();
        invokers.extend(external_invokers);
        self.router.replace_invokers(invokers.clone())?;
        self.kernel
            .surface()
            .replace_capabilities(&invokers, self.kernel.events());
        Ok(())
    }
}
