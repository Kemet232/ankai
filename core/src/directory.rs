//! Client-side interface to ANKAI's "thin cloud" identity/discovery service —
//! the directory devices publish `KeyPackage`s (and, optionally, a current
//! P2P address) to, and that other devices query to find them. Per
//! `docs/adr/0004-e2ee-stack.md`: "each device publishes `KeyPackage`s to a
//! directory ahead of time, so a sender can initiate a conversation before
//! the recipient is ever online." `crate::identity` and `crate::p2p` both
//! already point at this exact gap in their own doc comments —
//! `identity::PublishedKeyPackage.key_package` stays `Option` because
//! *publishing* to a directory is a distinct, unbuilt step from building a
//! `KeyPackage` locally, and `p2p`'s doc comment says peer discovery (how a
//! caller learns a remote's `EndpointAddr`) is this same centralized
//! service, out of that module's scope.
//!
//! **What this module is not**: a real server, or a decision about how a
//! client would talk to one. Which concrete technology that server uses
//! (HTTP? gRPC? what auth? what database? self-hosted or managed?) is an
//! undecided architectural question of the same weight as ADRs 0002-0007 —
//! deliberately not answered here. This module only fixes the *shape* of
//! the boundary a real client-side "directory client" would need to conform
//! to, once that ADR lands and a server actually exists on the other end.
//!
//! [`InMemoryDirectory`] is the one implementation provided: a
//! `Mutex`-backed, process-local stand-in, useful for tests and as a
//! concrete reference for the trait's intended semantics. It is **not**
//! wired into `client` — there is nothing at the other end of a network
//! call for it to stand in for yet, and wiring it in would fake multi-
//! device discovery that doesn't actually work.
//!
//! Left for a real, server-backed implementation to figure out (not modeled
//! here):
//! - **Auth** — what stops a caller from publishing a `KeyPackage` (or
//!   `EndpointAddr`) under a `DeviceId` that isn't theirs. Per
//!   `docs/threat-model.md`'s "Identity & authentication" section, device
//!   keys must be signed by the account's root identity key and the server
//!   must never issue device trust on its own authority — this trait has no
//!   opinion on how that gets enforced.
//! - **Staleness/expiry and consumption** — MLS `KeyPackage`s are meant to
//!   be used once each (Signal-style prekeys); a real directory needs to
//!   decide whether a lookup consumes/removes what it returns, and what
//!   happens once a device runs out. [`InMemoryDirectory::key_packages`]
//!   just returns everything currently published for a device, unconsumed.
//! - **Revocation** — a device losing its signature key (or a whole account
//!   being compromised) needs a way to invalidate whatever it already
//!   published; not modeled here.
//! - **`EndpointAddr` freshness** — direct network addresses go stale as NAT
//!   mappings/relay assignments change; a real server needs a refresh/expiry
//!   policy this trait doesn't attempt to define.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use iroh::EndpointAddr;
use openmls::key_packages::KeyPackage;

use crate::error::Error;
use crate::identity::DeviceId;

/// What ANKAI's client code needs from a directory/discovery service: a way
/// to publish a device's `KeyPackage`s (and current `EndpointAddr`) so other
/// devices can find it, and a way to look both up. See this module's doc
/// comment for what real server-backed implementations still need to
/// decide (auth, expiry/consumption, revocation) that this trait doesn't.
///
/// Methods are spelled as `fn(..) -> impl Future<..> + Send` rather than
/// `async fn` — plain `async fn` in a public trait can't express the `Send`
/// bound its `Future` needs (rustc's own `async_fn_in_trait` lint flags
/// exactly this), and a real network-backed implementation will need that
/// bound to run its futures on a multi-threaded executor.
pub trait DirectoryService {
    /// Publishes `key_package` as available for another device to fetch when
    /// starting a conversation with `device` while it's offline. Additive —
    /// does not replace previously published key packages for `device`, see
    /// [`Self::key_packages`].
    fn publish_key_package(
        &self,
        device: &DeviceId,
        key_package: KeyPackage,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Every `KeyPackage` currently published for `device`, oldest first.
    /// Empty (not an error) if `device` has never published one.
    fn key_packages(
        &self,
        device: &DeviceId,
    ) -> impl Future<Output = Result<Vec<KeyPackage>, Error>> + Send;

    /// Publishes `addr` as `device`'s current dialable P2P endpoint address
    /// (see `crate::p2p::P2pNode::addr`), replacing whatever was previously
    /// published for it.
    fn publish_endpoint_addr(
        &self,
        device: &DeviceId,
        addr: EndpointAddr,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// The most recently published `EndpointAddr` for `device`, or `None` if
    /// it has never published one.
    fn endpoint_addr(
        &self,
        device: &DeviceId,
    ) -> impl Future<Output = Result<Option<EndpointAddr>, Error>> + Send;
}

/// A process-local, `Mutex`-backed [`DirectoryService`]. See this module's
/// doc comment for why this is a reference implementation and test double,
/// not a real server or anything wired into `client`.
#[derive(Default)]
pub struct InMemoryDirectory {
    key_packages: Mutex<HashMap<DeviceId, Vec<KeyPackage>>>,
    endpoint_addrs: Mutex<HashMap<DeviceId, EndpointAddr>>,
}

impl InMemoryDirectory {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DirectoryService for InMemoryDirectory {
    async fn publish_key_package(
        &self,
        device: &DeviceId,
        key_package: KeyPackage,
    ) -> Result<(), Error> {
        self.key_packages
            .lock()
            .map_err(|_| Error::Directory("key package store lock poisoned".to_string()))?
            .entry(device.clone())
            .or_default()
            .push(key_package);
        Ok(())
    }

    async fn key_packages(&self, device: &DeviceId) -> Result<Vec<KeyPackage>, Error> {
        Ok(self
            .key_packages
            .lock()
            .map_err(|_| Error::Directory("key package store lock poisoned".to_string()))?
            .get(device)
            .cloned()
            .unwrap_or_default())
    }

    async fn publish_endpoint_addr(
        &self,
        device: &DeviceId,
        addr: EndpointAddr,
    ) -> Result<(), Error> {
        self.endpoint_addrs
            .lock()
            .map_err(|_| Error::Directory("endpoint addr store lock poisoned".to_string()))?
            .insert(device.clone(), addr);
        Ok(())
    }

    async fn endpoint_addr(&self, device: &DeviceId) -> Result<Option<EndpointAddr>, Error> {
        Ok(self
            .endpoint_addrs
            .lock()
            .map_err(|_| Error::Directory("endpoint addr store lock poisoned".to_string()))?
            .get(device)
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::identity::{create_key_package, load_or_create_device, CIPHERSUITE};
    use crate::mls_provider::AnkaiMlsProvider;
    use crate::p2p::P2pNode;

    #[tokio::test]
    async fn published_key_package_round_trips_through_lookup() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        let provider = AnkaiMlsProvider::load(&db).unwrap();
        let device = load_or_create_device(&db, &provider).unwrap();

        let published = create_key_package(&device, &provider).unwrap();
        let key_package = published
            .key_package
            .expect("create_key_package should always return Some");

        let directory = InMemoryDirectory::new();
        directory
            .publish_key_package(&device.id, key_package.clone())
            .await
            .unwrap();

        let found = directory.key_packages(&device.id).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ciphersuite(), CIPHERSUITE);
        assert_eq!(found[0], key_package);
    }

    #[tokio::test]
    async fn publishing_twice_accumulates_rather_than_replacing() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        let provider = AnkaiMlsProvider::load(&db).unwrap();
        let device = load_or_create_device(&db, &provider).unwrap();

        let first = create_key_package(&device, &provider)
            .unwrap()
            .key_package
            .unwrap();
        let second = create_key_package(&device, &provider)
            .unwrap()
            .key_package
            .unwrap();
        assert_ne!(
            first, second,
            "two independently built key packages should differ (fresh HPKE material each time)"
        );

        let directory = InMemoryDirectory::new();
        directory
            .publish_key_package(&device.id, first.clone())
            .await
            .unwrap();
        directory
            .publish_key_package(&device.id, second.clone())
            .await
            .unwrap();

        let found = directory.key_packages(&device.id).await.unwrap();
        assert_eq!(found, vec![first, second]);
    }

    #[tokio::test]
    async fn endpoint_addr_round_trips_and_republishing_replaces_it() {
        let alice = DeviceId("alice-device".to_string());

        let node_a = P2pNode::bind().await.unwrap();
        let node_b = P2pNode::bind().await.unwrap();
        let addr_a = node_a.addr();
        let addr_b = node_b.addr();

        let directory = InMemoryDirectory::new();
        directory
            .publish_endpoint_addr(&alice, addr_a.clone())
            .await
            .unwrap();
        assert_eq!(directory.endpoint_addr(&alice).await.unwrap(), Some(addr_a));

        // Republishing (e.g. the device reconnected under a new address)
        // replaces the previous one rather than accumulating, unlike
        // key packages.
        directory
            .publish_endpoint_addr(&alice, addr_b.clone())
            .await
            .unwrap();
        assert_eq!(directory.endpoint_addr(&alice).await.unwrap(), Some(addr_b));

        node_a.close().await;
        node_b.close().await;
    }

    #[tokio::test]
    async fn looking_up_a_device_nobody_published_for_is_empty_not_an_error() {
        let directory = InMemoryDirectory::new();
        let nobody = DeviceId("does-not-exist".to_string());

        assert_eq!(directory.key_packages(&nobody).await.unwrap(), Vec::new());
        assert_eq!(directory.endpoint_addr(&nobody).await.unwrap(), None);
    }
}
