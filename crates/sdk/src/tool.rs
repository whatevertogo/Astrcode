//! 工具处理器 SDK。
//!
//! 本模块提供插件作者注册和实现工具的核心接口。
//!
//! ## 核心抽象
//!
//! - **`ToolHandler`**: 类型安全的工具处理 trait，插件作者实现此 trait 来定义工具逻辑
//! - **`ToolRegistration`**: 将 `ToolHandler` 包装为可被运行时调用的注册项
//! - **`DynToolHandler`**: 类型擦除后的动态分发 trait，由运行时内部使用
//!
//! ## 类型擦除设计
//!
//! 插件作者实现的是泛型 `ToolHandler<I, O>`（输入/输出为具体 Rust 类型），
//! 但运行时只知道 `Value`（JSON）。`ErasedToolHandler` 在中间层负责
//! `Value <-> I/O` 的 serde 转换，并统一错误处理。
//!
//! 这样插件作者只需关注业务逻辑，无需手动处理 JSON 编解码。
//!
//! ## 使用示例
//!
//! ```ignore
//! struct MyTool;
//!
//! impl ToolHandler<MyInput, MyOutput> for MyTool {
//!     fn descriptor(&self) -> CapabilitySpec { /* ... */ }
//!
//!     fn execute(&self, input: MyInput, context: PluginContext, stream: StreamWriter) -> ToolFuture<'_, MyOutput> {
//!         Box::pin(async move {
//!             // 业务逻辑
//!             Ok(MyOutput { result: input.value })
//!         })
//!     }
//! }
//!
//! let registration = ToolRegistration::new(MyTool);
//! ```

use std::{future::Future, pin::Pin};

use astrcode_core::CapabilitySpec;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{PluginContext, SdkError, StreamWriter, ToolSerdeStage};

/// 工具执行的返回类型别名。
///
/// 所有工具执行结果都包装在此 Result 中，
/// 成功时返回输出类型 `T`，失败时返回 `SdkError`。
pub type ToolResult<T> = Result<T, SdkError>;

/// 工具执行返回的 Future 类型。
///
/// 使用 `Pin<Box<dyn Future>>` 是因为 `ToolHandler::execute`
/// 需要返回 trait object，而 async trait 在稳定 Rust 中
/// 需要通过这种方式实现类型擦除。
pub type ToolFuture<'a, T> = Pin<Box<dyn Future<Output = ToolResult<T>> + Send + 'a>>;

/// 类型安全的工具处理 trait。
///
/// 插件作者实现此 trait 来定义工具的行为。
/// 泛型参数 `I` 和 `O` 分别是工具的输入和输出类型，
/// 由 serde 自动处理 JSON 编解码。
///
/// ## 生命周期
///
/// `ToolHandler` 实例通常被 `Arc` 或 `Box` 包装后注册到运行时，
/// 因此需要 `Send + Sync`。`execute` 返回的 future 生命周期
/// 绑定在 `&self` 上，因为工具实例在调用期间保持存活。
///
/// ## 为什么 `execute` 返回 `ToolFuture` 而不是 `async fn`
///
/// trait 中的 `async fn` 在稳定 Rust 中需要 `async_trait` crate，
/// 此处手动返回 `Pin<Box<dyn Future>>` 避免额外依赖，
/// 同时保持与 `async fn` 相同的语义。
pub trait ToolHandler<I = Value, O = Value>: Send + Sync {
    /// 返回工具的能力描述。
    ///
    /// 描述包含工具名称、文档、副作用级别等元数据，
    /// 用于 LLM 决定是否调用此工具，以及前端如何渲染工具卡片。
    fn descriptor(&self) -> CapabilitySpec;

    /// 执行工具逻辑。
    ///
    /// ## 参数
    ///
    /// - `input`: 已反序列化的工具输入，类型由泛型 `I` 决定
    /// - `context`: 当前调用的插件上下文（工作区、会话、追踪信息等）
    /// - `stream`: 流式写入器，用于发送增量输出
    ///
    /// ## 返回值
    ///
    /// 返回 `ToolFuture<'_, O>`，即一个产出 `ToolResult<O>` 的 future。
    /// 成功时返回输出值，失败时返回 `SdkError`。
    fn execute(&self, input: I, context: PluginContext, stream: StreamWriter) -> ToolFuture<'_, O>;
}

/// 为 `Box<T>` 实现 `ToolHandler`，允许工具处理器被装箱。
///
/// 这在工具需要动态分发或存储在集合中时很有用，
/// 确保装箱后的工具仍然保持类型安全的 `ToolHandler` 接口。
impl<T, I, O> ToolHandler<I, O> for Box<T>
where
    T: ToolHandler<I, O> + ?Sized,
{
    fn descriptor(&self) -> CapabilitySpec {
        (**self).descriptor()
    }

    fn execute(&self, input: I, context: PluginContext, stream: StreamWriter) -> ToolFuture<'_, O> {
        (**self).execute(input, context, stream)
    }
}

/// 类型擦除后的动态分发工具处理 trait。
///
/// 运行时内部使用此 trait 调用工具，不关心工具的具体输入/输出类型。
/// 所有输入/输出都通过 `Value`（JSON）传递，serde 转换由 `ErasedToolHandler` 处理。
///
/// ## 为什么需要这个 trait
///
/// 运行时维护一个 `Vec<Box<dyn DynToolHandler>>` 集合，
/// 如果直接用 `ToolHandler<I, O>`，集合中的每个元素类型都不同，
/// 无法统一存储。类型擦除后所有工具都实现同一个 trait，可放入同一集合。
pub trait DynToolHandler: Send + Sync {
    /// 返回工具的能力描述。
    fn descriptor(&self) -> CapabilitySpec;

    /// 以 `Value` 作为输入/输出执行工具。
    ///
    /// 内部实现会先将 `Value` 反序列化为具体类型，
    /// 调用类型安全的 `ToolHandler::execute`，
    /// 再将结果序列化为 `Value` 返回。
    fn execute_value(
        &self,
        input: Value,
        context: PluginContext,
        stream: StreamWriter,
    ) -> ToolFuture<'_, Value>;
}

/// 类型擦除适配器，将 `ToolHandler<I, O>` 包装为 `DynToolHandler`。
///
/// 此结构体不公开，插件作者无需直接与之交互。
/// 它由 `ToolRegistration::new` 内部创建，负责：
/// 1. 将输入的 `Value` 反序列化为 `I`
/// 2. 调用内部 `ToolHandler::execute`
/// 3. 将输出的 `O` 序列化为 `Value`
/// 4. 统一处理 serde 错误为 `SdkError::Serde`
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
    fn descriptor(&self) -> CapabilitySpec {
        ToolHandler::<I, O>::descriptor(&self.inner)
    }

    fn execute_value(
        &self,
        input: Value,
        context: PluginContext,
        stream: StreamWriter,
    ) -> ToolFuture<'_, Value> {
        let capability_spec = ToolHandler::<I, O>::descriptor(&self.inner);
        let capability_name = capability_spec.name.to_string();
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

/// 工具注册项。
///
/// 将 `ToolHandler` 与其能力描述打包，
/// 是插件向运行时注册工具的最小单元。
///
/// ## 使用方式
///
/// 插件作者通过 `ToolRegistration::new(handler)` 创建注册项，
/// 然后将其传递给运行时。运行时通过 `descriptor()` 获取工具元数据，
/// 通过 `handler()` 进行动态分发调用。
///
/// ## 类型擦除
///
/// 构造函数内部创建 `ErasedToolHandler` 包装器，
/// 将泛型 `ToolHandler<I, O>` 转换为 `dyn DynToolHandler`，
/// 使运行时可用统一接口调用所有工具。
pub struct ToolRegistration {
    descriptor: CapabilitySpec,
    handler: Box<dyn DynToolHandler>,
}

impl ToolRegistration {
    /// 从 `ToolHandler` 创建工具注册项。
    ///
    /// 此方法会自动：
    /// 1. 从 handler 提取能力描述
    /// 2. 创建类型擦除包装器
    /// 3. 打包为 `ToolRegistration`
    ///
    /// ## 泛型约束
    ///
    /// - `I: DeserializeOwned`: 输入类型必须可从 JSON 反序列化
    /// - `O: Serialize`: 输出类型必须可序列化为 JSON
    /// - `'static`: handler 必须拥有所有数据，不能有非静态引用
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

    /// 返回工具的能力描述引用。
    ///
    /// 运行时用此信息向 LLM 暴露工具列表，
    /// 前端用此信息渲染工具卡片。
    pub fn descriptor(&self) -> &CapabilitySpec {
        &self.descriptor
    }

    /// 返回类型擦除后的处理器引用。
    ///
    /// 运行时通过此接口以 `Value` 为输入/输出调用工具，
    /// serde 转换由内部的 `ErasedToolHandler` 处理。
    pub fn handler(&self) -> &dyn DynToolHandler {
        self.handler.as_ref()
    }
}
