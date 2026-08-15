//! ED25519 request-signing auth, per `docs/adr/0008-identity-discovery-service.md`'s
//! "Auth" section: a device signs a canonical representation of its request
//! with the same `SignatureKeyPair` `ankai_core::identity` already gives it
//! (no new cryptography, per README's non-negotiables), and the server
//! verifies that signature against the device's TOFU-pinned public key
//! before accepting a publish call.
//!
//! What this module does *not* do, matching the ADR's explicit scope: prove
//! the claimed public key is the "real" owner of a `DeviceId` in any
//! stronger sense than "first validly-signed publish wins" (TOFU), or
//! replay-protect beyond the timestamp skew window (no nonce cache). Both
//! are named as open gaps in the ADR, not silently solved here.

use ed25519_dalek::{Signature, VerifyingKey};

/// How far a request's `x-ankai-timestamp` may drift from the server's own
/// clock (either direction) before it's rejected as stale/replayed. Named
/// in the ADR's "Auth" section.
pub const TIMESTAMP_SKEW_SECS: i64 = 300;

pub const DEVICE_PUBKEY_HEADER: &str = "x-ankai-device-pubkey";
pub const TIMESTAMP_HEADER: &str = "x-ankai-timestamp";
pub const SIGNATURE_HEADER: &str = "x-ankai-signature";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("missing or malformed '{0}' header")]
    MalformedHeader(&'static str),
    #[error("request timestamp is outside the allowed skew window")]
    StaleTimestamp,
    #[error("signature does not verify against the claimed device public key")]
    BadSignature,
    #[error("device id is already bound to a different public key")]
    DeviceKeyMismatch,
}

/// The canonical byte string a device signs (and the server re-derives and
/// verifies against) for a given request. Covers the method, path, claimed
/// device id, timestamp, and the exact request body — binding the signature
/// to all of them so a captured signed request can't be replayed against a
/// different endpoint, device id, or with a tampered body.
pub fn signing_message(
    method: &str,
    path: &str,
    device_id: &str,
    timestamp: i64,
    body: &[u8],
) -> Vec<u8> {
    let mut msg = format!("{method}\n{path}\n{device_id}\n{timestamp}\n").into_bytes();
    msg.extend_from_slice(body);
    msg
}

/// A request's parsed (but not yet verified-against-storage) auth headers.
#[derive(Debug)]
pub struct SignedRequest {
    pub pubkey_hex: String,
    pub pubkey: [u8; 32],
    pub timestamp: i64,
}

/// Parses `x-ankai-device-pubkey`/`x-ankai-timestamp`/`x-ankai-signature`
/// out of `headers`, checks the timestamp is within [`TIMESTAMP_SKEW_SECS`]
/// of `now`, and verifies the signature against the claimed public key over
/// [`signing_message`]`(method, path, device_id, timestamp, body)`.
///
/// Does **not** check the pubkey is the one actually bound to `device_id` —
/// that's [`crate::db::Storage::bind_device_key`]'s job, since it needs
/// storage access this function deliberately doesn't take.
pub fn parse_and_verify(
    headers: &[(&str, &str)],
    method: &str,
    path: &str,
    device_id: &str,
    body: &[u8],
    now: i64,
) -> Result<SignedRequest, AuthError> {
    let header = |name: &'static str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
            .ok_or(AuthError::MalformedHeader(name))
    };

    let pubkey_hex = header(DEVICE_PUBKEY_HEADER)?;
    let timestamp_str = header(TIMESTAMP_HEADER)?;
    let signature_hex = header(SIGNATURE_HEADER)?;

    let pubkey_bytes = ankai_core::util::decode_hex(pubkey_hex)
        .ok_or(AuthError::MalformedHeader(DEVICE_PUBKEY_HEADER))?;
    let pubkey: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| AuthError::MalformedHeader(DEVICE_PUBKEY_HEADER))?;

    let timestamp: i64 = timestamp_str
        .parse()
        .map_err(|_| AuthError::MalformedHeader(TIMESTAMP_HEADER))?;
    if (timestamp - now).abs() > TIMESTAMP_SKEW_SECS {
        return Err(AuthError::StaleTimestamp);
    }

    let signature_bytes = ankai_core::util::decode_hex(signature_hex)
        .ok_or(AuthError::MalformedHeader(SIGNATURE_HEADER))?;
    let signature_bytes: [u8; 64] = signature_bytes
        .try_into()
        .map_err(|_| AuthError::MalformedHeader(SIGNATURE_HEADER))?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey)
        .map_err(|_| AuthError::MalformedHeader(DEVICE_PUBKEY_HEADER))?;
    let signature = Signature::from_bytes(&signature_bytes);

    let message = signing_message(method, path, device_id, timestamp, body);
    // `verify_strict` (rather than `verify`) rejects non-canonical/malleable
    // signature encodings — the stricter check is the right default for an
    // auth boundary, not just a consensus-critical one.
    verifying_key
        .verify_strict(&message, &signature)
        .map_err(|_| AuthError::BadSignature)?;

    Ok(SignedRequest {
        pubkey_hex: pubkey_hex.to_string(),
        pubkey,
        timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn sign(
        signing_key: &SigningKey,
        method: &str,
        path: &str,
        device_id: &str,
        timestamp: i64,
        body: &[u8],
    ) -> String {
        use ed25519_dalek::Signer;
        let msg = signing_message(method, path, device_id, timestamp, body);
        ankai_core::util::encode_hex(&signing_key.sign(&msg).to_bytes())
    }

    #[test]
    fn valid_signature_verifies() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_hex = ankai_core::util::encode_hex(signing_key.verifying_key().as_bytes());
        let now = 1_000_000;
        let sig_hex = sign(
            &signing_key,
            "POST",
            "/v1/devices/d1/key-packages",
            "d1",
            now,
            b"body",
        );

        let headers = [
            (DEVICE_PUBKEY_HEADER, pubkey_hex.as_str()),
            (TIMESTAMP_HEADER, "1000000"),
            (SIGNATURE_HEADER, sig_hex.as_str()),
        ];

        let parsed = parse_and_verify(
            &headers,
            "POST",
            "/v1/devices/d1/key-packages",
            "d1",
            b"body",
            now,
        )
        .unwrap();
        assert_eq!(parsed.pubkey_hex, pubkey_hex);
    }

    #[test]
    fn tampered_body_fails_verification() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_hex = ankai_core::util::encode_hex(signing_key.verifying_key().as_bytes());
        let now = 1_000_000;
        let sig_hex = sign(
            &signing_key,
            "POST",
            "/v1/devices/d1/key-packages",
            "d1",
            now,
            b"body",
        );

        let headers = [
            (DEVICE_PUBKEY_HEADER, pubkey_hex.as_str()),
            (TIMESTAMP_HEADER, "1000000"),
            (SIGNATURE_HEADER, sig_hex.as_str()),
        ];

        let result = parse_and_verify(
            &headers,
            "POST",
            "/v1/devices/d1/key-packages",
            "d1",
            b"tampered",
            now,
        );
        assert_eq!(result.unwrap_err(), AuthError::BadSignature);
    }

    #[test]
    fn stale_timestamp_rejected() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_hex = ankai_core::util::encode_hex(signing_key.verifying_key().as_bytes());
        let request_time = 1_000_000;
        let server_now = request_time + TIMESTAMP_SKEW_SECS + 1;
        let sig_hex = sign(
            &signing_key,
            "POST",
            "/v1/devices/d1/key-packages",
            "d1",
            request_time,
            b"body",
        );

        let headers = [
            (DEVICE_PUBKEY_HEADER, pubkey_hex.as_str()),
            (TIMESTAMP_HEADER, "1000000"),
            (SIGNATURE_HEADER, sig_hex.as_str()),
        ];

        let result = parse_and_verify(
            &headers,
            "POST",
            "/v1/devices/d1/key-packages",
            "d1",
            b"body",
            server_now,
        );
        assert_eq!(result.unwrap_err(), AuthError::StaleTimestamp);
    }

    #[test]
    fn missing_header_rejected() {
        let headers: [(&str, &str); 0] = [];
        let result = parse_and_verify(&headers, "POST", "/x", "d1", b"", 0);
        assert_eq!(
            result.unwrap_err(),
            AuthError::MalformedHeader(DEVICE_PUBKEY_HEADER)
        );
    }
}
