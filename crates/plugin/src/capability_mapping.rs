//! 插件协议描述符与宿主内部 `CapabilitySpec` 的边界映射。
//!
//! Why: `protocol` crate 只承载 wire types，不负责宿主内部模型转换。

use astrcode_core::{
    CapabilityKind as CoreCapabilityKind, CapabilitySpec, CapabilitySpecBuildError,
    InvocationMode as CoreInvocationMode, PermissionSpec, SideEffect as CoreSideEffect,
    Stability as CoreStability,
};
use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityKind, DescriptorBuildError, PermissionHint, SideEffectLevel,
    StabilityLevel,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CapabilityMappingError {
    #[error("invalid capability descriptor: {0}")]
    InvalidDescriptor(#[from] CapabilitySpecBuildError),
    #[error("invalid capability spec: {0}")]
    InvalidSpec(#[from] DescriptorBuildError),
}

pub fn descriptor_to_spec(
    descriptor: &CapabilityDescriptor,
) -> std::result::Result<CapabilitySpec, CapabilityMappingError> {
    let mut builder = CapabilitySpec::builder(
        descriptor.name.clone(),
        CoreCapabilityKind::from(descriptor.kind.as_str()),
    )
    .description(descriptor.description.clone())
    .schema(
        descriptor.input_schema.clone(),
        descriptor.output_schema.clone(),
    )
    .invocation_mode(if descriptor.streaming {
        CoreInvocationMode::Streaming
    } else {
        CoreInvocationMode::Unary
    })
    .concurrency_safe(descriptor.concurrency_safe)
    .compact_clearable(descriptor.compact_clearable)
    .profiles(descriptor.profiles.clone())
    .tags(descriptor.tags.clone())
    .permissions(
        descriptor
            .permissions
            .iter()
            .map(|permission| PermissionSpec {
                name: permission.name.clone(),
                rationale: permission.rationale.clone(),
            })
            .collect(),
    )
    .side_effect(match descriptor.side_effect {
        SideEffectLevel::None => CoreSideEffect::None,
        SideEffectLevel::Local => CoreSideEffect::Local,
        SideEffectLevel::Workspace => CoreSideEffect::Workspace,
        SideEffectLevel::External => CoreSideEffect::External,
    })
    .stability(match descriptor.stability {
        StabilityLevel::Experimental => CoreStability::Experimental,
        StabilityLevel::Stable => CoreStability::Stable,
        StabilityLevel::Deprecated => CoreStability::Deprecated,
    })
    .metadata(descriptor.metadata.clone());
    if let Some(size) = descriptor.max_result_inline_size {
        builder = builder.max_result_inline_size(size);
    }
    Ok(builder.build()?)
}

pub fn spec_to_descriptor(
    spec: &CapabilitySpec,
) -> std::result::Result<CapabilityDescriptor, CapabilityMappingError> {
    let mut builder =
        CapabilityDescriptor::builder(spec.name.as_str(), CapabilityKind::new(spec.kind.as_str()))
            .description(spec.description.clone())
            .schema(spec.input_schema.clone(), spec.output_schema.clone())
            .streaming(matches!(
                spec.invocation_mode,
                CoreInvocationMode::Streaming
            ))
            .concurrency_safe(spec.concurrency_safe)
            .compact_clearable(spec.compact_clearable)
            .profiles(spec.profiles.clone())
            .tags(spec.tags.clone())
            .permissions(
                spec.permissions
                    .iter()
                    .map(|permission| PermissionHint {
                        name: permission.name.clone(),
                        rationale: permission.rationale.clone(),
                    })
                    .collect(),
            )
            .side_effect(match spec.side_effect {
                CoreSideEffect::None => SideEffectLevel::None,
                CoreSideEffect::Local => SideEffectLevel::Local,
                CoreSideEffect::Workspace => SideEffectLevel::Workspace,
                CoreSideEffect::External => SideEffectLevel::External,
            })
            .stability(match spec.stability {
                CoreStability::Experimental => StabilityLevel::Experimental,
                CoreStability::Stable => StabilityLevel::Stable,
                CoreStability::Deprecated => StabilityLevel::Deprecated,
            })
            .metadata(spec.metadata.clone());
    if let Some(size) = spec.max_result_inline_size {
        builder = builder.max_result_inline_size(size);
    }
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilityKind, CapabilitySpec, InvocationMode, SideEffect, Stability};
    use serde_json::json;

    use super::{descriptor_to_spec, spec_to_descriptor};

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
        let descriptor = spec_to_descriptor(&spec).expect("spec->descriptor should pass");
        let mapped = descriptor_to_spec(&descriptor).expect("descriptor->spec should pass");
        assert_eq!(mapped, spec);
    }
}
