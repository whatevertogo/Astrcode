//! # 外部工具目录
//!
//! 为 `tool_search` 提供 MCP / plugin 工具的轻量索引，
//! 避免把外部工具完整 schema 常驻注入 prompt。

use std::sync::{Arc, RwLock};

use astrcode_protocol::capability::CapabilityDescriptor;

#[derive(Clone, Default)]
pub(crate) struct ExternalToolCatalog {
    descriptors: Arc<RwLock<Vec<CapabilityDescriptor>>>,
}

impl ExternalToolCatalog {
    pub(crate) fn replace_from_descriptors(&self, descriptors: &[CapabilityDescriptor]) {
        let filtered = descriptors
            .iter()
            .filter(|descriptor| {
                descriptor
                    .tags
                    .iter()
                    .any(|tag| tag == "source:mcp" || tag == "source:plugin")
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Ok(mut guard) = self.descriptors.write() {
            *guard = filtered;
        }
    }

    pub(crate) fn search(&self, query: &str, limit: usize) -> Vec<CapabilityDescriptor> {
        let normalized_query = query.trim().to_ascii_lowercase();
        let Ok(guard) = self.descriptors.read() else {
            return Vec::new();
        };
        let mut descriptors = guard.clone();
        if normalized_query.is_empty() {
            descriptors.sort_by(|left, right| left.name.cmp(&right.name));
            descriptors.truncate(limit);
            return descriptors;
        }

        descriptors.retain(|descriptor| matches_query(descriptor, &normalized_query));
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors.truncate(limit);
        descriptors
    }
}

fn matches_query(descriptor: &CapabilityDescriptor, query: &str) -> bool {
    descriptor.name.to_ascii_lowercase().contains(query)
        || descriptor.description.to_ascii_lowercase().contains(query)
        || descriptor
            .tags
            .iter()
            .any(|tag| tag.to_ascii_lowercase().contains(query))
}

#[cfg(test)]
mod tests {
    use astrcode_protocol::capability::{CapabilityDescriptor, CapabilityKind};
    use serde_json::json;

    use super::*;

    fn descriptor(name: &str, tag: &str) -> CapabilityDescriptor {
        CapabilityDescriptor::builder(name, CapabilityKind::tool())
            .description(format!("description for {name}"))
            .schema(json!({"type": "object"}), json!({"type": "object"}))
            .tag(tag)
            .build()
            .expect("descriptor should build")
    }

    #[test]
    fn only_keeps_external_tools() {
        let catalog = ExternalToolCatalog::default();
        catalog.replace_from_descriptors(&[
            descriptor("builtin", "builtin"),
            descriptor("mcp__demo__search", "source:mcp"),
            descriptor("plugin.search", "source:plugin"),
        ]);

        let names = catalog
            .search("", 10)
            .into_iter()
            .map(|descriptor| descriptor.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["mcp__demo__search", "plugin.search"]);
    }

    #[test]
    fn query_matches_name_and_description() {
        let catalog = ExternalToolCatalog::default();
        catalog.replace_from_descriptors(&[descriptor("mcp__demo__search", "source:mcp")]);
        assert_eq!(catalog.search("demo", 10).len(), 1);
        assert_eq!(catalog.search("description", 10).len(), 1);
    }
}
