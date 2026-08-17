//! Request-path builders shared by [`crate::app`] (the server's route
//! table) and [`crate::client`] (the HTTP client that calls it), so the two
//! sides can't drift — same rationale as `server/directory/src/paths.rs`.
//! A path is also part of what [`crate::auth`]'s envelope signature covers
//! for the routes that use it, so a mismatch here would be an auth failure,
//! not just a 404.

/// Path for adding/refreshing a device registration, or listing an
/// account's current (non-revoked) device registrations.
pub fn devices_path(account_id: &str) -> String {
    format!("/v1/accounts/{account_id}/devices")
}

/// Path for revoking a specific device's registration.
pub fn revoke_path(account_id: &str, device_id: &str) -> String {
    format!("/v1/accounts/{account_id}/devices/{device_id}/revoke")
}

/// Path for storing/fetching an account's opaque, client-side-encrypted
/// recovery backup blob (per `docs/adr/0004-e2ee-stack.md`'s "Secondary,
/// opt-in path").
pub fn recovery_blob_path(account_id: &str) -> String {
    format!("/v1/accounts/{account_id}/recovery-blob")
}

pub const DEVICES_ROUTE: &str = "/v1/accounts/{account_id}/devices";
pub const REVOKE_ROUTE: &str = "/v1/accounts/{account_id}/devices/{device_id}/revoke";
pub const RECOVERY_BLOB_ROUTE: &str = "/v1/accounts/{account_id}/recovery-blob";
