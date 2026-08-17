//! Reference implementation of `docs/adr/0009-account-system.md` — a real,
//! running instance of the account-root-key model's server side, plus a
//! real network client for it.
//!
//! Two halves, mirroring `server/directory`'s own shape:
//! - [`app`] + [`db`] + [`auth`]: the server — an `axum` HTTP+JSON app
//!   (built by [`app::router`]) backed by SQLite ([`db::Storage`]).
//!   `src/main.rs` boots this as a real standalone binary.
//! - [`client`]: [`client::AccountsClient`], a real client that talks to a
//!   server built from [`app::router`] over genuine HTTP, signing requests
//!   with a real `ankai_core::account::AccountRootKeyPair`.
//!
//! **What this crate is not**: a production deployment, and not (yet) a
//! decision the codebase has committed to — see the ADR's own
//! "Status: Proposed". Per that ADR's own scope, this crate deliberately
//! does **not** implement the multi-device MLS-group-membership fan-out
//! (adding a newly authorized device to every existing conversation,
//! removing a revoked one from them) — that's real, unbuilt future work,
//! not faked here. It is also not wired into `client` (the desktop app) —
//! `core::identity::load_or_create_device` still produces the same honest
//! placeholder `AccountId` it always has; nothing in this crate changes
//! that behavior.

pub mod app;
pub mod auth;
pub mod client;
pub mod db;
pub mod paths;

pub use client::AccountsClient;

/// Current Unix time in whole seconds. Centralized so every "now" the auth
/// window and staleness checks use comes from one place, matching
/// `server/directory`'s equivalent helper.
pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_secs() as i64
}
