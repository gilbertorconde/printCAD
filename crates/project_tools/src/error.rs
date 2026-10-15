//! Failure stages stay distinct from conversion losses.

use kernel_api::KernelError;
use project_archive::archive::FcstdError;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Archive,
    Parse,
    Conversion,
    KernelInitialization,
    Rebuild,
    Stationarity,
    Panic,
}

#[derive(Debug)]
pub enum ImportError {
    Source(FcstdError),
    KernelInitialization(KernelError),
}

impl ImportError {
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Source(_) => ErrorCategory::Conversion,
            Self::KernelInitialization(_) => ErrorCategory::KernelInitialization,
        }
    }
}

impl From<FcstdError> for ImportError {
    fn from(error: FcstdError) -> Self {
        Self::Source(error)
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(f),
            Self::KernelInitialization(error) => write!(f, "kernel initialization: {error}"),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Source(error) => error,
            Self::KernelInitialization(error) => error,
        })
    }
}

/// Serializable diagnostics retain the stage and the complete underlying message.
#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub category: ErrorCategory,
    pub reason: String,
}

impl Failure {
    pub fn new(category: ErrorCategory, error: impl fmt::Display) -> Self {
        Self {
            category,
            reason: error.to_string(),
        }
    }
}

impl From<ImportError> for Failure {
    fn from(error: ImportError) -> Self {
        Self::new(error.category(), error)
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.category, self.reason)
    }
}
