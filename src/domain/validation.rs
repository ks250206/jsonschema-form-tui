use anyhow::{Result, anyhow};
use jsonschema::{Draft, JSONSchema};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct ValidationSummary {
    pub is_valid: bool,
    pub errors: Vec<String>,
}

impl ValidationSummary {
    pub fn status_line(&self) -> String {
        if self.is_valid {
            "ok".to_owned()
        } else {
            format!("{} errors", self.errors.len())
        }
    }
}

pub fn validate_document(schema: &Value, instance: &Value) -> Result<ValidationSummary> {
    let owned_schema = schema.clone();
    let compiled = JSONSchema::options()
        .with_draft(Draft::Draft202012)
        .compile(&owned_schema)
        .map_err(|err| anyhow!(err.to_string()))?;

    let errors: Vec<String> = compiled
        .validate(instance)
        .err()
        .map(|errs| errs.map(|err| err.to_string()).collect())
        .unwrap_or_default();

    Ok(ValidationSummary {
        is_valid: errors.is_empty(),
        errors,
    })
}
