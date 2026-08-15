//! Integration test for `docs/adr/0008-identity-discovery-service.md`'s
//! reference implementation: a real `app::router` server bound to a real
//! local TCP port, a real device identity (built the same way
//! `ankai_core::identity` builds one for the actual client), and a real
//! `HttpDirectoryClient` publishing/looking-up over genuine HTTP with real
//! ED25519 request-signing auth — proving the `DirectoryService` trait
//! boundary `core::directory::InMemoryDirectory` already validated
//! in-process also holds across a real process/network split, and that an
//! unsigned or badly-signed publish is genuinely rejected, not silently
//! accepted.

use std::sync::Arc;

use ankai_core::db::Db;
use ankai_core::directory::DirectoryService;
use ankai_core::identity::{
    create_key_package, load_or_create_device, Device, DeviceId, DEVICE_SIGNATURE_SCHEME,
};
use ankai_core::mls_provider::AnkaiMlsProvider;
use ankai_core::p2p::P2pNode;
use ankai_directory_server::db::Storage;
use ankai_directory_server::{app, HttpDirectoryClient};
use openmls_basic_credential::SignatureKeyPair;
use openmls_traits::signatures::Signer;
use openmls_traits::OpenMlsProvider;

/// Boots a real `app::router` server on an OS-assigned local port and
/// returns its base URL. The server runs for the lifetime of the test
/// process (background `tokio::spawn`, never joined) — fine for a
/// short-lived test binary.
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

/// A real device identity plus the real `SignatureKeyPair` backing it and
/// a real `KeyPackage` built for it — same machinery `client` itself would
/// use, not a test-only stand-in.
struct RealDevice {
    device: Device,
    signer: SignatureKeyPair,
    key_package: openmls::key_packages::KeyPackage,
}

fn real_device() -> RealDevice {
    let db = Db::open_in_memory("correct horse battery staple").expect("in-memory db should open");
    let provider = AnkaiMlsProvider::load(&db).expect("provider should load");
    let device = load_or_create_device(&db, &provider).expect("device should be created");
    provider.flush(&db).expect("flush should succeed");

    let published = create_key_package(&device, &provider).expect("key package should build");
    let key_package = published
        .key_package
        .expect("create_key_package always returns Some");

    let signer = SignatureKeyPair::read(
        provider.storage(),
        &device.signature_key.0,
        DEVICE_SIGNATURE_SCHEME,
    )
    .expect("device signature key should be readable back out of provider storage");

    RealDevice {
        device,
        signer,
        key_package,
    }
}

fn throwaway_client(base_url: String, device_id: DeviceId) -> HttpDirectoryClient {
    let signer =
        SignatureKeyPair::new(DEVICE_SIGNATURE_SCHEME).expect("keypair generation should succeed");
    HttpDirectoryClient::new(base_url, device_id, signer)
}

#[tokio::test]
async fn published_key_package_round_trips_through_a_separate_client_session_and_is_consumed() {
    let base_url = spawn_test_server().await;
    let real = real_device();

    let publisher = HttpDirectoryClient::new(base_url.clone(), real.device.id.clone(), real.signer);
    publisher
        .publish_key_package(&real.device.id, real.key_package.clone())
        .await
        .expect("publish should succeed with a valid device signature");

    // A separate client "session": a different HttpDirectoryClient
    // instance (different device identity entirely), doing a real lookup
    // over the network — not the same in-process struct that published it.
    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));

    let found = looker
        .key_packages(&real.device.id)
        .await
        .expect("lookup should succeed");
    assert_eq!(found, vec![real.key_package]);

    // Lookup consumes what it returns (per the ADR) — a second lookup finds nothing left.
    let found_again = looker
        .key_packages(&real.device.id)
        .await
        .expect("second lookup should succeed");
    assert!(
        found_again.is_empty(),
        "a second lookup should find nothing left to consume"
    );
}

#[tokio::test]
async fn published_endpoint_addr_round_trips_and_republishing_replaces_it() {
    let base_url = spawn_test_server().await;
    let real = real_device();

    let node_a = P2pNode::bind().await.expect("node a should bind");
    let node_b = P2pNode::bind().await.expect("node b should bind");
    let addr_a = node_a.addr();
    let addr_b = node_b.addr();

    let publisher = HttpDirectoryClient::new(base_url.clone(), real.device.id.clone(), real.signer);
    publisher
        .publish_endpoint_addr(&real.device.id, addr_a.clone())
        .await
        .expect("publish should succeed");

    let looker = throwaway_client(base_url.clone(), DeviceId("looker-session".to_string()));
    assert_eq!(
        looker.endpoint_addr(&real.device.id).await.unwrap(),
        Some(addr_a)
    );

    publisher
        .publish_endpoint_addr(&real.device.id, addr_b.clone())
        .await
        .expect("republish should succeed");
    assert_eq!(
        looker.endpoint_addr(&real.device.id).await.unwrap(),
        Some(addr_b)
    );

    node_a.close().await;
    node_b.close().await;
}

#[tokio::test]
async fn looking_up_a_device_nobody_published_for_is_empty_not_an_error() {
    let base_url = spawn_test_server().await;
    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    let nobody = DeviceId("does-not-exist".to_string());

    assert_eq!(looker.key_packages(&nobody).await.unwrap(), Vec::new());
    assert_eq!(looker.endpoint_addr(&nobody).await.unwrap(), None);
}

#[tokio::test]
async fn unsigned_publish_is_rejected_not_silently_accepted() {
    let base_url = spawn_test_server().await;
    let device_id = "victim-device";

    let raw = reqwest::Client::new();
    let resp = raw
        .post(format!("{base_url}/v1/devices/{device_id}/key-packages"))
        .body("{}")
        .send()
        .await
        .expect("request should complete (even though it should be rejected)");
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Confirm nothing was actually stored despite the attempt.
    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    let found = looker
        .key_packages(&DeviceId(device_id.to_string()))
        .await
        .unwrap();
    assert!(
        found.is_empty(),
        "an unsigned publish must not have been stored"
    );
}

#[tokio::test]
async fn badly_signed_publish_is_rejected() {
    let base_url = spawn_test_server().await;
    let real = real_device();
    let body = serde_json::to_vec(&real.key_package).unwrap();
    let path = format!("/v1/devices/{}/key-packages", real.device.id.0);

    // A signature produced by a key that is *not* the device's real key,
    // sent alongside the device's *real* claimed public key header — the
    // signature must not verify against a key that didn't produce it.
    let wrong_signer = SignatureKeyPair::new(DEVICE_SIGNATURE_SCHEME).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let msg =
        ankai_directory_server::auth::signing_message("POST", &path, &real.device.id.0, now, &body);
    let bad_signature = wrong_signer.sign(&msg).unwrap();

    let raw = reqwest::Client::new();
    let resp = raw
        .post(format!("{base_url}{path}"))
        .header(
            "x-ankai-device-pubkey",
            ankai_core::util::encode_hex(&real.device.signature_key.0),
        )
        .header("x-ankai-timestamp", now.to_string())
        .header(
            "x-ankai-signature",
            ankai_core::util::encode_hex(&bad_signature),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    let found = looker.key_packages(&real.device.id).await.unwrap();
    assert!(
        found.is_empty(),
        "a badly-signed publish must not have been stored"
    );
}

#[tokio::test]
async fn republishing_under_the_same_device_id_with_a_different_key_is_rejected() {
    let base_url = spawn_test_server().await;
    let real = real_device();

    let publisher = HttpDirectoryClient::new(base_url.clone(), real.device.id.clone(), real.signer);
    publisher
        .publish_key_package(&real.device.id, real.key_package.clone())
        .await
        .expect("first publish (which binds the TOFU key) should succeed");

    // An impostor who knows the victim's DeviceId but signs with its own,
    // different key — TOFU pinning from the first publish should reject this.
    let impostor_signer = SignatureKeyPair::new(DEVICE_SIGNATURE_SCHEME).unwrap();
    let impostor = HttpDirectoryClient::new(base_url, real.device.id.clone(), impostor_signer);
    let result = impostor
        .publish_key_package(&real.device.id, real.key_package)
        .await;
    assert!(
        result.is_err(),
        "publishing under an already-claimed device id with a different key must fail"
    );
}

// ---- Usernames (crate::username / Storage::claim_username /
// HttpDirectoryClient::claim_username, lookup_username) ----
//
// Same rigor bar as the KeyPackage/EndpointAddr tests above: real signed
// requests from real device identities against a real running server, not
// just exercising `Storage` in-process.

#[tokio::test]
async fn claimed_username_resolves_to_the_right_device_id_via_a_real_signed_request() {
    let base_url = spawn_test_server().await;
    let real = real_device();

    let claimant = HttpDirectoryClient::new(base_url.clone(), real.device.id.clone(), real.signer);
    claimant
        .claim_username(&real.device.id, "ash_ketchum")
        .await
        .expect("claiming a valid, unclaimed username with a real signature should succeed");

    // A separate client session (no shared state with the claimant) looks
    // it up over a real network round trip.
    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    let resolved = looker
        .lookup_username("ash_ketchum")
        .await
        .expect("lookup should succeed");
    assert_eq!(
        resolved,
        Some(real.device.id.clone()),
        "the username must resolve to the exact DeviceId that claimed it"
    );
}

#[tokio::test]
async fn a_different_real_device_cannot_steal_an_already_claimed_username() {
    let base_url = spawn_test_server().await;
    let owner = real_device();

    let owner_client =
        HttpDirectoryClient::new(base_url.clone(), owner.device.id.clone(), owner.signer);
    owner_client
        .claim_username(&owner.device.id, "misty")
        .await
        .expect("first claim should succeed");

    // A second, entirely independent real device identity — its own
    // signature key, its own DeviceId — tries to claim the same name.
    let impostor = real_device();
    let impostor_client = HttpDirectoryClient::new(
        base_url.clone(),
        impostor.device.id.clone(),
        impostor.signer,
    );
    let result = impostor_client
        .claim_username(&impostor.device.id, "misty")
        .await;
    assert!(
        result.is_err(),
        "a different device claiming an already-claimed username must be rejected"
    );

    // The username must still resolve to its real original owner.
    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    assert_eq!(
        looker.lookup_username("misty").await.unwrap(),
        Some(owner.device.id)
    );
}

#[tokio::test]
async fn the_same_device_can_update_its_own_claimed_username() {
    let base_url = spawn_test_server().await;
    let real = real_device();

    let client = HttpDirectoryClient::new(base_url.clone(), real.device.id.clone(), real.signer);
    client
        .claim_username(&real.device.id, "old_handle")
        .await
        .expect("first claim should succeed");
    client
        .claim_username(&real.device.id, "new_handle")
        .await
        .expect("the same device re-claiming under a new name should succeed");

    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    assert_eq!(
        looker.lookup_username("old_handle").await.unwrap(),
        None,
        "the old username should be freed once the device updates to a new one"
    );
    assert_eq!(
        looker.lookup_username("new_handle").await.unwrap(),
        Some(real.device.id)
    );
}

#[tokio::test]
async fn invalid_usernames_are_genuinely_rejected_by_the_real_server_not_silently_accepted() {
    let base_url = spawn_test_server().await;
    let real = real_device();
    let client = HttpDirectoryClient::new(base_url.clone(), real.device.id.clone(), real.signer);

    // Too short, bad characters, and an uppercase collision attempt against
    // an already-claimed lowercase name.
    for bad in ["ab", "Ash_Ketchum", "ash ketchum", &"x".repeat(21)] {
        let result = client.claim_username(&real.device.id, bad).await;
        assert!(
            result.is_err(),
            "expected {bad:?} to be rejected by real server-side validation"
        );
    }

    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    assert_eq!(looker.lookup_username("ab").await.unwrap(), None);
    assert_eq!(looker.lookup_username("ash_ketchum").await.unwrap(), None);
}

#[tokio::test]
async fn unsigned_username_claim_is_rejected_not_silently_accepted() {
    let base_url = spawn_test_server().await;
    let device_id = "victim-device";

    let raw = reqwest::Client::new();
    let resp = raw
        .post(format!("{base_url}/v1/devices/{device_id}/username"))
        .body(r#"{"username":"villain"}"#)
        .send()
        .await
        .expect("request should complete (even though it should be rejected)");
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    assert_eq!(
        looker.lookup_username("villain").await.unwrap(),
        None,
        "an unsigned claim must not have been stored"
    );
}

#[tokio::test]
async fn looking_up_a_username_nobody_claimed_is_empty_not_an_error() {
    let base_url = spawn_test_server().await;
    let looker = throwaway_client(base_url, DeviceId("looker-session".to_string()));
    assert_eq!(
        looker.lookup_username("nobody_has_this").await.unwrap(),
        None
    );
}
