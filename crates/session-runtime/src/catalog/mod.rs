//! Session catalog 事件与生命周期协调。
//!
//! catalog 事件的 canonical owner 已下沉到 `astrcode_core`；
//! session-runtime 这里只保留 re-export，负责生命周期编排与广播。

pub use astrcode_core::SessionCatalogEvent;
