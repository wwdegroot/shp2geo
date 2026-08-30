//! API errors mapped to HTTP responses.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unsupported output format '{0}', expected 'json' or 'fgb'")]
    UnsupportedFormat(String),

    #[error("missing multipart field 'file' (must contain a zip archive)")]
    MissingFile,

    #[error("upload exceeds size limit of {max} bytes")]
    UploadTooLarge { max: usize },

    #[error("extracted archive exceeds size limit of {max} bytes")]
    ExtractTooLarge { max: usize },

    #[error("invalid zip archive: {0}")]
    InvalidZip(String),

    #[error("unsafe or unsupported entry in zip archive: {0}")]
    UnsafeEntry(String),

    #[error("archive contains no shapefile (.shp)")]
    NoShapefile,

    #[error("{0}")]
    BadRequest(String),

    #[error("not found")]
    NotFound,

    #[error("method not allowed")]
    MethodNotAllowed,

    #[error("internal error: {0}")]
    Internal(String),
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            ApiError::UploadTooLarge { .. } | ApiError::ExtractTooLarge { .. } => {
                StatusCode::PAYLOAD_TOO_LARGE
            }
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        tracing::info!(%status, error = %self, "request rejected");
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}
