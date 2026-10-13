//! HTTP route handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Query, State},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;

use crate::{
    config::Limits,
    convert::{OutputFormat, convert_all},
    error::ApiError,
    shapefile, upload,
};

pub fn router(limits: Arc<Limits>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/convert", post(convert))
        .layer(DefaultBodyLimit::max(limits.max_upload_bytes + 64 * 1024))
        .with_state(limits)
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
}

async fn healthz() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

#[derive(serde::Deserialize)]
struct ConvertQuery {
    f: Option<String>,
}

async fn convert(
    State(limits): State<Arc<Limits>>,
    Query(query): Query<ConvertQuery>,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    let format = OutputFormat::from_query(query.f.as_deref())?.unwrap_or(OutputFormat::GeoJson);
    let zip_bytes = read_file_field(multipart, &limits).await?;

    let tmp = upload::unzip(&zip_bytes, &limits)?;
    let shp_paths = shapefile::discover(tmp.path())?;
    if shp_paths.is_empty() {
        return Err(ApiError::NoShapefile);
    }

    let mut shapefiles = Vec::with_capacity(shp_paths.len());
    for path in &shp_paths {
        let data = shapefile::read(path)?;
        tracing::debug!(
            file = %path.display(),
            features = data.features.len(),
            "read shapefile"
        );
        shapefiles.push(data);
    }

    let total_features: usize = shapefiles.iter().map(|s| s.features.len()).sum();
    let (body, content_type) = convert_all(shapefiles, format)?;
    tracing::info!(
        format = ?format,
        features = total_features,
        bytes = body.len(),
        "converted"
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Read the `file` field from the multipart body into memory, enforcing the
/// upload size cap while streaming chunks.
async fn read_file_field(mut multipart: Multipart, limits: &Limits) -> Result<Vec<u8>, ApiError> {
    let mut zip_bytes: Option<Vec<u8>> = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("malformed multipart body: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let mut buf = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ApiError::BadRequest(format!("failed reading upload: {e}")))?
        {
            if buf.len() + chunk.len() > limits.max_upload_bytes {
                return Err(ApiError::UploadTooLarge {
                    max: limits.max_upload_bytes,
                });
            }
            buf.extend_from_slice(&chunk);
        }
        zip_bytes = Some(buf);
        break;
    }
    zip_bytes.ok_or(ApiError::MissingFile)
}

async fn not_found(_req: Request<Body>) -> ApiError {
    ApiError::NotFound
}

async fn method_not_allowed(_req: Request<Body>) -> ApiError {
    ApiError::MethodNotAllowed
}
