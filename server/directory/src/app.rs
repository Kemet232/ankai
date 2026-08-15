//! The directory server's real HTTP+JSON surface, per
//! `docs/adr/0008-identity-discovery-service.md`. Four routes, matching
//! `ankai_core::directory::DirectoryService`'s four methods exactly:
//! publish/lookup a device's `KeyPackage`s, publish/lookup a device's
//! current `EndpointAddr`. [`router`] builds the `axum::Router`;
//! `src/main.rs` binds it to a real TCP port.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use iroh::EndpointAddr;
use openmls::key_packages::KeyPackage;

use crate::auth::{self, AuthError};
use crate::db::{self, Storage, StorageError};
use crate::{now_unix, paths};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("malformed request body: {0}")]
    BadBody(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::Auth(AuthError::DeviceKeyMismatch) => StatusCode::FORBIDDEN,
            ApiError::Auth(_) => StatusCode::UNAUTHORIZED,
            ApiError::BadBody(_) => StatusCode::BAD_REQUEST,
            ApiError::Storage(_) | ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

pub fn router(storage: Arc<Storage>) -> Router {
    Router::new()
        .route(
            paths::KEY_PACKAGES_ROUTE,
            post(publish_key_package).get(lookup_key_packages),
        )
        .route(
            paths::ENDPOINT_ADDR_ROUTE,
            post(publish_endpoint_addr).get(lookup_endpoint_addr),
        )
        .with_state(storage)
}

fn header_pairs(headers: &HeaderMap) -> Vec<(&str, &str)> {
    headers
        .iter()
        .filter_map(|(k, v)| Some((k.as_str(), v.to_str().ok()?)))
        .collect()
}

/// Verifies `headers`/`body` are a validly-signed, TOFU-bindable request
/// for `device_id`. See `crate::auth` and `crate::db::Storage::
/// bind_device_key` for what each half of this actually checks.
fn authenticate(
    storage: &Storage,
    method: &str,
    path: &str,
    device_id: &str,
    headers: &HeaderMap,
    body: &[u8],
    now: i64,
) -> Result<(), ApiError> {
    let signed =
        auth::parse_and_verify(&header_pairs(headers), method, path, device_id, body, now)?;
    storage.bind_device_key(device_id, &signed.pubkey_hex, now)?;
    Ok(())
}

async fn publish_key_package(
    State(storage): State<Arc<Storage>>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let now = now_unix();
    let path = paths::key_packages_path(&device_id);
    authenticate(&storage, "POST", &path, &device_id, &headers, &body, now)?;

    // Parse-then-reserialize: validates the body is actually a well-formed
    // KeyPackage (not just arbitrary bytes) before it's persisted, and
    // gives storage a canonical JSON string regardless of the exact
    // whitespace/ordering the client sent.
    let key_package: KeyPackage =
        serde_json::from_slice(&body).map_err(|e| ApiError::BadBody(e.to_string()))?;
    let key_package_json =
        serde_json::to_string(&key_package).map_err(|e| ApiError::Internal(e.to_string()))?;

    tokio::task::spawn_blocking(move || {
        storage.insert_key_package(&device_id, &key_package_json, now)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))??;

    Ok(StatusCode::NO_CONTENT)
}

async fn lookup_key_packages(
    State(storage): State<Arc<Storage>>,
    Path(device_id): Path<String>,
) -> Result<Json<Vec<KeyPackage>>, ApiError> {
    let now = now_unix();
    let jsons = tokio::task::spawn_blocking(move || {
        storage.take_key_packages(&device_id, now, db::KEY_PACKAGE_MAX_AGE_SECS)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))??;

    let key_packages = jsons
        .into_iter()
        .map(|json| serde_json::from_str(&json).map_err(|e| ApiError::Internal(e.to_string())))
        .collect::<Result<Vec<KeyPackage>, ApiError>>()?;

    Ok(Json(key_packages))
}

async fn publish_endpoint_addr(
    State(storage): State<Arc<Storage>>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let now = now_unix();
    let path = paths::endpoint_addr_path(&device_id);
    authenticate(&storage, "POST", &path, &device_id, &headers, &body, now)?;

    let addr: EndpointAddr =
        serde_json::from_slice(&body).map_err(|e| ApiError::BadBody(e.to_string()))?;
    let addr_json = serde_json::to_string(&addr).map_err(|e| ApiError::Internal(e.to_string()))?;

    tokio::task::spawn_blocking(move || storage.upsert_endpoint_addr(&device_id, &addr_json, now))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

    Ok(StatusCode::NO_CONTENT)
}

async fn lookup_endpoint_addr(
    State(storage): State<Arc<Storage>>,
    Path(device_id): Path<String>,
) -> Result<Json<Option<EndpointAddr>>, ApiError> {
    let now = now_unix();
    let addr_json = tokio::task::spawn_blocking(move || {
        storage.get_endpoint_addr(&device_id, now, db::ENDPOINT_ADDR_TTL_SECS)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))??;

    let addr = addr_json
        .map(|json| serde_json::from_str(&json).map_err(|e| ApiError::Internal(e.to_string())))
        .transpose()?;

    Ok(Json(addr))
}
