//! Error responses: `{"error": "..."}` with the status code.

use axum::{
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use poe_accounts::AccountError;
use serde::Serialize;
use utoipa::ToSchema;

/// Body of every error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    /// Seconds for the `Retry-After` header (429 answers).
    pub retry_after: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            retry_after: None,
        }
    }

    /// 429 with `Retry-After`: too many failed password attempts (`[security]`).
    pub fn too_many_attempts(retry_after_secs: u64) -> Self {
        let mut e = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            format!("too many failed login attempts, try again in {retry_after_secs} seconds"),
        );
        e.retry_after = Some(retry_after_secs);
        e
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn internal(what: impl std::fmt::Display) -> Self {
        tracing::error!("web api: {what}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut res = (
            self.status,
            axum::Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response();
        if let Some(secs) = self.retry_after {
            res.headers_mut()
                .insert(axum::http::header::RETRY_AFTER, secs.into());
        }
        res
    }
}

impl From<AccountError> for ApiError {
    fn from(e: AccountError) -> Self {
        match e {
            AccountError::NoAccount => Self::not_found(e.to_string()),
            AccountError::WrongPassword => Self::unauthorized(e.to_string()),
            AccountError::Disabled => Self::forbidden(e.to_string()),
            AccountError::TokenUnknown
            | AccountError::TokenExpired
            | AccountError::TokenRevoked
            | AccountError::TokenWrongCluster => Self::unauthorized(e.to_string()),
            AccountError::NameTaken => Self::new(StatusCode::CONFLICT, e.to_string()),
            AccountError::InvalidName(_) | AccountError::WeakPassword => {
                Self::bad_request(e.to_string())
            }
            AccountError::Db(_) => Self::internal(e),
        }
    }
}

impl From<poe_store::StoreError> for ApiError {
    fn from(e: poe_store::StoreError) -> Self {
        Self::internal(e)
    }
}

impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> Self {
        Self::new(r.status(), r.body_text())
    }
}

/// `Json` that reports a bad body as a JSON [`ApiError`].
#[derive(axum::extract::FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);
