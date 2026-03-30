use std::future::Future;
use std::pin::Pin;

use astrcode_protocol::plugin::CapabilityDescriptor;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::{PluginContext, SdkError, StreamWriter, ToolSerdeStage};

pub type ToolResult<T> = Result<T, SdkError>;
pub type ToolFuture<'a, T> = Pin<Box<dyn Future<Output = ToolResult<T>> + Send + 'a>>;

pub trait ToolHandler<I = Value, O = Value>: Send + Sync {
    fn descriptor(&self) -> CapabilityDescriptor;

    fn execute(&self, input: I, context: PluginContext, stream: StreamWriter) -> ToolFuture<'_, O>;
}

impl<T, I, O> ToolHandler<I, O> for Box<T>
where
    T: ToolHandler<I, O> + ?Sized,
{
    fn descriptor(&self) -> CapabilityDescriptor {
        (**self).descriptor()
    }

    fn execute(&self, input: I, context: PluginContext, stream: StreamWriter) -> ToolFuture<'_, O> {
        (**self).execute(input, context, stream)
    }
}

pub trait DynToolHandler: Send + Sync {
    fn descriptor(&self) -> CapabilityDescriptor;

    fn execute_value(
        &self,
        input: Value,
        context: PluginContext,
        stream: StreamWriter,
    ) -> ToolFuture<'_, Value>;
}

struct ErasedToolHandler<H, I, O> {
    inner: H,
    _marker: std::marker::PhantomData<fn(I) -> O>,
}

impl<H, I, O> ErasedToolHandler<H, I, O> {
    fn new(inner: H) -> Self {
        Self {
            inner,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<H, I, O> DynToolHandler for ErasedToolHandler<H, I, O>
where
    H: ToolHandler<I, O> + Send + Sync,
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + 'static,
{
    fn descriptor(&self) -> CapabilityDescriptor {
        ToolHandler::<I, O>::descriptor(&self.inner)
    }

    fn execute_value(
        &self,
        input: Value,
        context: PluginContext,
        stream: StreamWriter,
    ) -> ToolFuture<'_, Value> {
        let descriptor = ToolHandler::<I, O>::descriptor(&self.inner);
        let capability_name = descriptor.name;
        let typed_input = serde_json::from_value::<I>(input).map_err(|source| SdkError::Serde {
            capability: capability_name.clone(),
            stage: ToolSerdeStage::DecodeInput,
            rust_type: std::any::type_name::<I>(),
            message: source.to_string(),
        });

        // The registration stores an erased handler so plugin authors only implement
        // typed logic once while the SDK owns serde conversion and consistent errors.
        Box::pin(async move {
            let typed_input = typed_input?;
            let output =
                ToolHandler::<I, O>::execute(&self.inner, typed_input, context, stream).await?;
            serde_json::to_value(output).map_err(|source| SdkError::Serde {
                capability: capability_name,
                stage: ToolSerdeStage::EncodeOutput,
                rust_type: std::any::type_name::<O>(),
                message: source.to_string(),
            })
        })
    }
}

pub struct ToolRegistration {
    descriptor: CapabilityDescriptor,
    handler: Box<dyn DynToolHandler>,
}

impl ToolRegistration {
    pub fn new<H, I, O>(handler: H) -> Self
    where
        H: ToolHandler<I, O> + 'static,
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
    {
        let descriptor = handler.descriptor();
        Self {
            descriptor,
            handler: Box::new(ErasedToolHandler::<H, I, O>::new(handler)),
        }
    }

    pub fn descriptor(&self) -> &CapabilityDescriptor {
        &self.descriptor
    }

    pub fn handler(&self) -> &dyn DynToolHandler {
        self.handler.as_ref()
    }
}
