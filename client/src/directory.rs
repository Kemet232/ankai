//! Optional, opt-in glue between `client` and a real running
//! `ankai-directory-server` (per `docs/adr/0008-identity-discovery-service.md`,
//! **Status: Proposed**, not Accepted). See `main.rs` for how this module is
//! only ever exercised if a human sets [`DIRECTORY_URL_ENV_VAR`], and
//! `ankai_core::messaging`'s module doc comment ("Optional: sourcing a
//! PeerInvite from a directory server") for the full picture, including
//! every gap this integration inherits unchanged from the ADR (TOFU pubkey
//! pinning, unauthenticated `EndpointAddr` lookups, `KeyPackage`
//! consumption-on-lookup, no re-publish-on-rotation, no revocation).
//!
//! This module deliberately does not touch MLS/group-setup logic at all —
//! [`lookup_peer_invite`] only turns a directory lookup into the exact same
//! `ankai_core::messaging::PeerInvite` shape that module already accepts
//! from a manually pasted blob, so `main.rs` can hand the result straight
//! to the same `peer-address-input` field (and therefore the same
//! `on_send_message` path) a manual paste already uses. Nothing about how a
//! `PeerInvite` is consumed downstream is forked to support this.
//!
//! **Usernames** ([`claim_own_username`], [`lookup_peer_invite_by_username`])
//! slot into the exact same opt-in shape: both only ever run if
//! `ANKAI_DIRECTORY_URL` was set (same `directory_client` in `main.rs`), and
//! [`lookup_peer_invite_by_username`] does nothing but resolve a username to
//! a `DeviceId` and then hand off to [`lookup_peer_invite`] — the *same*
//! function the Device-ID lookup path already uses, not a parallel one. See
//! `ankai_directory_server::username`'s module doc comment for the
//! device-scoped limitation and validation rule usernames are subject to.

use ankai_core::directory::DirectoryService;
use ankai_core::identity::DeviceId;
use ankai_core::messaging::PeerInvite;
use ankai_directory_server::HttpDirectoryClient;

/// Name of the env var that opts a running `client` into the experimental
/// directory-lookup/publish path. Unset (the default): `main.rs` never
/// constructs an `HttpDirectoryClient` and never makes a network call to
/// any directory server — behavior is identical to how this client behaved
/// before this integration existed. Deliberately no default URL: this
/// stays an explicit human opt-in, matching ADR-0008's `Proposed` status.
pub const DIRECTORY_URL_ENV_VAR: &str = "ANKAI_DIRECTORY_URL";

/// Reads [`DIRECTORY_URL_ENV_VAR`] from the environment, trimmed. `None` if
/// unset or empty — both treated as "directory integration is off."
pub fn configured_directory_url() -> Option<String> {
    std::env::var(DIRECTORY_URL_ENV_VAR)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Looks up `peer`'s currently published `KeyPackage` and `EndpointAddr`
/// from `client`'s directory server and, if both are present, assembles
/// them into the same `PeerInvite` shape a manually pasted invite blob
/// already produces. `Ok(None)` means the lookup succeeded but `peer`
/// hasn't published enough to connect to yet (no `KeyPackage`, or no
/// fresh-enough `EndpointAddr`) — not an error, matching
/// `DirectoryService`'s own "not found is not an error" convention.
///
/// Note (see ADR-0008's "Staleness / expiry / consumption" section): a
/// successful `KeyPackage` lookup *removes* it from the server — looking
/// the same peer up again immediately after will return `Ok(None)` unless
/// they've republished (or had more than one `KeyPackage` published, in
/// which case this call still takes all of them, not just the one it
/// uses — a real, documented gap, not something papered over here).
pub async fn lookup_peer_invite(
    client: &HttpDirectoryClient,
    peer: &DeviceId,
) -> Result<Option<PeerInvite>, ankai_core::Error> {
    let mut key_packages = client.key_packages(peer).await?;
    if key_packages.is_empty() {
        return Ok(None);
    }
    let key_package = key_packages.remove(0);

    let Some(addr) = client.endpoint_addr(peer).await? else {
        return Ok(None);
    };

    Ok(Some(PeerInvite { addr, key_package }))
}

/// Claims (or updates) this device's own username against the directory
/// server. A thin pass-through to `HttpDirectoryClient::claim_username`,
/// kept here so every directory-network call `client` makes routes through
/// this module rather than `main.rs` calling `ankai_directory_server`
/// directly. See `ankai_directory_server::username`'s module doc comment
/// for the validation rule and device-scoped-not-account-scoped limitation.
pub async fn claim_own_username(
    client: &HttpDirectoryClient,
    device: &DeviceId,
    username: &str,
) -> Result<(), ankai_core::Error> {
    client.claim_username(device, username).await
}

/// Looks up `username` against the directory server and, if it resolves to
/// a `DeviceId`, hands off to [`lookup_peer_invite`] — the exact same
/// function the "look up by Device ID" path already uses — so a
/// username-sourced peer flows through the identical
/// lookup-by-DeviceId -> `PeerInvite` path, not a forked one. `Ok(None)`
/// means either nobody has claimed `username`, or they have but haven't
/// published enough to connect to yet (same "not found is not an error"
/// convention as `lookup_peer_invite`).
pub async fn lookup_peer_invite_by_username(
    client: &HttpDirectoryClient,
    username: &str,
) -> Result<Option<PeerInvite>, ankai_core::Error> {
    let Some(device_id) = client.lookup_username(username).await? else {
        return Ok(None);
    };
    lookup_peer_invite(client, &device_id).await
}
