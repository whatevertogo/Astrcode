//! File-backed storage implementations for Astrcode runtime services.

pub mod session;

use astrcode_core::store::{StoreError, StoreResult};

pub(crate) type Result<T> = StoreResult<T>;

pub(crate) struct AstrError;

impl AstrError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> StoreError {
        StoreError::Io {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn parse(context: impl Into<String>, source: serde_json::Error) -> StoreError {
        StoreError::Parse {
            context: context.into(),
            source,
        }
    }
}

pub(crate) fn internal_io_error(context: impl Into<String>) -> StoreError {
    StoreError::Io {
        context: context.into(),
        source: std::io::Error::other("storage invariant violation"),
    }
}
