//! The directory server's real HTTP+JSON surface, per
//! `docs/adr/0008-identity-discovery-service.md`. Four routes matching
//! `ankai_core::directory::DirectoryService`'s four methods exactly:
//! publish/lookup a device's `KeyPackage`s, publish/lookup a device's
//! current `EndpointAddr`. Plus two more, added alongside (not part of that
//! trait — see `crate::username`'s module doc comment for why usernames
//! stay a directory-server-only concept for now): claim/lookup a device's
//! username. [`router`] builds the `axum::Router`; `src/main.rs` binds it
//! to a real TCP port.
//!
//! Username claims (`POST .../username`) go through [`authenticate`], the
//! exact same signed-request verification every other publish route uses —
//! no new auth mechanism. Username lookups (`GET /v1/usernames/{username}`)
//! are unauthenticated, matching `lookup_key_packages`/`lookup_endpoint_addr`'s
//! existing open-read pattern (see the ADR's Auth section on why lookups
//! aren't gated here) — resolving a username to a `DeviceId` is no more
//! sensitive than looking up that `DeviceId`'s `KeyPackage`s directly.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use iroh::EndpointAddr;
use openmls::key_packages::KeyPackage;

use crate::auth::{self, AuthError};
use crate::db::{self, ClaimUsernameError, Storage, StorageError};
use crate::username::UsernameError;
use crate::{now_unix, paths};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("malformed request body: {0}")]
    BadBody(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Username(#[from] UsernameError),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<ClaimUsernameError> for ApiError {
    fn from(err: ClaimUsernameError) -> Self {
        match err {
            ClaimUsernameError::Storage(e) => ApiError::Storage(e),
            ClaimUsernameError::Username(e) => ApiError::Username(e),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::Auth(AuthError::DeviceKeyMismatch) => StatusCode::FORBIDDEN,
            ApiError::Auth(_) => StatusCode::UNAUTHORIZED,
            ApiError::BadBody(_) => StatusCode::BAD_REQUEST,
            ApiError::Username(UsernameError::Invalid(_)) => StatusCode::BAD_REQUEST,
            // Distinct from a bad-signature 401/pinned-key-mismatch 403 —
            // this is "your request was valid, but someone else already
            // holds this name," the standard meaning of 409 Conflict.
            ApiError::Username(UsernameError::AlreadyClaimed) => StatusCode::CONFLICT,
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
        .route(paths::USERNAME_ROUTE, post(claim_username))
        .route(paths::USERNAME_LOOKUP_ROUTE, get(lookup_username))
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

/// Request body for `POST /v1/devices/{device_id}/username` — just the
/// desired username, validated by `crate::username::validate` (via
/// `Storage::claim_username`) before it's ever written to storage.
#[derive(Debug, serde::Deserialize)]
struct ClaimUsernameRequest {
    username: String,
}

async fn claim_username(
    State(storage): State<Arc<Storage>>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let now = now_unix();
    let path = paths::username_path(&device_id);
    authenticate(&storage, "POST", &path, &device_id, &headers, &body, now)?;

    let request: ClaimUsernameRequest =
        serde_json::from_slice(&body).map_err(|e| ApiError::BadBody(e.to_string()))?;

    tokio::task::spawn_blocking(move || storage.claim_username(&device_id, &request.username, now))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

    Ok(StatusCode::NO_CONTENT)
}

async fn lookup_username(
    State(storage): State<Arc<Storage>>,
    Path(username): Path<String>,
) -> Result<Json<Option<String>>, ApiError> {
    let device_id = tokio::task::spawn_blocking(move || storage.lookup_username(&username))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

    Ok(Json(device_id))
}
