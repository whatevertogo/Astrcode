//! 插件协议描述符与宿主内部 `CapabilitySpec` 的边界映射。
//!
//! Why: `protocol` crate 只承载 wire types，不负责宿主内部模型转换。
//! `CapabilitySpec` 是宿主内部语义真相，`CapabilityWireDescriptor`
//! 只是握手/传输使用的 DTO 名称。

use astrcode_core::{CapabilitySpec, CapabilitySpecBuildError};
use astrcode_protocol::plugin::CapabilityWireDescriptor;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CapabilityMappingError {
    #[error("invalid capability payload: {0}")]
    InvalidCapability(#[from] CapabilitySpecBuildError),
}

pub fn wire_descriptor_to_spec(
    descriptor: &CapabilityWireDescriptor,
) -> std::result::Result<CapabilitySpec, CapabilityMappingError> {
    descriptor.validate()?;
    Ok(descriptor.clone())
}

pub fn spec_to_wire_descriptor(
    spec: &CapabilitySpec,
) -> std::result::Result<CapabilityWireDescriptor, CapabilityMappingError> {
    spec.validate()?;
    Ok(spec.clone())
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilityKind, CapabilitySpec, InvocationMode, SideEffect, Stability};
    use serde_json::json;

    use super::{spec_to_wire_descriptor, wire_descriptor_to_spec};

    fn sample_spec() -> CapabilitySpec {
        CapabilitySpec {
            name: "tool.echo".into(),
            kind: CapabilityKind::Tool,
            description: "echo".to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            invocation_mode: InvocationMode::Unary,
            concurrency_safe: true,
            compact_clearable: true,
            profiles: vec!["coding".to_string()],
            tags: vec!["builtin".to_string()],
            permissions: vec![],
            side_effect: SideEffect::None,
            stability: Stability::Stable,
            metadata: json!({ "prompt": { "summary": "x" } }),
            max_result_inline_size: Some(1024),
        }
    }

    #[test]
    fn round_trip_between_spec_and_descriptor() {
        let spec = sample_spec();
        let descriptor = spec_to_wire_descriptor(&spec).expect("spec->wire descriptor should pass");
        let mapped =
            wire_descriptor_to_spec(&descriptor).expect("wire descriptor->spec should pass");
        assert_eq!(mapped, spec);
    }
}
