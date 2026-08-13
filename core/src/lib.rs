//! ankai-core: shared logic for all ANKAI clients (desktop now, mobile later).
//!
//! Deliberately UI-toolkit-agnostic — nothing here should depend on
//! docs/adr/0002 (native UI stack). Networking and crypto module internals
//! are placeholders pending docs/adr/0003 and docs/adr/0004.

pub mod db;
pub mod error;
pub mod identity;
pub mod keychain;
pub mod mls_provider;
pub mod p2p;
pub mod protocol;

pub use error::Error;
