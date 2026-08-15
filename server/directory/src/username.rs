//! Human-readable usernames for the directory service.
//!
//! **Scope — read this first, same discipline as `core::top8`'s doc
//! comment applies here.** A username here is bound to a `DeviceId`, not to
//! a person or account: ANKAI has no real multi-device/account model yet
//! (`ankai_core::identity`'s own doc comment calls `AccountId` "an honest
//! placeholder... not derived from a real account root identity key"), so
//! "one username per person across every device they own" isn't a shape
//! this codebase can honestly build today. What this module *can* build,
//! and does, is the same first-come-first-served binding
//! `crate::db::Storage::bind_device_key` already uses for TOFU pubkey
//! pinning, applied to a second kind of claim: a device claims a username
//! the same way it proves it owns a `DeviceId` — by signing the claim
//! request with its real key (see `crate::auth`). **This should be
//! revisited once a real account/multi-device model exists** — most likely
//! by moving the claim from `DeviceId`-scoped to account-root-key-scoped,
//! not by pretending this is already that.
//!
//! ## Validation rule
//!
//! A username must be **3-20 characters, ASCII lowercase letters, digits,
//! or underscores only** (`^[a-z0-9_]{3,20}$`). Anything else — uppercase,
//! spaces, punctuation, unicode, too short, too long — is a real, explicit
//! rejection ([`UsernameError::Invalid`]), never silently lowercased or
//! truncated. Requiring lowercase-only input (rather than accepting mixed
//! case and folding it) is what makes uniqueness case-insensitive "for
//! free": since uppercase is rejected outright, there is no `Alice`/`alice`
//! pair that could ever both reach storage, so no separate case-folding
//! step is needed to detect the collision.
//!
//! ## Ownership
//!
//! Claiming an already-claimed username fails with
//! [`UsernameError::AlreadyClaimed`] unless the claiming device is the same
//! one that claimed it (proven the same way every other authenticated call
//! on this server is proven: a valid signature from the device's pinned
//! key — see `crate::auth::parse_and_verify` and
//! `crate::db::Storage::bind_device_key`). A device can re-claim/update its
//! own username to change it — see `crate::db::Storage::claim_username` for
//! the exact "first claim wins, same device can update, different device
//! can't steal" semantics, and why changing a username frees the old one.

/// Minimum accepted username length, in characters.
pub const MIN_LEN: usize = 3;
/// Maximum accepted username length, in characters.
pub const MAX_LEN: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsernameError {
    #[error(
        "username must be {MIN_LEN}-{MAX_LEN} characters, lowercase ascii letters/digits/underscores only (got {0:?})"
    )]
    Invalid(String),
    #[error("username is already claimed by a different device")]
    AlreadyClaimed,
}

/// Validates `raw` against this module's rule (see the module doc comment).
/// Returns `Ok(())` if it's acceptable as-is — this never normalizes or
/// modifies `raw`, only accepts or rejects it.
pub fn validate(raw: &str) -> Result<(), UsernameError> {
    let len_ok = (MIN_LEN..=MAX_LEN).contains(&raw.chars().count());
    let chars_ok = raw
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

    if len_ok && chars_ok {
        Ok(())
    } else {
        Err(UsernameError::Invalid(raw.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_valid_username() {
        assert!(validate("ash_ketchum1").is_ok());
        assert!(validate("abc").is_ok());
        assert!(validate(&"a".repeat(MAX_LEN)).is_ok());
    }

    #[test]
    fn rejects_uppercase() {
        assert_eq!(
            validate("Alice"),
            Err(UsernameError::Invalid("Alice".to_string()))
        );
    }

    #[test]
    fn rejects_too_short() {
        assert_eq!(
            validate("ab"),
            Err(UsernameError::Invalid("ab".to_string()))
        );
    }

    #[test]
    fn rejects_too_long() {
        let too_long = "a".repeat(MAX_LEN + 1);
        assert_eq!(
            validate(&too_long),
            Err(UsernameError::Invalid(too_long.clone()))
        );
    }

    #[test]
    fn rejects_spaces_and_punctuation() {
        assert!(validate("ash ketchum").is_err());
        assert!(validate("ash-ketchum").is_err());
        assert!(validate("ash.ketchum").is_err());
    }

    #[test]
    fn rejects_unicode() {
        assert!(validate("aşh_ketchum").is_err());
    }
}
