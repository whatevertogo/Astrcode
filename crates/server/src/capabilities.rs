use astrcode_core::{AstrError, CapabilityRouter, ToolRegistry};
use astrcode_plugin::{PluginLoader, Supervisor};
use astrcode_protocol::plugin::{PeerDescriptor, PeerRole};
use astrcode_tools::tools::{
    edit_file::EditFileTool, find_files::FindFilesTool, grep::GrepTool, list_dir::ListDirTool,
    read_file::ReadFileTool, shell::ShellTool, write_file::WriteFileTool,
};

pub(crate) async fn load_runtime_capabilities(
) -> std::result::Result<(CapabilityRouter, Vec<Supervisor>), AstrError> {
    let registry = built_in_tool_registry();
    let mut builder = CapabilityRouter::builder().register_tool_registry(registry);
    let mut supervisors = Vec::new();

    let Some(raw_paths) = std::env::var_os("ASTRCODE_PLUGIN_DIRS") else {
        return builder.build().map(|router| (router, supervisors));
    };

    let search_paths = std::env::split_paths(&raw_paths).collect::<Vec<_>>();
    if search_paths.is_empty() {
        return builder.build().map(|router| (router, supervisors));
    }

    let loader = PluginLoader { search_paths };
    for manifest in loader.discover()? {
        let supervisor = loader
            .start(&manifest, server_peer_descriptor(), None)
            .await?;
        for invoker in supervisor.capability_invokers() {
            builder = builder.register_invoker(invoker);
        }
        log::info!("loaded plugin '{}'", manifest.name);
        supervisors.push(supervisor);
    }

    builder.build().map(|router| (router, supervisors))
}

fn built_in_tool_registry() -> ToolRegistry {
    ToolRegistry::builder()
        .register(Box::new(ShellTool::default()))
        .register(Box::new(ListDirTool::default()))
        .register(Box::new(ReadFileTool::default()))
        .register(Box::new(WriteFileTool::default()))
        .register(Box::new(EditFileTool::default()))
        .register(Box::new(FindFilesTool::default()))
        .register(Box::new(GrepTool::default()))
        .build()
}

fn server_peer_descriptor() -> PeerDescriptor {
    PeerDescriptor {
        id: "astrcode-server".to_string(),
        name: "astrcode-server".to_string(),
        role: PeerRole::Supervisor,
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_profiles: vec!["coding".to_string()],
        metadata: serde_json::Value::Null,
    }
}
