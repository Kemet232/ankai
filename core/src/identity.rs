//! Account/device identity primitives.
//!
//! Shapes follow docs/adr/0004-e2ee-stack.md's MLS/OpenMLS decision: a
//! device is an MLS client with its own signature keypair and published
//! `KeyPackage`s. This is Phase 0/early-Phase-1 type scaffolding, not a
//! security-reviewed implementation — no real key material is generated or
//! handled here. See each type's doc comment for what's backed by real
//! `openmls` types vs. our own placeholder.

/// Opaque handle for a user's root identity key. Concrete key material and
/// algorithm choice land with ADR-0004.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AccountId(pub String);

/// Opaque handle for a single device's identity, subordinate to an
/// `AccountId`. A device key must be signed by the account's root key —
/// see docs/threat-model.md, "Identity & authentication".
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeviceId(pub String);

/// Stand-in for a device's real `openmls_basic_credential::SignatureKeyPair`
/// id. Not wired to that crate yet — real key generation happens against a
/// live `OpenMlsProvider`, and the resulting key belongs in the same
/// SQLCipher-backed keystore ADR-0004 specifies for MLS group state, not
/// inline here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SignatureKeyPlaceholder(pub Vec<u8>);

/// A device is an MLS client (ADR-0004: "a device is an MLS client with its
/// own signature keypair and published KeyPackages") — not a separate
/// identity concept layered on top of MLS.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub account: AccountId,
    /// Real `openmls` type: identity bytes only, no key material — safe to
    /// model now since constructing one performs no cryptographic operation.
    pub credential: openmls::credentials::BasicCredential,
    /// Placeholder — see `SignatureKeyPlaceholder`.
    pub signature_key: SignatureKeyPlaceholder,
}

/// A device's published `KeyPackage` — MLS's prekey equivalent. ADR-0004:
/// "each device publishes KeyPackages to a directory ahead of time" so a
/// sender can start a conversation with an offline recipient.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PublishedKeyPackage {
    pub owner: DeviceId,
    /// Real `openmls` type, left `None` for now: building an actual
    /// `KeyPackage` requires a live `OpenMlsProvider` + `Signer` +
    /// `Ciphersuite`, none of which this scaffolding phase wires up.
    pub key_package: Option<openmls::key_packages::KeyPackage>,
}
