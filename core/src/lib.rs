//! ankai-core: shared logic for all ANKAI clients (desktop now, mobile later).
//!
//! Deliberately UI-toolkit-agnostic — nothing here should depend on
//! docs/adr/0002 (native UI stack). Networking and crypto module internals
//! are placeholders pending docs/adr/0003 and docs/adr/0004.

pub mod account;
pub mod addons;
pub mod anime;
pub mod board;
pub mod catalog_cache;
pub mod communities;
pub mod db;
pub mod deeplink;
pub mod directory;
pub mod error;
pub mod forum_posts;
pub mod friends;
pub mod hangouts;
pub mod identity;
pub mod keychain;
pub mod lastfm;
pub mod letterboxd;
pub mod messaging;
pub mod mls_provider;
pub mod p2p;
pub mod playback_progress;
pub mod protocol;
pub mod stremio;
pub mod top8;
pub mod util;

pub use error::Error;
