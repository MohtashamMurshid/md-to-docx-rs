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
}

impl MarkdownConversionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            context: None,
        }
    }

    pub fn with_context(message: impl Into<String>, context: ErrorContext) -> Self {
        Self {
            message: message.into(),
            context: if context.is_empty() {
                None
            } else {
                Some(context)
            },
        }
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
