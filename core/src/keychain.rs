//! OS-secure-storage-backed device secrets.
//!
//! See `docs/adr/0004-e2ee-stack.md`'s "Key management" section: the local
//! database's encryption key is a device-local secret, generated on first
//! run and stored via OS-level secure storage (macOS Keychain / Windows
//! Credential Manager / Linux Secret Service, via the `keyring` crate) —
//! never derived from or escrowed with the server. This module owns that
//! generate-once/read-thereafter lifecycle; `db::Db::open` still just takes
//! whatever passphrase string its caller supplies.

use keyring::Entry;

use crate::error::Error;
use crate::util::encode_hex;

const SERVICE: &str = "ankai";
const DB_KEY_ACCOUNT: &str = "device-db-key";

/// Returns this device's local database encryption passphrase, generating
/// and persisting a fresh one in OS secure storage the first time this is
/// called on a given device. Stable across calls/process restarts thereafter.
pub fn device_db_passphrase() -> Result<String, Error> {
    let entry = Entry::new(SERVICE, DB_KEY_ACCOUNT)
        .map_err(|e| Error::Keychain(format!("failed to access OS secure storage: {e}")))?;

    match entry.get_password() {
        Ok(passphrase) => Ok(passphrase),
        Err(keyring::Error::NoEntry) => {
            let passphrase = generate_passphrase();
            entry
                .set_password(&passphrase)
                .map_err(|e| Error::Keychain(format!("failed to store new device key: {e}")))?;
            Ok(passphrase)
        }
        Err(e) => Err(Error::Keychain(format!(
            "failed to read device key from OS secure storage: {e}"
        ))),
    }
}

/// A fresh 256-bit random passphrase, hex-encoded so it's a plain ASCII
/// string — both the OS keychain APIs and SQLCipher's PBKDF2 keying expect a
/// string, not raw bytes.
fn generate_passphrase() -> String {
    encode_hex(&rand::random::<[u8; 32]>())
}

#[cfg(test)]
mod tests {
    use super::*;

    // `device_db_passphrase` itself talks to a real OS keychain, which
    // isn't reliably available in headless CI sandboxes (no Secret Service
    // daemon on a bare Linux runner, keychain-access prompts on macOS) — so
    // only the pure passphrase-generation logic is unit-tested here. The
    // actual OS-storage round-trip is exercised by running the real client.
    #[test]
    fn generated_passphrase_is_64_hex_chars() {
        let p = generate_passphrase();
        assert_eq!(p.len(), 64);
        assert!(p.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generated_passphrases_are_not_all_identical() {
        assert_ne!(generate_passphrase(), generate_passphrase());
    }
}
