//! ANKAI's `OpenMlsProvider`: persists OpenMLS's group/ratchet state into
//! the same SQLCipher-encrypted local DB everything else in `crate::db`
//! uses, per `docs/adr/0004-e2ee-stack.md`'s "one encrypted file, one key"
//! call for backing OpenMLS's storage trait.
//!
//! **Phase 1 durability tradeoff**: rather than implementing OpenMLS's
//! ~40-method `StorageProvider` trait against individual SQL rows,
//! `AnkaiMlsProvider` wraps `openmls_rust_crypto`'s own already-correct
//! in-memory `MemoryStorage` (its `values` map is a public field) and
//! round-trips that map's *entire* contents as one opaque blob via
//! `load`/`flush`. This means:
//! - The data really does live in the SQLCipher-encrypted file (satisfies
//!   the ADR's "one encrypted file, one key" intent), and all of OpenMLS's
//!   actual key-construction/serialization logic stays in OpenMLS's own
//!   already-reviewed-elsewhere `MemoryStorage`, not hand-ported here.
//! - Durability is whole-blob-grained: a crash between an MLS operation and
//!   the next explicit `flush` loses that operation, unlike a real
//!   per-row backend. Callers must `flush` after any operation that
//!   mutates MLS state.
//! - This is explicitly a Phase 1 simplification, not a final design —
//!   revisit before ADR-0004's "independent security audit" gate if
//!   per-operation durability turns out to matter for real group traffic.
//!
//! No new dependency was needed for this: `openmls_rust_crypto` already
//! re-exports `MemoryStorage`, and its `values` field is `pub`.

use std::collections::HashMap;

use openmls::prelude::OpenMlsProvider;
use openmls_rust_crypto::{MemoryStorage, RustCrypto};

use crate::db::Db;
use crate::error::Error;

/// Bundles `openmls_rust_crypto`'s default crypto/rand backend with a
/// `MemoryStorage` whose contents are loaded from, and persisted back into,
/// `db::Db` — see this module's doc comment for the persistence strategy.
pub struct AnkaiMlsProvider {
    crypto: RustCrypto,
    storage: MemoryStorage,
}

impl AnkaiMlsProvider {
    /// Loads any previously persisted MLS state from `db` into a fresh
    /// provider. Safe to call on a brand-new database — starts with empty
    /// storage in that case.
    pub fn load(db: &Db) -> Result<Self, Error> {
        let storage = MemoryStorage::default();
        if let Some(blob) = db.get_mls_storage_blob()? {
            let map = decode(&blob)?;
            *storage
                .values
                .write()
                .map_err(|_| Error::Db("MLS storage lock poisoned".to_string()))? = map;
        }

        Ok(Self {
            crypto: RustCrypto::default(),
            storage,
        })
    }

    /// Persists this provider's current MLS state back into `db`. Callers
    /// must call this after any operation that mutates MLS state — see this
    /// module's doc comment for the durability tradeoff of the current
    /// whole-blob persistence strategy.
    pub fn flush(&self, db: &Db) -> Result<(), Error> {
        let map = self
            .storage
            .values
            .read()
            .map_err(|_| Error::Db("MLS storage lock poisoned".to_string()))?;
        db.set_mls_storage_blob(&encode(&map))
    }
}

impl OpenMlsProvider for AnkaiMlsProvider {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = MemoryStorage;

    fn storage(&self) -> &Self::StorageProvider {
        &self.storage
    }

    fn crypto(&self) -> &Self::CryptoProvider {
        &self.crypto
    }

    fn rand(&self) -> &Self::RandProvider {
        &self.crypto
    }
}

/// A plain length-prefixed encoding of `MemoryStorage`'s
/// `HashMap<Vec<u8>, Vec<u8>>` — `count`, then `(key_len, value_len, key,
/// value)` per entry, all as big-endian `u64` lengths. Written by hand
/// rather than pulling in a general-purpose serialization crate for one
/// map of byte strings; not a public format, just this module's own
/// on-disk representation.
fn encode(map: &HashMap<Vec<u8>, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(map.len() as u64).to_be_bytes());
    for (key, value) in map {
        out.extend_from_slice(&(key.len() as u64).to_be_bytes());
        out.extend_from_slice(&(value.len() as u64).to_be_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(value);
    }
    out
}

fn decode(bytes: &[u8]) -> Result<HashMap<Vec<u8>, Vec<u8>>, Error> {
    let bad_blob = || Error::Db("corrupt MLS storage blob".to_string());

    let read_u64 = |cursor: &mut &[u8]| -> Result<u64, Error> {
        let (head, rest) = cursor.split_at_checked(8).ok_or_else(bad_blob)?;
        *cursor = rest;
        Ok(u64::from_be_bytes(head.try_into().unwrap()))
    };
    let read_bytes = |cursor: &mut &[u8], len: usize| -> Result<Vec<u8>, Error> {
        let (head, rest) = cursor.split_at_checked(len).ok_or_else(bad_blob)?;
        *cursor = rest;
        Ok(head.to_vec())
    };

    let mut cursor = bytes;
    let count = read_u64(&mut cursor)?;

    let mut map = HashMap::with_capacity(count as usize);
    for _ in 0..count {
        let key_len = read_u64(&mut cursor)? as usize;
        let value_len = read_u64(&mut cursor)? as usize;
        let key = read_bytes(&mut cursor, key_len)?;
        let value = read_bytes(&mut cursor, value_len)?;
        map.insert(key, value);
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmls::prelude::SignatureScheme;
    use openmls_basic_credential::SignatureKeyPair;

    #[test]
    fn signature_key_pair_survives_flush_and_reload() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();

        let public_key = {
            let provider = AnkaiMlsProvider::load(&db).unwrap();
            let keys = SignatureKeyPair::new(SignatureScheme::ED25519).unwrap();
            keys.store(provider.storage()).unwrap();
            provider.flush(&db).unwrap();
            keys.to_public_vec()
        };

        // A brand-new provider loaded from the same db must see the key
        // pair the first provider stored and flushed.
        let reloaded = AnkaiMlsProvider::load(&db).unwrap();
        let found =
            SignatureKeyPair::read(reloaded.storage(), &public_key, SignatureScheme::ED25519);
        assert!(
            found.is_some(),
            "signature key pair should have survived flush + reload"
        );
        assert_eq!(found.unwrap().to_public_vec(), public_key);
    }

    #[test]
    fn fresh_database_loads_empty_storage() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        let provider = AnkaiMlsProvider::load(&db).unwrap();
        assert!(provider.storage.values.read().unwrap().is_empty());
    }

    #[test]
    fn encode_decode_round_trips_empty_and_nonempty_maps() {
        let empty = HashMap::new();
        assert_eq!(decode(&encode(&empty)).unwrap(), empty);

        let mut map = HashMap::new();
        map.insert(b"key-one".to_vec(), b"value-one".to_vec());
        map.insert(Vec::new(), b"empty key".to_vec());
        map.insert(b"empty value".to_vec(), Vec::new());
        assert_eq!(decode(&encode(&map)).unwrap(), map);
    }
}
