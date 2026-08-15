//! Request-path builders shared by [`crate::app`] (the server's route
//! table) and [`crate::client`] (the HTTP client that calls it), so the two
//! sides can't drift — the path string is also part of what
//! [`crate::auth`]'s signature covers, so a mismatch here wouldn't just be
//! a 404, it would be an auth failure that's annoying to debug.

/// Path for publishing/looking up `device`'s `KeyPackage`s.
/// `axum`'s route table (see [`crate::app::router`]) uses the `{device_id}`
/// placeholder form of this same path.
pub fn key_packages_path(device_id: &str) -> String {
    format!("/v1/devices/{device_id}/key-packages")
}

/// Path for publishing/looking up `device`'s current `EndpointAddr`.
pub fn endpoint_addr_path(device_id: &str) -> String {
    format!("/v1/devices/{device_id}/endpoint-addr")
}

/// Path for `device` claiming/updating its username. See `crate::username`.
pub fn username_path(device_id: &str) -> String {
    format!("/v1/devices/{device_id}/username")
}

/// Path for looking up which `DeviceId` currently claims `username`.
pub fn username_lookup_path(username: &str) -> String {
    format!("/v1/usernames/{username}")
}

pub const KEY_PACKAGES_ROUTE: &str = "/v1/devices/{device_id}/key-packages";
pub const ENDPOINT_ADDR_ROUTE: &str = "/v1/devices/{device_id}/endpoint-addr";
pub const USERNAME_ROUTE: &str = "/v1/devices/{device_id}/username";
pub const USERNAME_LOOKUP_ROUTE: &str = "/v1/usernames/{username}";
