//! server 内部应用用例层。
//!
//! 这里承载 HTTP 之外的业务编排：root execution、agent 协作、治理面、执行配置等。

pub(crate) mod agent;
pub(crate) mod error;
pub(crate) mod execution;
pub(crate) mod governance_surface;
pub(crate) mod lifecycle;
pub(crate) mod root_execute;
pub(crate) mod route_error;
