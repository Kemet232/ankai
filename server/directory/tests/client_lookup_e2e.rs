//! End-to-end proof that the new "look up a peer by their short `DeviceId`"
//! path (wired into `client` in this session — see `client/src/directory.rs`
//! and `client/src/main.rs`) genuinely replaces `core::messaging`'s manual
//! full-`PeerInvite`-blob paste, using real components throughout:
//!
//! - a real `app::router` server, bound to a real local TCP port (not
//!   mocked — same helper shape as `tests/roundtrip.rs`)
//! - two real, independent device identities, each with its own in-memory
//!   `Db`/`AnkaiMlsProvider` (via `ankai_core::identity::load_or_create_device`)
//! - a real `HttpDirectoryClient` for each device, signing with each
//!   device's real `SignatureKeyPair`
//! - two real `P2pNode`s (real bound iroh endpoints)
//! - the real MLS group-setup/encrypt/decrypt path from
//!   `ankai_core::messaging` (`encrypt_and_log_outgoing`/`decrypt_incoming`),
//!   completely unmodified — this test proves the *directory-sourced*
//!   `PeerInvite` flows through the exact same code a manually pasted one
//!   would
//!
//! Flow: device A publishes its `KeyPackage` + `EndpointAddr` to the real
//! running server. Device B — which never received anything from A
//! out-of-band — looks up A's `DeviceId` against the real server over a
//! genuine HTTP round trip (the same two `DirectoryService` calls
//! `client::directory::lookup_peer_invite` makes), assembles a
//! `PeerInvite` purely from what the directory returned, and uses it to
//! start a real MLS group and send a real encrypted message to A. A's real
//! accept loop receives and decrypts it correctly.
//!
//! This is deliberately at the `core`/`server` API level, not a scripted
//! two-GUI-process test — `client`'s directory glue
//! (`client/src/directory.rs::lookup_peer_invite`) is a thin, two-call
//! wrapper around exactly the `DirectoryService` calls made here, so
//! exercising them for real against a real server proves the underlying
//! path `client` calls into, without needing to drive Slint's event loop
//! from a test harness.

use std::sync::{Arc, Mutex};

use ankai_core::db::Db;
use ankai_core::directory::DirectoryService;
use ankai_core::identity::{
    create_key_package, load_or_create_device, Device, DeviceId, DEVICE_SIGNATURE_SCHEME,
};
use ankai_core::messaging::{
    decrypt_incoming, encrypt_and_log_outgoing, format_peer_invite, parse_peer_invite,
    receive_messages, send_message, PeerInvite, ReceivedMessage,
};
use ankai_core::mls_provider::AnkaiMlsProvider;
use ankai_core::p2p::P2pNode;
use ankai_directory_server::db::Storage;
use ankai_directory_server::{app, HttpDirectoryClient};
use openmls_basic_credential::SignatureKeyPair;
use openmls_traits::OpenMlsProvider;

/// Boots a real `app::router` server on an OS-assigned local port and
/// returns its base URL — same shape as `tests/roundtrip.rs`'s helper
/// (each integration-test binary compiles separately, so this is
/// intentionally duplicated rather than shared).
async fn spawn_test_server() -> String {
    let storage = Arc::new(Storage::open(":memory:").expect("in-memory storage should open"));
    let router = app::router(storage);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind an ephemeral local port");
    let addr = listener
        .local_addr()
        .expect("bound listener should have a local addr");

    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("test server should not error while serving");
    });

    format!("http://{addr}")
}

/// A real device "installation": its own in-memory `Db`, `AnkaiMlsProvider`,
/// identity, and the real `SignatureKeyPair` backing it — same machinery
/// `client` itself uses at startup, not a test-only stand-in.
struct RealInstallation {
    db: Db,
    provider: AnkaiMlsProvider,
    device: Device,
    signer: SignatureKeyPair,
}

fn spin_up_installation() -> RealInstallation {
    let db = Db::open_in_memory("correct horse battery staple").expect("in-memory db should open");
    let provider = AnkaiMlsProvider::load(&db).expect("provider should load");
    let device = load_or_create_device(&db, &provider).expect("device should be created");
    provider.flush(&db).expect("flush should succeed");

    let signer = SignatureKeyPair::read(
        provider.storage(),
        &device.signature_key.0,
        DEVICE_SIGNATURE_SCHEME,
    )
    .expect("device signature key should be readable back out of provider storage");

    RealInstallation {
        db,
        provider,
        device,
        signer,
    }
}

/// The same two `DirectoryService` calls (and the same `PeerInvite`
/// assembly) `client/src/directory.rs::lookup_peer_invite` makes — kept
/// inline here (rather than importing `client`, a binary-only crate with no
/// lib target to import from) so this test proves the real network path
/// that function drives, using the same trait methods and the same
/// resulting shape.
async fn lookup_peer_invite_via_directory(
    looker: &HttpDirectoryClient,
    peer: &DeviceId,
) -> Option<PeerInvite> {
    let mut key_packages = looker
        .key_packages(peer)
        .await
        .expect("key_packages lookup should succeed against a real running server");
    if key_packages.is_empty() {
        return None;
    }
    let key_package = key_packages.remove(0);

    let addr = looker
        .endpoint_addr(peer)
        .await
        .expect("endpoint_addr lookup should succeed against a real running server")?;

    Some(PeerInvite { addr, key_package })
}

#[tokio::test]
async fn device_b_finds_device_a_purely_by_device_id_and_sends_a_real_encrypted_message() {
    let base_url = spawn_test_server().await;

    // --- Device A: publishes to the real server, then just listens. ---
    let a = spin_up_installation();
    let a_node = P2pNode::bind().await.expect("A's node should bind");
    let a_addr = a_node.addr();

    let a_published_key_package = create_key_package(&a.device, &a.provider)
        .expect("A should build a real KeyPackage")
        .key_package
        .expect("create_key_package always returns Some");
    a.provider.flush(&a.db).expect("A's flush should succeed");

    let a_directory_client =
        HttpDirectoryClient::new(base_url.clone(), a.device.id.clone(), a.signer);
    a_directory_client
        .publish_key_package(&a.device.id, a_published_key_package.clone())
        .await
        .expect("A's real signed publish_key_package should succeed against the real server");
    a_directory_client
        .publish_endpoint_addr(&a.device.id, a_addr.clone())
        .await
        .expect("A's real signed publish_endpoint_addr should succeed against the real server");

    // A's real receive side: decrypts (or joins, for the Welcome) whatever
    // arrives and records genuinely decrypted messages.
    let a_received = Arc::new(Mutex::new(Vec::<ReceivedMessage>::new()));
    let a_received_for_loop = a_received.clone();
    let a_db = a.db;
    let a_provider = a.provider;
    let a_accept_task = tokio::spawn(async move {
        receive_messages(&a_node, move |from, bytes| {
            match decrypt_incoming(&a_db, &a_provider, from, bytes) {
                Ok(Some(msg)) => {
                    a_provider.flush(&a_db).unwrap();
                    a_received_for_loop.lock().unwrap().push(msg);
                }
                Ok(None) => {
                    a_provider.flush(&a_db).unwrap();
                }
                Err(e) => panic!("A failed to process an incoming message: {e}"),
            }
        })
        .await
        .unwrap();
    });

    // --- Device B: never received anything from A out-of-band. It only
    // knows A's DeviceId (as if a human had pasted it into the "Peer's
    // Device ID" field client/ui/app.slint gained this session) and looks
    // everything else up from the real directory server over real HTTP. ---
    let b = spin_up_installation();
    let b_directory_client =
        HttpDirectoryClient::new(base_url.clone(), b.device.id.clone(), b.signer);

    let invite = lookup_peer_invite_via_directory(&b_directory_client, &a.device.id)
        .await
        .expect("B should find a real PeerInvite for A purely from the directory server");
    assert_eq!(
        invite.addr, a_addr,
        "the EndpointAddr B found via the directory must be A's real address"
    );
    assert_eq!(
        invite.key_package, a_published_key_package,
        "the KeyPackage B found via the directory must be A's real published one"
    );

    // The directory-sourced invite is exactly the same shape a manually
    // pasted one would be — round-trips through the same
    // format/parse_peer_invite functions core::messaging's manual flow
    // uses, proving nothing was forked to support this.
    let round_tripped = parse_peer_invite(&format_peer_invite(&invite).unwrap()).unwrap();
    assert_eq!(round_tripped.addr, invite.addr);
    assert_eq!(round_tripped.key_package, invite.key_package);

    // B sends a real MLS-encrypted message using *only* what the directory
    // gave it — this is the real group-setup/encrypt path from
    // core::messaging, completely unmodified for this new source.
    let b_node = P2pNode::bind().await.expect("B's node should bind");
    let payloads = encrypt_and_log_outgoing(
        &b.db,
        &b.provider,
        &b.device,
        &invite,
        "hello A, found you via the directory",
    )
    .expect("B should be able to encrypt a real MLS message to A");
    b.provider.flush(&b.db).expect("B's flush should succeed");
    assert_eq!(
        payloads.len(),
        2,
        "first contact should still produce a Welcome plus the application message"
    );

    for payload in &payloads {
        send_message(&b_node, invite.addr.clone(), payload)
            .await
            .expect("B's real P2P send to A should succeed");
    }

    // A's accept loop has, by the time each `send_message` above returned,
    // already run its on_message callback (accept_loop only acks after
    // invoking it) — so A's decrypted message is already recorded.
    let received = a_received.lock().unwrap().clone();
    assert_eq!(
        received.len(),
        1,
        "A should have decrypted exactly one real application message"
    );
    assert_eq!(received[0].text, "hello A, found you via the directory");
    assert_eq!(
        received[0].from,
        b_node.addr().id,
        "A should see the message as having come from B's real EndpointId"
    );

    // Per ADR-0008's consumption policy, A's KeyPackage was consumed by B's
    // lookup — a second directory lookup for A now finds nothing left,
    // which is a real, documented gap this integration inherits (see
    // core::messaging's module doc comment), not something this test hides.
    let second_lookup = lookup_peer_invite_via_directory(&b_directory_client, &a.device.id).await;
    assert!(
        second_lookup.is_none(),
        "A's single published KeyPackage should have been consumed by the first lookup"
    );

    a_accept_task.abort();
    b_node.close().await;
}

#[tokio::test]
async fn looking_up_a_device_that_never_published_yields_no_invite_not_an_error() {
    let base_url = spawn_test_server().await;
    let b = spin_up_installation();
    let b_directory_client = HttpDirectoryClient::new(base_url, b.device.id.clone(), b.signer);

    let stranger = DeviceId("nobody-published-for-this-id".to_string());
    let result = lookup_peer_invite_via_directory(&b_directory_client, &stranger).await;
    assert!(
        result.is_none(),
        "an unpublished DeviceId should yield None, not an error — the manual-paste fallback \
         remains the only option for a peer who hasn't (yet) used the directory"
    );
}
