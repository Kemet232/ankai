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
