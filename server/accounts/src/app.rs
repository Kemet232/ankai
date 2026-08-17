//! The accounts server's real HTTP+JSON surface, per
//! `docs/adr/0009-account-system.md`. Four routes:
//!
//! - `POST .../devices` — add or refresh a device registration (also
//!   implicitly the *only* "create an account" step there is — see
//!   [`add_device`]'s doc comment for why there's no separate signup
//!   endpoint).
//! - `GET .../devices` — list an account's current (non-revoked) device
//!   registrations, unauthenticated, matching
//!   `docs/adr/0008-identity-discovery-service.md`'s existing open-read
//!   precedent for public key material.
//! - `POST .../devices/{device_id}/revoke` — revoke a device.
//! - `PUT`/`GET .../recovery-blob` — opaque encrypted-backup storage.
//!
//! [`router`] builds the `axum::Router`; `src/main.rs` binds it to a real
//! TCP port.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{post, put};
use axum::{Json, Router};

use ankai_core::account::{verify_device_registration, DeviceRegistration, REGISTRATION_SKEW_SECS};
use ankai_core::util::{decode_hex, encode_hex};

use crate::auth::{self, AuthError};
use crate::db::{
    DeviceRegistrationRow, RegisterDeviceError, Storage, StorageError, MAX_RECOVERY_BLOB_BYTES,
};
use crate::{now_unix, paths};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("malformed request body: {0}")]
    BadBody(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("invalid device registration: {0}")]
    InvalidRegistration(String),
    #[error("device id is already registered under a different account or key")]
    DeviceIdConflict,
    #[error("recovery blob exceeds the {MAX_RECOVERY_BLOB_BYTES}-byte limit")]
    RecoveryBlobTooLarge,
    #[error("not found")]
    NotFound,
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<RegisterDeviceError> for ApiError {
    fn from(err: RegisterDeviceError) -> Self {
        match err {
            RegisterDeviceError::Storage(e) => ApiError::Storage(e),
            RegisterDeviceError::DeviceIdConflict => ApiError::DeviceIdConflict,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::Auth(AuthError::AccountIdMismatch) => StatusCode::FORBIDDEN,
            ApiError::Auth(_) => StatusCode::UNAUTHORIZED,
            ApiError::BadBody(_) => StatusCode::BAD_REQUEST,
            ApiError::InvalidRegistration(_) => StatusCode::UNAUTHORIZED,
            ApiError::DeviceIdConflict => StatusCode::CONFLICT,
            ApiError::RecoveryBlobTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::Storage(_) | ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

pub fn router(storage: Arc<Storage>) -> Router {
    Router::new()
        .route(paths::DEVICES_ROUTE, post(add_device).get(list_devices))
        .route(paths::REVOKE_ROUTE, post(revoke_device))
        .route(
            paths::RECOVERY_BLOB_ROUTE,
            put(put_recovery_blob).get(get_recovery_blob),
        )
        .with_state(storage)
}

fn header_pairs(headers: &HeaderMap) -> Vec<(&str, &str)> {
    headers
        .iter()
        .filter_map(|(k, v)| Some((k.as_str(), v.to_str().ok()?)))
        .collect()
}

/// Body for `POST /v1/accounts/{account_id}/devices`. Carries the account's
/// raw public key alongside the signed [`DeviceRegistration`] because the
/// server has no other way to learn it on a brand-new account's very first
/// device (there is nothing to look up yet) — see [`add_device`]'s doc
/// comment.
#[derive(Debug, serde::Deserialize)]
struct AddDeviceRequest {
    account_public_key_hex: String,
    registration: DeviceRegistration,
}

/// Adds (or idempotently refreshes) a device registration. **This is also
/// the only "create an account" operation this server has** — there is no
/// separate signup/register-account endpoint, because there is nothing for
/// one to do: per `docs/adr/0009-account-system.md`, an account *is* a
/// keypair the moment it's generated, entirely client-side. The first time
/// any device registration arrives for a given `account_id`, this handler
/// implicitly creates that account's row.
///
/// Authorization here is **not** the generic envelope in [`crate::auth`] —
/// it's the [`DeviceRegistration`] body's own embedded account-root-key
/// signature, verified via
/// `ankai_core::account::verify_device_registration`. See that function's
/// doc comment and [`crate::auth`]'s module doc comment for why that's both
/// necessary and sufficient here, unlike the revoke/recovery-blob routes.
async fn add_device(
    State(storage): State<Arc<Storage>>,
    Path(account_id): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let now = now_unix();
    let request: AddDeviceRequest =
        serde_json::from_slice(&body).map_err(|e| ApiError::BadBody(e.to_string()))?;

    if request.registration.account.0 != account_id {
        return Err(ApiError::InvalidRegistration(
            "registration's account id does not match the URL".to_string(),
        ));
    }

    let account_public_key = decode_hex(&request.account_public_key_hex)
        .ok_or_else(|| ApiError::BadBody("account_public_key_hex is not valid hex".to_string()))?;

    verify_device_registration(&request.registration, &account_public_key)
        .map_err(|e| ApiError::InvalidRegistration(e.to_string()))?;

    if (request.registration.signed_at_unix - now).abs() > REGISTRATION_SKEW_SECS {
        return Err(ApiError::InvalidRegistration(
            "signed_at_unix is outside the allowed skew window".to_string(),
        ));
    }

    let device_id = request.registration.device.0.clone();
    let device_public_key_hex = encode_hex(&request.registration.device_public_key);
    let signature_hex = encode_hex(&request.registration.signature);

    tokio::task::spawn_blocking(move || {
        storage.register_device(
            &account_id,
            &request.account_public_key_hex,
            &device_id,
            &device_public_key_hex,
            request.registration.signed_at_unix,
            &signature_hex,
            now,
        )
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))??;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Serialize)]
struct ListDevicesResponse {
    account_public_key_hex: String,
    devices: Vec<DeviceRegistrationRow>,
}

async fn list_devices(
    State(storage): State<Arc<Storage>>,
    Path(account_id): Path<String>,
) -> Result<Json<ListDevicesResponse>, ApiError> {
    let result = tokio::task::spawn_blocking(move || storage.list_devices(&account_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

    let (account_public_key_hex, devices) = result.ok_or(ApiError::NotFound)?;
    Ok(Json(ListDevicesResponse {
        account_public_key_hex,
        devices,
    }))
}

async fn revoke_device(
    State(storage): State<Arc<Storage>>,
    Path((account_id, device_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let now = now_unix();
    let path = paths::revoke_path(&account_id, &device_id);
    auth::parse_and_verify(
        &header_pairs(&headers),
        "POST",
        &path,
        &account_id,
        &body,
        now,
    )?;

    let revoked =
        tokio::task::spawn_blocking(move || storage.revoke_device(&account_id, &device_id, now))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))??;

    if revoked {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

async fn put_recovery_blob(
    State(storage): State<Arc<Storage>>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    if body.len() > MAX_RECOVERY_BLOB_BYTES {
        return Err(ApiError::RecoveryBlobTooLarge);
    }

    let now = now_unix();
    let path = paths::recovery_blob_path(&account_id);
    auth::parse_and_verify(
        &header_pairs(&headers),
        "PUT",
        &path,
        &account_id,
        &body,
        now,
    )?;

    let blob = body.to_vec();
    tokio::task::spawn_blocking(move || storage.put_recovery_blob(&account_id, &blob, now))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

    Ok(StatusCode::NO_CONTENT)
}

async fn get_recovery_blob(
    State(storage): State<Arc<Storage>>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Result<Vec<u8>, ApiError> {
    let now = now_unix();
    let path = paths::recovery_blob_path(&account_id);
    // A GET carries no body; the signed envelope still covers method, path,
    // account id, and timestamp, over an empty body — same shape as any
    // other envelope-authenticated call.
    auth::parse_and_verify(&header_pairs(&headers), "GET", &path, &account_id, &[], now)?;

    let blob = tokio::task::spawn_blocking(move || storage.get_recovery_blob(&account_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

    blob.ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_blob_too_large_maps_to_413() {
        let err = ApiError::RecoveryBlobTooLarge;
        assert_eq!(err.into_response().status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
