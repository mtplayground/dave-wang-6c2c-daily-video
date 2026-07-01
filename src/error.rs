use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Repository,
    Storage,
    VideoProvider,
    ImageTo3DProvider,
    ProviderTimeout,
    ProviderQuota,
    ProviderTransient,
    Media,
    Io,
    NotFound,
    InvalidState,
}

impl ErrorCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Storage => "storage",
            Self::VideoProvider => "video_provider",
            Self::ImageTo3DProvider => "image_to_3d_provider",
            Self::ProviderTimeout => "provider_timeout",
            Self::ProviderQuota => "provider_quota",
            Self::ProviderTransient => "provider_transient",
            Self::Media => "media",
            Self::Io => "io",
            Self::NotFound => "not_found",
            Self::InvalidState => "invalid_state",
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureRecord {
    pub category: ErrorCategory,
    pub message: String,
}

impl FailureRecord {
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    pub fn persistable_message(&self) -> String {
        format!("[{}] {}", self.category, self.message)
    }
}

pub fn classify_provider_message(message: &str) -> ErrorCategory {
    let normalized = message.to_ascii_lowercase();

    if contains_any(
        &normalized,
        &[
            "quota",
            "rate limit",
            "rate_limit",
            "resource_exhausted",
            "too many requests",
            "http 429",
        ],
    ) {
        ErrorCategory::ProviderQuota
    } else if contains_any(
        &normalized,
        &["timeout", "timed out", "deadline", "http 408"],
    ) {
        ErrorCategory::ProviderTimeout
    } else if contains_any(
        &normalized,
        &[
            "transient",
            "temporarily",
            "http 409",
            "http 425",
            "http 500",
            "http 502",
            "http 503",
            "http 504",
        ],
    ) {
        ErrorCategory::ProviderTransient
    } else {
        ErrorCategory::VideoProvider
    }
}

pub fn provider_http_category(status: u16) -> ErrorCategory {
    match status {
        408 => ErrorCategory::ProviderTimeout,
        429 => ErrorCategory::ProviderQuota,
        409 | 425 | 500..=599 => ErrorCategory::ProviderTransient,
        _ => ErrorCategory::VideoProvider,
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_messages_classify_quota_and_timeout_failures() {
        assert_eq!(
            classify_provider_message("Gemini/Veo returned HTTP 429: quota exceeded"),
            ErrorCategory::ProviderQuota
        );
        assert_eq!(
            classify_provider_message("provider timed out while polling"),
            ErrorCategory::ProviderTimeout
        );
        assert_eq!(
            classify_provider_message("Meshy returned HTTP 503: temporarily unavailable"),
            ErrorCategory::ProviderTransient
        );
    }

    #[test]
    fn failure_record_is_prefixed_for_persistence() {
        let record = FailureRecord::new(ErrorCategory::ProviderQuota, "quota exceeded");
        assert_eq!(record.persistable_message(), "[provider_quota] quota exceeded");
    }
}
