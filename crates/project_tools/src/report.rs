use serde::{Deserialize, Serialize};

/// Every unsupported setting and frozen dependency is part of the conversion result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub object: String,
    pub category: String,
    pub reason: String,
}

pub fn note(findings: &mut Vec<Finding>, object: &str, category: &str, reason: impl Into<String>) {
    findings.push(Finding {
        object: object.into(),
        category: category.into(),
        reason: reason.into(),
    });
}
