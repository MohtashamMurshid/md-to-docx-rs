use std::collections::BTreeMap;
use std::fmt;

/// String context attached to a conversion failure.
///
/// Keys match the TypeScript error context (`code`, `phase`, `styleContext`,
/// and the rest). Non-text values are formatted into strings by the caller.
pub type ErrorContext = BTreeMap<String, String>;

/// Failure from conversion or from option checks such as a blank `fontFamily`.
///
/// Display and [`std::error::Error`] expose `message` only, so a test that
/// matches a substring still sees the same text as the TypeScript error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownConversionError {
    pub message: String,
    pub context: Option<ErrorContext>,
    /// Typed context for the Node API. Rust callers retain the string map.
    pub structured_context: Option<serde_json::Value>,
}

impl MarkdownConversionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            context: None,
            structured_context: None,
        }
    }

    pub fn with_context(message: impl Into<String>, context: ErrorContext) -> Self {
        Self {
            message: message.into(),
            structured_context: None,
            context: if context.is_empty() {
                None
            } else {
                Some(context)
            },
        }
    }

    pub fn with_json_context(message: impl Into<String>, context: serde_json::Value) -> Self {
        let strings = context.as_object().map(|values| {
            values
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string()),
                    )
                })
                .collect()
        });
        Self {
            message: message.into(),
            context: strings,
            structured_context: Some(context),
        }
    }

    pub fn context_json(&self) -> Option<serde_json::Value> {
        self.structured_context.clone().or_else(|| {
            self.context
                .as_ref()
                .map(|context| serde_json::json!(context))
        })
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn context(&self) -> Option<&ErrorContext> {
        self.context.as_ref()
    }
}

impl fmt::Display for MarkdownConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MarkdownConversionError {}

impl From<&str> for MarkdownConversionError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<String> for MarkdownConversionError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_context_preserves_json_types_and_rust_string_access() {
        let context =
            serde_json::json!({"length": 42, "enabled": false, "details": {"part": "body"}});
        let error = MarkdownConversionError::with_json_context("failed", context.clone());
        assert_eq!(error.context_json(), Some(context));
        assert_eq!(error.context().unwrap()["length"], "42");
        assert_eq!(error.context().unwrap()["enabled"], "false");
    }

    #[test]
    fn legacy_context_remains_a_json_string_map() {
        let error = MarkdownConversionError::with_context(
            "failed",
            BTreeMap::from([("length".into(), "42".into())]),
        );
        assert_eq!(
            error.context_json(),
            Some(serde_json::json!({"length": "42"}))
        );
        assert_eq!(MarkdownConversionError::new("failed").context_json(), None);
    }
}
