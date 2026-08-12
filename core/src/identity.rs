//! Account/device identity primitives.
//!
//! Placeholder pending docs/adr/0004-e2ee-stack.md, which will fix the
//! concrete key types (root identity key vs. per-device keys, signing vs.
//! key-agreement algorithms). Don't build on this module's shapes yet.

/// Opaque handle for a user's root identity key. Concrete key material and
/// algorithm choice land with ADR-0004.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AccountId(pub String);

/// Opaque handle for a single device's identity, subordinate to an
/// `AccountId`. A device key must be signed by the account's root key —
/// see docs/threat-model.md, "Identity & authentication".
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeviceId(pub String);
