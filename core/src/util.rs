//! Small shared helpers with no better home — hex encoding and random
//! opaque-id generation, used by `keychain`, `identity`, and `communities`.
//! Hand-written rather than pulling in the `hex`/`uuid` crates: the actual
//! logic is a couple of lines and none of those modules need anything more
//! than "N random bytes, as an ASCII string."

/// Hex-encodes `bytes` (lowercase, no separators).
pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes a lowercase hex string back to bytes. `None` if `hex` has an odd
/// length or contains a non-hex-digit character.
pub fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// A fresh random 128-bit id, hex-encoded. Plenty of entropy to avoid
/// collisions for an opaque local identifier; these are ids, not secret key
/// material, so 128 bits is a deliberately lighter budget than e.g.
/// `keychain`'s 256-bit passphrases.
pub fn random_id() -> String {
    encode_hex(&rand::random::<[u8; 16]>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_including_empty() {
        assert_eq!(decode_hex(&encode_hex(&[])).unwrap(), Vec::<u8>::new());
        assert_eq!(
            decode_hex(&encode_hex(&[0, 1, 254, 255])).unwrap(),
            vec![0, 1, 254, 255]
        );
    }

    #[test]
    fn decode_hex_rejects_odd_length_and_non_hex_chars() {
        assert!(decode_hex("abc").is_none());
        assert!(decode_hex("zz").is_none());
    }

    #[test]
    fn random_id_is_32_hex_chars_and_not_constant() {
        let id = random_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(id, random_id());
    }
}
