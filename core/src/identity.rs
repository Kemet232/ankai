//! Account/device identity primitives, plus this installation's first-run
//! local identity creation.
//!
//! Shapes follow docs/adr/0004-e2ee-stack.md's MLS/OpenMLS decision: a
//! device is an MLS client with its own signature keypair and published
//! `KeyPackage`s. See each type's doc comment for what's backed by real
//! `openmls`/`openmls_basic_credential` types vs. our own placeholder.
//!
//! **Still a placeholder, deliberately:** `AccountId` here is just a random
//! opaque string generated on first run, *not* derived from a real account
//! root identity key. ADR-0004's multi-device "Registration" model (an
//! account root identity key that co-signs new devices) isn't implemented
//! anywhere in this codebase yet — that's real future work, not something
//! to fake by e.g. reusing the device signature key as if it were an
//! account key. There is also no server-side account service yet (ANKAI's
//! "thin cloud" identity/discovery side), so today "creating an account" can
//! only mean "generate and persist a local identity for this installation."

use openmls::credentials::BasicCredential;
use openmls::prelude::{OpenMlsProvider, SignatureScheme};
use openmls_basic_credential::SignatureKeyPair;

use crate::db::Db;
use crate::error::Error;
use crate::mls_provider::AnkaiMlsProvider;

/// Opaque handle for a user's root identity key. Concrete key material and
/// algorithm choice land with ADR-0004 — see this module's doc comment for
/// why this is still just a random opaque string today.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AccountId(pub String);

/// Opaque handle for a single device's identity, subordinate to an
/// `AccountId`. A device key must be signed by the account's root key —
/// see docs/threat-model.md, "Identity & authentication".
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeviceId(pub String);

/// The signature scheme ANKAI devices use for their MLS signature keypair.
/// Hardcoded rather than made configurable — nothing yet needs more than
/// one scheme, and ED25519 is the obvious default (small keys/signatures,
/// no parameter-choice footguns unlike the ECDSA variants).
pub const DEVICE_SIGNATURE_SCHEME: SignatureScheme = SignatureScheme::ED25519;

/// A device's real `openmls_basic_credential::SignatureKeyPair` public key.
/// The private key is *not* stored here — it lives in whichever
/// `AnkaiMlsProvider`'s storage it was generated into, keyed by this public
/// key under `DEVICE_SIGNATURE_SCHEME` (see
/// `openmls_basic_credential::SignatureKeyPair::store`/`read`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeviceSignatureKey(pub Vec<u8>);

/// A device is an MLS client (ADR-0004: "a device is an MLS client with its
/// own signature keypair and published KeyPackages") — not a separate
/// identity concept layered on top of MLS.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub account: AccountId,
    /// Real `openmls` type: identity bytes only, no key material — safe to
    /// model now since constructing one performs no cryptographic operation.
    pub credential: BasicCredential,
    pub signature_key: DeviceSignatureKey,
}

const ACCOUNT_ID_SETTING: &str = "account_id";
const DEVICE_ID_SETTING: &str = "device_id";
const DEVICE_SIGNATURE_PUBLIC_KEY_SETTING: &str = "device_signature_public_key";

/// Loads this installation's local account/device identity from `db` if one
/// was already created on a previous run, or generates and persists a
/// fresh one (a new device signature keypair, plus random opaque account/
/// device ids) if this is the first run.
///
/// Callers must call `provider.flush(db)` after this returns `Ok` with a
/// newly-created device (harmless, if slightly redundant, to always call
/// it) — this function persists account/device ids into `db`'s settings
/// table directly, but the device signature *key material* only exists in
/// `provider`'s in-memory MLS storage until flushed.
pub fn load_or_create_device(db: &Db, provider: &AnkaiMlsProvider) -> Result<Device, Error> {
    let existing = (
        db.get_setting(ACCOUNT_ID_SETTING)?,
        db.get_setting(DEVICE_ID_SETTING)?,
        db.get_setting(DEVICE_SIGNATURE_PUBLIC_KEY_SETTING)?,
    );

    if let (Some(account_id), Some(device_id), Some(public_key_hex)) = existing {
        let public_key = decode_hex(&public_key_hex)?;
        return Ok(Device {
            id: DeviceId(device_id.clone()),
            account: AccountId(account_id),
            credential: BasicCredential::new(device_id.into_bytes()),
            signature_key: DeviceSignatureKey(public_key),
        });
    }

    let device = create_device()?;
    device
        .signature_key_pair
        .store(provider.storage())
        .map_err(|e| Error::Identity(format!("failed to persist device signature key: {e}")))?;

    db.set_setting(ACCOUNT_ID_SETTING, &device.device.account.0)?;
    db.set_setting(DEVICE_ID_SETTING, &device.device.id.0)?;
    db.set_setting(
        DEVICE_SIGNATURE_PUBLIC_KEY_SETTING,
        &encode_hex(&device.device.signature_key.0),
    )?;

    Ok(device.device)
}

/// A freshly generated device, plus the real `SignatureKeyPair` it was
/// generated with — kept alongside the plain `Device` only long enough for
/// `load_or_create_device` to store it into an `AnkaiMlsProvider`.
struct NewDevice {
    device: Device,
    signature_key_pair: SignatureKeyPair,
}

fn create_device() -> Result<NewDevice, Error> {
    let account = AccountId(random_id());
    let device_id = DeviceId(random_id());

    let signature_key_pair = SignatureKeyPair::new(DEVICE_SIGNATURE_SCHEME)
        .map_err(|e| Error::Identity(format!("failed to generate device signature key: {e}")))?;

    let device = Device {
        credential: BasicCredential::new(device_id.0.clone().into_bytes()),
        signature_key: DeviceSignatureKey(signature_key_pair.to_public_vec()),
        id: device_id,
        account,
    };

    Ok(NewDevice {
        device,
        signature_key_pair,
    })
}

/// A fresh random 128-bit id, hex-encoded. Plenty of entropy to avoid
/// collisions for a single-installation identifier; these are opaque
/// handles, not secret key material, so 128 bits (vs. `keychain`'s 256-bit
/// passphrases) is a deliberately lighter budget.
fn random_id() -> String {
    let bytes: [u8; 16] = rand::random();
    encode_hex(&bytes)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, Error> {
    let bad_hex = || Error::Identity(format!("corrupt stored hex value: {hex:?}"));

    if !hex.len().is_multiple_of(2) {
        return Err(bad_hex());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad_hex()))
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_creates_a_device_with_a_verifiable_signature_key() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        let provider = AnkaiMlsProvider::load(&db).unwrap();

        let device = load_or_create_device(&db, &provider).unwrap();
        provider.flush(&db).unwrap();

        // The persisted public key must actually resolve to a real,
        // readable private key in the provider's MLS storage — not just an
        // opaque id nobody stored anything under.
        let found = SignatureKeyPair::read(
            provider.storage(),
            &device.signature_key.0,
            DEVICE_SIGNATURE_SCHEME,
        );
        assert!(
            found.is_some(),
            "device's signature key pair should be stored in the MLS provider"
        );
    }

    #[test]
    fn second_run_reloads_the_same_device_instead_of_creating_another() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();

        let first = {
            let provider = AnkaiMlsProvider::load(&db).unwrap();
            let device = load_or_create_device(&db, &provider).unwrap();
            provider.flush(&db).unwrap();
            device
        };

        let second = {
            let provider = AnkaiMlsProvider::load(&db).unwrap();
            load_or_create_device(&db, &provider).unwrap()
        };

        assert_eq!(first.id, second.id);
        assert_eq!(first.account, second.account);
        assert_eq!(first.signature_key, second.signature_key);
    }

    #[test]
    fn hex_round_trips_including_empty_and_odd_length_is_rejected() {
        assert_eq!(decode_hex(&encode_hex(&[])).unwrap(), Vec::<u8>::new());
        assert_eq!(
            decode_hex(&encode_hex(&[0, 1, 254, 255])).unwrap(),
            vec![0, 1, 254, 255]
        );
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
    }
}
