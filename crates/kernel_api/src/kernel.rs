//! The error and result every kernel operation returns.

use thiserror::Error;

/// Convenience alias for kernel fallible operations.
pub type KernelResult<T> = Result<T, KernelError>;

/// Standardized error type for kernel interactions.
#[derive(Debug, Error)]
pub enum KernelError {
    #[error("kernel initialization failed: {0}")]
    Initialization(String),
    #[error("kernel not initialized")]
    NotInitialized,
    #[error("operation unsupported: {0}")]
    Unsupported(String),
    #[error("invalid kernel input: {0}")]
    InvalidInput(String),
    #[error("import failed: {0}")]
    Import(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
