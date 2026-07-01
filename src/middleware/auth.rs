use std::{fmt, sync::Arc};

use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct AdminApiKey {
    value: Arc<str>,
}

impl AdminApiKey {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthConfigError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(AuthConfigError::EmptyAdminApiKey);
        }

        Ok(Self {
            value: Arc::from(value),
        })
    }

    fn matches(&self, candidate: &str) -> bool {
        constant_time_eq(self.value.as_bytes(), candidate.as_bytes())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthConfigError {
    EmptyAdminApiKey,
}

impl fmt::Display for AuthConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAdminApiKey => formatter.write_str("ADMIN_API_KEY must not be empty"),
        }
    }
}

impl std::error::Error for AuthConfigError {}

#[derive(Debug, Serialize)]
struct AuthErrorResponse {
    error: &'static str,
}

pub async fn require_admin_api_key(
    State(admin_api_key): State<AdminApiKey>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request_has_valid_admin_key(request.headers(), &admin_api_key) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(AuthErrorResponse {
                error: "valid admin API key required",
            }),
        )
            .into_response()
    }
}

fn request_has_valid_admin_key(headers: &HeaderMap, admin_api_key: &AdminApiKey) -> bool {
    bearer_token(headers)
        .or_else(|| header_value(headers, "x-admin-api-key"))
        .is_some_and(|candidate| admin_api_key.matches(candidate))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = header_value(headers, "authorization")?;
    let (scheme, token) = value.split_once(' ')?;

    if scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty() {
        Some(token.trim())
    } else {
        None
    }
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok().map(str::trim)
}

fn constant_time_eq(expected: &[u8], candidate: &[u8]) -> bool {
    let mut diff = expected.len() ^ candidate.len();
    let max_len = expected.len().max(candidate.len());

    for index in 0..max_len {
        let expected_byte = expected.get(index).copied().unwrap_or(0);
        let candidate_byte = candidate.get(index).copied().unwrap_or(0);
        diff |= usize::from(expected_byte ^ candidate_byte);
    }

    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn accepts_bearer_token() {
        let key = AdminApiKey::new("secret").expect("key");
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer secret"));

        assert!(request_has_valid_admin_key(&headers, &key));
    }

    #[test]
    fn accepts_admin_key_header() {
        let key = AdminApiKey::new("secret").expect("key");
        let mut headers = HeaderMap::new();
        headers.insert("x-admin-api-key", HeaderValue::from_static("secret"));

        assert!(request_has_valid_admin_key(&headers, &key));
    }

    #[test]
    fn rejects_missing_or_wrong_key() {
        let key = AdminApiKey::new("secret").expect("key");
        let mut headers = HeaderMap::new();
        assert!(!request_has_valid_admin_key(&headers, &key));

        headers.insert("authorization", HeaderValue::from_static("Bearer wrong"));
        assert!(!request_has_valid_admin_key(&headers, &key));
    }
}
