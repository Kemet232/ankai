//! Account-root identity: the cryptographic backbone
//! `docs/adr/0009-account-system.md` proposes to replace `identity::
//! AccountId`'s current honest placeholder (a random opaque string, chosen
//! independently of any key) with a real, self-certifying identity derived
//! from an actual keypair.
//!
//! **Not wired into `identity::load_or_create_device` or `client` yet** —
//! matching `docs/adr/0008-identity-discovery-service.md`'s own precedent
//! (`core::directory`'s trait + `InMemoryDirectory` existed for a full
//! session before that ADR was even written, and a further session before
//! anything wired it into `client`), this module is real, tested
//! cryptographic machinery built and proven in isolation, backing a
//! `Proposed` ADR — not a shipped feature. See the ADR for the full
//! rationale and the open questions that need a human decision before this
//! could change.
//!
//! ## What an "account" is here
//!
//! An ANKAI account **is** an Ed25519 keypair (the "account root key"), not
//! a username/password or an OAuth login. `identity::AccountId` is now
//! derivable as a real, content-addressed hash of the account root public
//! key — see [`derive_account_id`] — rather than an arbitrary string chosen
//! independently of any key. This is "self-certifying" identity (the same
//! idea behind Tor onion-service addresses and IPFS PeerIDs): anyone who
//! has an account's public key can verify for themselves, offline, that a
//! claimed `AccountId` genuinely corresponds to it — no directory/CA needs
//! to vouch for the binding, and (see `docs/adr/0009-account-system.md`'s
//! "Auth" section) there is no first-write-wins TOFU race for the account
//! id itself, unlike `docs/adr/0008-identity-discovery-service.md`'s
//! still-unsolved `DeviceId` binding gap.
//!
//! ## Recovery phrase = account root key, not a separate wrapping secret
//!
//! [`AccountRootKeyPair::generate`] returns both a real keypair and a
//! human-writable 24-word BIP39 mnemonic ([`RecoveryPhrase`]) that encodes
//! the *exact same* 32 bytes of entropy used as the Ed25519 signing-key
//! seed (`ed25519_dalek::SigningKey::from_bytes`) — not a mnemonic for a
//! separately generated wrapping key. [`AccountRootKeyPair::from_mnemonic`]
//! reverses this: typing the phrase back in on a brand-new device
//! regenerates the identical keypair, with no server involved at all.
//!
//! This is a deliberate simplification worth flagging explicitly for
//! `docs/adr/0004-e2ee-stack.md`'s required security audit before shipping:
//! it is **not** the standard BIP32/SLIP-0010 hierarchical-derivation path
//! real cryptocurrency wallets use (HMAC-SHA512-based, produces a chain
//! code so many child keys can be derived from one seed). ANKAI only ever
//! needs *one* non-hierarchical account key, so this module uses BIP39
//! purely for its mnemonic encode/decode of raw entropy
//! (`bip39::Mnemonic::from_entropy`/`.to_entropy()`), not its seed-
//! derivation machinery (`Mnemonic::to_seed`, PBKDF2 stretching). This
//! composition (BIP39 entropy used directly as an Ed25519 seed) is a
//! well-defined operation, but the exact choice of skipping BIP32/SLIP-0010
//! is this module's own, and should be one of the things the audit
//! explicitly signs off on, not assumed safe by analogy to how wallets do
//! it (they don't do exactly this).
//!
//! `docs/adr/0004-e2ee-stack.md` already named a "24-word BIP39-style
//! mnemonic" as the wrapping secret for its client-side-encrypted recovery
//! backup. This module's design lets the *same* phrase serve both roles —
//! see `docs/adr/0009-account-system.md`'s "Recovery" section for why doing
//! that safely needs HKDF-style domain separation (distinct derived
//! subkeys for "sign as this account" vs. "unwrap my backup blob," not the
//! same 32 raw bytes reused for both purposes directly) if/when the
//! backup-blob encryption itself gets built — **not implemented in this
//! module**, which deliberately stops at the account root keypair and
//! device-registration signing. Backup-blob format/encryption stays
//! `docs/adr/0004-e2ee-stack.md`'s job.
//!
//! ## Device registration
//!
//! [`DeviceRegistration`] is the artifact this ADR proposes as the real
//! "device keys must be signed by the account's root identity key" binding
//! `docs/threat-model.md`'s "Identity & authentication" section already
//! calls for, and that `docs/adr/0008-identity-discovery-service.md`
//! explicitly named as the thing its TOFU pubkey pinning is a bridge
//! *until*. See [`sign_device_registration`]/[`verify_device_registration`].
//! Building the actual multi-device MLS-group-membership fan-out (adding a
//! newly registered device to every existing conversation, removing a
//! revoked one) is **not** implemented here — see the ADR's "Multi-device
//! model" section for the honest design-vs-implementation split.

use ed25519_dalek::{Signature, SigningKey, VerifyingKey};

use crate::error::Error;
use crate::identity::{AccountId, DeviceId};
use crate::util::encode_hex;

/// Bytes of raw entropy backing an account root key. 32 bytes (256 bits)
/// encodes to a 24-word BIP39 mnemonic, matching
/// `docs/adr/0004-e2ee-stack.md`'s "24-word BIP39-style mnemonic" exactly.
const ENTROPY_BYTES: usize = 32;

/// How far a [`DeviceRegistration`]'s `signed_at_unix` may drift from a
/// verifier's own clock before it's treated as stale — same window and
/// same rationale as `docs/adr/0008-identity-discovery-service.md`'s
/// request-signing timestamp check (bounds how long a captured, validly
/// signed registration could be replayed against an idempotent-safe
/// endpoint; see `server/accounts`).
pub const REGISTRATION_SKEW_SECS: i64 = 300;

/// A real Ed25519 keypair that *is* an ANKAI account, per this module's doc
/// comment. Never persisted in plaintext anywhere by this module itself —
/// callers decide storage, matching `core::identity`'s existing device-key
/// pattern (this module only builds/tests the cryptography).
pub struct AccountRootKeyPair {
    signing_key: SigningKey,
}

impl std::fmt::Debug for AccountRootKeyPair {
    /// Deliberately does not print any key material, even the public key's
    /// raw bytes — only the derived, non-secret `AccountId`. An
    /// `AccountRootKeyPair` holds real private key material directly
    /// (unlike `identity::Device`, whose private key lives only in MLS
    /// provider storage), so an accidental `{:?}` in a log line must not be
    /// able to leak it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountRootKeyPair")
            .field("account_id", &self.identity().id)
            .finish_non_exhaustive()
    }
}

/// A human-writable 24-word recovery phrase encoding the exact entropy an
/// [`AccountRootKeyPair`] was generated from. See the module doc comment
/// for why this is deliberately the *same* secret as the account root key,
/// not a separately generated wrapping key — treat it with the same care
/// as a private key, because it effectively is one.
#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryPhrase(pub String);

impl std::fmt::Debug for RecoveryPhrase {
    /// Redacted deliberately — see the struct doc comment. Printing this
    /// value is equivalent to printing the account's private key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RecoveryPhrase")
            .field(&"<redacted>")
            .finish()
    }
}

/// The public half of an account root key, plus the [`AccountId`] derived
/// from it — everything safe to hand to a server or another peer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AccountIdentity {
    pub id: AccountId,
    pub public_key: Vec<u8>,
}

/// Derives a self-certifying [`AccountId`] from a raw Ed25519 public key:
/// the first 16 bytes of its BLAKE3 hash, hex-encoded — 32 hex characters,
/// matching `util::random_id`'s existing id length/shape so this doesn't
/// look out of place next to today's placeholder ids, but *derived* rather
/// than chosen. Deterministic and content-addressed: two callers who both
/// hold the same public key always compute the same `AccountId`, and (per
/// the birthday bound on a 128-bit hash truncation) there is no realistic
/// collision risk at any scale this product will reach.
pub fn derive_account_id(public_key: &[u8]) -> AccountId {
    let hash = blake3::hash(public_key);
    AccountId(encode_hex(&hash.as_bytes()[..16]))
}

impl AccountRootKeyPair {
    /// Generates a fresh account root key from real OS randomness, plus the
    /// recovery phrase that encodes the exact same entropy. See the module
    /// doc comment for why these are the same secret, not two.
    pub fn generate() -> (Self, RecoveryPhrase) {
        let entropy: [u8; ENTROPY_BYTES] = rand::random();
        let phrase = entropy_to_phrase(&entropy);
        (
            Self {
                signing_key: SigningKey::from_bytes(&entropy),
            },
            phrase,
        )
    }

    /// Reconstructs the exact same account root key from a previously
    /// generated [`RecoveryPhrase`]. Fails on a malformed phrase (wrong
    /// word count, a word not in the BIP39 wordlist, or a bad checksum) —
    /// never silently produces a different/garbage key from bad input.
    pub fn from_mnemonic(phrase: &RecoveryPhrase) -> Result<Self, Error> {
        let entropy = phrase_to_entropy(phrase)?;
        Ok(Self {
            signing_key: SigningKey::from_bytes(&entropy),
        })
    }

    /// This account's public identity — safe to publish/hand to a server.
    pub fn identity(&self) -> AccountIdentity {
        let public_key = self.signing_key.verifying_key().to_bytes().to_vec();
        let id = derive_account_id(&public_key);
        AccountIdentity { id, public_key }
    }

    /// Signs arbitrary `message` bytes with this account's root key.
    /// [`sign_device_registration`] is built on top of this for its more
    /// specific canonical message; callers building their own signed
    /// envelopes (e.g. `server/accounts`'s `AccountsClient`, for requests
    /// that don't carry a naturally self-verifying signed object — see
    /// that crate's `auth` module doc comment) call this directly.
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        use ed25519_dalek::Signer;
        self.signing_key.sign(message).to_bytes().to_vec()
    }
}

fn entropy_to_phrase(entropy: &[u8; ENTROPY_BYTES]) -> RecoveryPhrase {
    let mnemonic = bip39::Mnemonic::from_entropy(entropy)
        .expect("32 bytes is always valid BIP39 entropy for a 24-word mnemonic");
    RecoveryPhrase(mnemonic.to_string())
}

fn phrase_to_entropy(phrase: &RecoveryPhrase) -> Result<[u8; ENTROPY_BYTES], Error> {
    let mnemonic: bip39::Mnemonic = phrase
        .0
        .parse()
        .map_err(|e| Error::Account(format!("invalid recovery phrase: {e}")))?;
    let entropy = mnemonic.to_entropy();
    entropy.try_into().map_err(|bad: Vec<u8>| {
        Error::Account(format!(
            "recovery phrase encoded {} bytes of entropy, expected a 24-word \
             phrase encoding {ENTROPY_BYTES}",
            bad.len()
        ))
    })
}

/// A device's cryptographic authorization to act as part of an account,
/// signed by that account's root key. This is the real artifact
/// `docs/threat-model.md` describes ("device keys must be signed by the
/// account's root identity key") and
/// `docs/adr/0008-identity-discovery-service.md` names as the thing that
/// should eventually replace its TOFU pubkey-pinning bridge.
///
/// Deliberately self-describing/independently verifiable: anyone who later
/// obtains a `DeviceRegistration` (e.g. from `server/accounts`'s open
/// device-list read) and separately obtains the account's public key can
/// call [`verify_device_registration`] themselves — no need to trust
/// whichever server happened to hand it to them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceRegistration {
    pub account: AccountId,
    pub device: DeviceId,
    /// The device's own Ed25519 signature public key — the same bytes
    /// `identity::DeviceSignatureKey` already stores; carried here so a
    /// verifier doesn't need a separate lookup to check this signature.
    pub device_public_key: Vec<u8>,
    pub signed_at_unix: i64,
    /// The account root key's signature over
    /// [`registration_signing_message`] for the fields above.
    pub signature: Vec<u8>,
}

/// The canonical bytes an account root key signs to authorize a device —
/// the same "newline-joined canonical fields" shape
/// `docs/adr/0008-identity-discovery-service.md`'s request-signing scheme
/// already uses, reused deliberately instead of inventing a second
/// canonical-message convention in the same codebase.
pub fn registration_signing_message(
    account: &AccountId,
    device: &DeviceId,
    device_public_key: &[u8],
    signed_at_unix: i64,
) -> Vec<u8> {
    format!(
        "{}\n{}\n{}\n{}",
        account.0,
        device.0,
        encode_hex(device_public_key),
        signed_at_unix
    )
    .into_bytes()
}

/// Signs a [`DeviceRegistration`] binding `device`/`device_public_key` to
/// `account_key`'s account — the account root key co-signing a new device,
/// per `docs/adr/0004-e2ee-stack.md`'s "Registration" model.
pub fn sign_device_registration(
    account_key: &AccountRootKeyPair,
    device: DeviceId,
    device_public_key: Vec<u8>,
    signed_at_unix: i64,
) -> DeviceRegistration {
    let account = account_key.identity().id;
    let message =
        registration_signing_message(&account, &device, &device_public_key, signed_at_unix);
    let signature = account_key.sign(&message);
    DeviceRegistration {
        account,
        device,
        device_public_key,
        signed_at_unix,
        signature,
    }
}

/// Verifies `registration` was really signed by the account root key whose
/// public key is `account_public_key`, **and** that `registration.account`
/// is genuinely the self-certifying [`AccountId`] derived from that same
/// public key (catching an attempt to present a validly-signed
/// registration — e.g. one eavesdropped from a public device-list read —
/// as if it applied to a different, mismatched account id).
///
/// Does **not** check `signed_at_unix` freshness — that's a policy decision
/// for the caller (a verifying peer reconstructing MLS group membership
/// offline might reasonably accept an old registration; `server/accounts`'s
/// write path enforces [`REGISTRATION_SKEW_SECS`] itself, since freshness
/// only matters for bounding replay of a *write*, not for reading history).
pub fn verify_device_registration(
    registration: &DeviceRegistration,
    account_public_key: &[u8],
) -> Result<(), Error> {
    let expected_id = derive_account_id(account_public_key);
    if registration.account != expected_id {
        return Err(Error::Account(
            "device registration's account id does not match the provided account public key"
                .to_string(),
        ));
    }

    let verifying_key = parse_verifying_key(account_public_key)?;
    let signature = parse_signature(&registration.signature)?;
    let message = registration_signing_message(
        &registration.account,
        &registration.device,
        &registration.device_public_key,
        registration.signed_at_unix,
    );

    verifying_key
        .verify_strict(&message, &signature)
        .map_err(|_| Error::Account("device registration signature does not verify".to_string()))
}

fn parse_verifying_key(bytes: &[u8]) -> Result<VerifyingKey, Error> {
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::Account("account public key must be 32 bytes".to_string()))?;
    VerifyingKey::from_bytes(&arr)
        .map_err(|e| Error::Account(format!("invalid account public key: {e}")))
}

fn parse_signature(bytes: &[u8]) -> Result<Signature, Error> {
    let arr: [u8; 64] = bytes
        .try_into()
        .map_err(|_| Error::Account("signature must be 64 bytes".to_string()))?;
    Ok(Signature::from_bytes(&arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_id_is_deterministic_and_content_derived() {
        let (key, _phrase) = AccountRootKeyPair::generate();
        let identity = key.identity();

        // Re-deriving from the same public key bytes always gives the same
        // id — this is what "self-certifying" means.
        assert_eq!(derive_account_id(&identity.public_key), identity.id);

        // A different key's public bytes derive a different id.
        let (other_key, _) = AccountRootKeyPair::generate();
        assert_ne!(
            derive_account_id(&other_key.identity().public_key),
            identity.id
        );
    }

    #[test]
    fn recovery_phrase_round_trips_to_the_identical_key() {
        let (original, phrase) = AccountRootKeyPair::generate();
        let original_identity = original.identity();

        let restored =
            AccountRootKeyPair::from_mnemonic(&phrase).expect("phrase should parse back");
        let restored_identity = restored.identity();

        assert_eq!(
            original_identity, restored_identity,
            "restoring from the recovery phrase must reconstruct the exact same account"
        );
    }

    #[test]
    fn phrase_is_a_real_24_word_bip39_mnemonic() {
        let (_key, phrase) = AccountRootKeyPair::generate();
        let words: Vec<&str> = phrase.0.split_whitespace().collect();
        assert_eq!(
            words.len(),
            24,
            "should be a 24-word phrase, matching ADR-0004"
        );
    }

    #[test]
    fn garbled_recovery_phrase_is_rejected_not_silently_accepted() {
        let bad = RecoveryPhrase("not a real bip39 mnemonic at all".to_string());
        assert!(AccountRootKeyPair::from_mnemonic(&bad).is_err());

        let wrong_word_count = RecoveryPhrase("abandon abandon abandon".to_string());
        assert!(AccountRootKeyPair::from_mnemonic(&wrong_word_count).is_err());
    }

    #[test]
    fn recovery_phrase_debug_output_is_redacted() {
        let phrase = RecoveryPhrase("some words that must never appear in a log".to_string());
        let debug = format!("{phrase:?}");
        assert!(!debug.contains("some words"));
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn account_root_key_pair_debug_output_does_not_leak_key_material() {
        let (key, _phrase) = AccountRootKeyPair::generate();
        let identity = key.identity();
        let debug = format!("{key:?}");
        assert!(
            !debug.contains(&encode_hex(&identity.public_key)),
            "Debug output must not print raw key bytes"
        );
    }

    #[test]
    fn device_registration_round_trips_and_verifies() {
        let (account_key, _phrase) = AccountRootKeyPair::generate();
        let device = DeviceId("device-1".to_string());
        let device_public_key = vec![7u8; 32];

        let registration = sign_device_registration(
            &account_key,
            device.clone(),
            device_public_key.clone(),
            1000,
        );

        assert_eq!(registration.account, account_key.identity().id);
        assert_eq!(registration.device, device);

        verify_device_registration(&registration, &account_key.identity().public_key)
            .expect("a genuine registration should verify");
    }

    #[test]
    fn tampered_device_public_key_fails_verification() {
        let (account_key, _phrase) = AccountRootKeyPair::generate();
        let mut registration = sign_device_registration(
            &account_key,
            DeviceId("device-1".to_string()),
            vec![7u8; 32],
            1000,
        );

        // An attacker who intercepts a registration and swaps in their own
        // device's public key, hoping the signature still "counts."
        registration.device_public_key = vec![9u8; 32];

        let result = verify_device_registration(&registration, &account_key.identity().public_key);
        assert!(
            result.is_err(),
            "tampering with the signed payload must invalidate it"
        );
    }

    #[test]
    fn registration_signed_by_a_different_account_is_rejected() {
        let (real_account, _phrase) = AccountRootKeyPair::generate();
        let (impostor_account, _phrase2) = AccountRootKeyPair::generate();

        let registration = sign_device_registration(
            &impostor_account,
            DeviceId("device-1".to_string()),
            vec![7u8; 32],
            1000,
        );

        // Verifying against the *real* account's public key must fail —
        // this registration was signed by a different key entirely.
        let result = verify_device_registration(&registration, &real_account.identity().public_key);
        assert!(result.is_err());
    }

    #[test]
    fn eavesdropped_registration_cannot_be_claimed_for_a_different_account_id() {
        // Simulates an attacker who observed a real, validly signed
        // DeviceRegistration (e.g. via a public device-list read) and tries
        // to present it as if it applied to a *different* account id than
        // the one it was actually signed for.
        let (real_account, _phrase) = AccountRootKeyPair::generate();
        let mut registration = sign_device_registration(
            &real_account,
            DeviceId("device-1".to_string()),
            vec![7u8; 32],
            1000,
        );
        registration.account = AccountId("some-other-account-id".to_string());

        let result = verify_device_registration(&registration, &real_account.identity().public_key);
        assert!(
            result.is_err(),
            "a registration whose claimed account id doesn't match the key's real id must fail"
        );
    }
}
