//! HTTP-envelope request signing for the routes that don't have a
//! naturally self-verifying signed object to authenticate against.
//!
//! `POST /v1/accounts/{account_id}/devices` (see [`crate::app`]) does
//! **not** use this module — it's authorized entirely by the
//! `ankai_core::account::DeviceRegistration` body's own embedded
//! account-root-key signature (checked via
//! `ankai_core::account::verify_device_registration`), which is both
//! necessary (the registration must be independently verifiable by anyone
//! who later fetches it from `GET .../devices`, not just by this server at
//! write time) and sufficient (it already proves the account root key
//! authorized exactly this device binding, with its own embedded
//! timestamp) — layering a *second*, redundant envelope signature on top
//! would just be two signatures proving the same fact.
//!
//! The revoke and recovery-blob routes have no such naturally-signed
//! object — "revoke device X" and "store these bytes" aren't independently
//! meaningful artifacts anyone else needs to verify later — so those use
//! this generic envelope instead: the same canonical-message shape
//! `docs/adr/0008-identity-discovery-service.md`'s device-key request
//! signing already established (method/path/id/timestamp/body), with one
//! real improvement this ADR calls out explicitly: the claimed public key
//! is checked against the path's `account_id` by **content-derivation**
//! (`ankai_core::account::derive_account_id`), not TOFU pubkey pinning —
//! there is no first-write-wins race here, because the id *is* a hash of
//! the key, so there's nothing to "pin" and no storage lookup needed to
//! authenticate a request at all.

use ed25519_dalek::{Signature, VerifyingKey};

use ankai_core::account::derive_account_id;
use ankai_core::util::decode_hex;

/// How far a request's `x-ankai-timestamp` may drift from the server's own
/// clock (either direction) before it's rejected as stale/replayed. Same
/// window as `docs/adr/0008-identity-discovery-service.md`'s equivalent
/// check and `ankai_core::account::REGISTRATION_SKEW_SECS`.
pub const TIMESTAMP_SKEW_SECS: i64 = 300;

pub const ACCOUNT_PUBKEY_HEADER: &str = "x-ankai-account-pubkey";
pub const TIMESTAMP_HEADER: &str = "x-ankai-timestamp";
pub const SIGNATURE_HEADER: &str = "x-ankai-signature";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("missing or malformed '{0}' header")]
    MalformedHeader(&'static str),
    #[error("request timestamp is outside the allowed skew window")]
    StaleTimestamp,
    #[error("signature does not verify against the claimed account public key")]
    BadSignature,
    #[error("claimed account public key does not hash to the account id in the URL")]
    AccountIdMismatch,
}

/// The canonical byte string an account root key signs (and a verifier
/// re-derives and checks against) for a given request. Covers the method,
/// path, claimed account id, timestamp, and the exact request body.
pub fn signing_message(
    method: &str,
    path: &str,
    account_id: &str,
    timestamp: i64,
    body: &[u8],
) -> Vec<u8> {
    let mut msg = format!("{method}\n{path}\n{account_id}\n{timestamp}\n").into_bytes();
    msg.extend_from_slice(body);
    msg
}

/// A request's verified account root public key, once
/// [`parse_and_verify`] has confirmed both the self-certification and the
/// signature.
#[derive(Debug)]
pub struct VerifiedRequest {
    pub account_public_key: [u8; 32],
}

/// Parses the envelope headers out of `headers`, checks the timestamp is
/// within [`TIMESTAMP_SKEW_SECS`] of `now`, checks the claimed public key
/// actually hashes to `account_id` (self-certification — see the module
/// doc comment), and verifies the signature over
/// [`signing_message`]`(method, path, account_id, timestamp, body)`.
pub fn parse_and_verify(
    headers: &[(&str, &str)],
    method: &str,
    path: &str,
    account_id: &str,
    body: &[u8],
    now: i64,
) -> Result<VerifiedRequest, AuthError> {
    let header = |name: &'static str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
            .ok_or(AuthError::MalformedHeader(name))
    };

    let pubkey_hex = header(ACCOUNT_PUBKEY_HEADER)?;
    let timestamp_str = header(TIMESTAMP_HEADER)?;
    let signature_hex = header(SIGNATURE_HEADER)?;

    let pubkey_bytes =
        decode_hex(pubkey_hex).ok_or(AuthError::MalformedHeader(ACCOUNT_PUBKEY_HEADER))?;
    let pubkey: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| AuthError::MalformedHeader(ACCOUNT_PUBKEY_HEADER))?;

    if derive_account_id(&pubkey).0 != account_id {
        return Err(AuthError::AccountIdMismatch);
    }

    let timestamp: i64 = timestamp_str
        .parse()
        .map_err(|_| AuthError::MalformedHeader(TIMESTAMP_HEADER))?;
    if (timestamp - now).abs() > TIMESTAMP_SKEW_SECS {
        return Err(AuthError::StaleTimestamp);
    }

    let signature_bytes =
        decode_hex(signature_hex).ok_or(AuthError::MalformedHeader(SIGNATURE_HEADER))?;
    let signature_bytes: [u8; 64] = signature_bytes
        .try_into()
        .map_err(|_| AuthError::MalformedHeader(SIGNATURE_HEADER))?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey)
        .map_err(|_| AuthError::MalformedHeader(ACCOUNT_PUBKEY_HEADER))?;
    let signature = Signature::from_bytes(&signature_bytes);

    let message = signing_message(method, path, account_id, timestamp, body);
    verifying_key
        .verify_strict(&message, &signature)
        .map_err(|_| AuthError::BadSignature)?;

    Ok(VerifiedRequest {
        account_public_key: pubkey,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankai_core::util::encode_hex;
    use ed25519_dalek::{Signer, SigningKey};

    fn sign_envelope(
        signing_key: &SigningKey,
        method: &str,
        path: &str,
        account_id: &str,
        timestamp: i64,
        body: &[u8],
    ) -> String {
        let msg = signing_message(method, path, account_id, timestamp, body);
        encode_hex(&signing_key.sign(&msg).to_bytes())
    }

    #[test]
    fn valid_envelope_verifies() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();
        let account_id = derive_account_id(&pubkey).0;
        let now = 1_000_000;
        let path = format!("/v1/accounts/{account_id}/devices/d1/revoke");
        let sig_hex = sign_envelope(&signing_key, "POST", &path, &account_id, now, b"body");

        let pubkey_hex = encode_hex(&pubkey);
        let headers = [
            (ACCOUNT_PUBKEY_HEADER, pubkey_hex.as_str()),
            (TIMESTAMP_HEADER, "1000000"),
            (SIGNATURE_HEADER, sig_hex.as_str()),
        ];

        let parsed = parse_and_verify(&headers, "POST", &path, &account_id, b"body", now).unwrap();
        assert_eq!(parsed.account_public_key, pubkey);
    }

    #[test]
    fn tampered_body_fails_verification() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();
        let account_id = derive_account_id(&pubkey).0;
        let now = 1_000_000;
        let path = format!("/v1/accounts/{account_id}/devices/d1/revoke");
        let sig_hex = sign_envelope(&signing_key, "POST", &path, &account_id, now, b"body");

        let pubkey_hex = encode_hex(&pubkey);
        let headers = [
            (ACCOUNT_PUBKEY_HEADER, pubkey_hex.as_str()),
            (TIMESTAMP_HEADER, "1000000"),
            (SIGNATURE_HEADER, sig_hex.as_str()),
        ];

        let result = parse_and_verify(&headers, "POST", &path, &account_id, b"tampered", now);
        assert_eq!(result.unwrap_err(), AuthError::BadSignature);
    }

    #[test]
    fn account_id_mismatch_is_rejected() {
        // A validly-signed envelope from a *real* key, but presented
        // against an account id that key does not actually hash to —
        // e.g. an attacker who has their own real key trying to write to
        // someone else's account id.
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();
        let wrong_account_id = "not-the-real-id";
        let now = 1_000_000;
        let path = format!("/v1/accounts/{wrong_account_id}/devices/d1/revoke");
        let sig_hex = sign_envelope(&signing_key, "POST", &path, wrong_account_id, now, b"body");

        let pubkey_hex = encode_hex(&pubkey);
        let headers = [
            (ACCOUNT_PUBKEY_HEADER, pubkey_hex.as_str()),
            (TIMESTAMP_HEADER, "1000000"),
            (SIGNATURE_HEADER, sig_hex.as_str()),
        ];

        let result = parse_and_verify(&headers, "POST", &path, wrong_account_id, b"body", now);
        assert_eq!(result.unwrap_err(), AuthError::AccountIdMismatch);
    }

    #[test]
    fn stale_timestamp_rejected() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();
        let account_id = derive_account_id(&pubkey).0;
        let request_time = 1_000_000;
        let server_now = request_time + TIMESTAMP_SKEW_SECS + 1;
        let path = format!("/v1/accounts/{account_id}/devices/d1/revoke");
        let sig_hex = sign_envelope(
            &signing_key,
            "POST",
            &path,
            &account_id,
            request_time,
            b"body",
        );
        let request_time_str = request_time.to_string();

        let pubkey_hex = encode_hex(&pubkey);
        let headers = [
            (ACCOUNT_PUBKEY_HEADER, pubkey_hex.as_str()),
            (TIMESTAMP_HEADER, request_time_str.as_str()),
            (SIGNATURE_HEADER, sig_hex.as_str()),
        ];

        let result = parse_and_verify(&headers, "POST", &path, &account_id, b"body", server_now);
        assert_eq!(result.unwrap_err(), AuthError::StaleTimestamp);
    }

    #[test]
    fn missing_header_rejected() {
        let headers: [(&str, &str); 0] = [];
        let result = parse_and_verify(&headers, "POST", "/x", "acct", b"", 0);
        assert_eq!(
            result.unwrap_err(),
            AuthError::MalformedHeader(ACCOUNT_PUBKEY_HEADER)
        );
    }
}
