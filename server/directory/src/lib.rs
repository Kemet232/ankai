//! Reference implementation of `docs/adr/0008-identity-discovery-service.md`
//! — a real, running instance of the "thin cloud" identity/discovery
//! service `core::directory`'s `DirectoryService` trait was built against,
//! plus a real network client for it.
//!
//! Two halves:
//! - [`app`] + [`db`] + [`auth`]: the server — an `axum` HTTP+JSON app
//!   (built by [`app::router`]) backed by SQLite ([`db::Storage`]),
//!   enforcing the ADR's ED25519-request-signing auth model on publish
//!   calls. `src/main.rs` boots this as a real standalone binary. Also
//!   includes [`username`]: human-readable, device-scoped usernames
//!   (claim/lookup) sitting alongside the ADR's original `KeyPackage`/
//!   `EndpointAddr` routes — see that module's doc comment for the scoping
//!   call and validation rule.
//! - [`client`]: [`client::HttpDirectoryClient`], a second, real
//!   implementation of `ankai_core::directory::DirectoryService` (the first
//!   being `core::directory::InMemoryDirectory`) that talks to a server
//!   built from [`app::router`] over genuine HTTP, proving the trait
//!   boundary works across a real process/network split, not just
//!   in-process.
//!
//! **What this crate is not**: a production deployment. Per the ADR's own
//! "Status: Proposed" and its "What would need to happen before this ADR
//! could be marked Accepted" list, this is a reference implementation and
//! integration-test subject — it is deliberately not wired into `client`
//! (see that crate's own scope; no ANKAI Node/relay infrastructure exists
//! to run this on anywhere). See the ADR for the auth model's known gaps
//! (TOFU pubkey pinning is a bridge, not a full PKI; `EndpointAddr` lookups
//! are unauthenticated, a real tension with `docs/threat-model.md` that the
//! ADR flags but does not resolve).

pub mod app;
pub mod auth;
pub mod client;
pub mod db;
pub mod paths;
pub mod username;

pub use client::HttpDirectoryClient;

/// Current Unix time in whole seconds. Centralized so every "now" the auth
/// window and staleness checks use comes from one place.
pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_secs() as i64
}
