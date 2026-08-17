//! Integration test for `docs/adr/0009-account-system.md`'s reference
//! implementation: a real `app::router` server bound to a real local TCP
//! port, real `ankai_core::account::AccountRootKeyPair`s, and a real
//! `AccountsClient` adding/listing/revoking devices and storing/fetching a
//! recovery blob over genuine HTTP — proving the happy path, plus a real
//! rejection-path bar matching `server/directory`'s own integration tests:
//! unauthenticated calls, malformed/tampered signatures, and impersonation
//! attempts (a different account trying to claim an already-registered
//! device id, or forge a revoke/recovery-blob request for an account it
//! doesn't hold the key for) are all genuinely rejected, not silently
//! accepted.

use std::sync::Arc;

use ankai_accounts_server::db::Storage;
use ankai_accounts_server::{app, auth, AccountsClient};
use ankai_core::account::{sign_device_registration, AccountRootKeyPair};
use ankai_core::identity::DeviceId;
use ankai_core::util::encode_hex;

/// Boots a real `app::router` server on an OS-assigned local port and
/// returns its base URL — same shape as `server/directory/tests/roundtrip.rs`.
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

fn device_public_key(seed: u8) -> Vec<u8> {
    vec![seed; 32]
}

#[tokio::test]
async fn adding_a_device_creates_the_account_and_lists_it() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let client = AccountsClient::new(base_url, account_key);

    client
        .add_device(DeviceId("device-1".to_string()), device_public_key(1))
        .await
        .expect("first device add should implicitly create the account");

    let listed = client
        .list_devices(&client.account_id())
        .await
        .expect("list_devices should succeed")
        .expect("account should exist after adding a device");
    assert_eq!(listed.devices.len(), 1);
    assert_eq!(listed.devices[0].device_id, "device-1");
}

#[tokio::test]
async fn a_second_device_is_added_alongside_the_first() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let client = AccountsClient::new(base_url, account_key);

    client
        .add_device(DeviceId("device-1".to_string()), device_public_key(1))
        .await
        .unwrap();
    client
        .add_device(DeviceId("device-2".to_string()), device_public_key(2))
        .await
        .unwrap();

    let listed = client
        .list_devices(&client.account_id())
        .await
        .unwrap()
        .unwrap();
    let ids: Vec<&str> = listed
        .devices
        .iter()
        .map(|d| d.device_id.as_str())
        .collect();
    assert_eq!(ids, vec!["device-1", "device-2"]);
}

#[tokio::test]
async fn listed_registrations_are_independently_verifiable_without_trusting_the_server() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let client = AccountsClient::new(base_url, account_key);
    client
        .add_device(DeviceId("device-1".to_string()), device_public_key(1))
        .await
        .unwrap();

    let listed = client
        .list_devices(&client.account_id())
        .await
        .unwrap()
        .unwrap();

    // A caller who only has the device rows and the account's public key
    // (both returned by the same call) can verify the binding themselves —
    // reconstruct a DeviceRegistration from the row and check it, exactly
    // as ankai_core::account::verify_device_registration expects.
    let row = &listed.devices[0];
    let registration = ankai_core::account::DeviceRegistration {
        account: ankai_core::identity::AccountId(client.account_id()),
        device: DeviceId(row.device_id.clone()),
        device_public_key: ankai_core::util::decode_hex(&row.device_public_key_hex).unwrap(),
        signed_at_unix: row.signed_at_unix,
        signature: ankai_core::util::decode_hex(&row.signature_hex).unwrap(),
    };
    ankai_core::account::verify_device_registration(&registration, &listed.account_public_key)
        .expect("a genuinely returned registration should verify independently");
}

#[tokio::test]
async fn listing_an_unknown_account_is_none_not_an_error() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let client = AccountsClient::new(base_url, account_key);

    let result = client.list_devices("some-account-nobody-registered").await;
    assert!(matches!(result, Ok(None)));
}

#[tokio::test]
async fn revoked_device_disappears_from_the_active_list() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let client = AccountsClient::new(base_url, account_key);
    let device = DeviceId("device-1".to_string());
    client
        .add_device(device.clone(), device_public_key(1))
        .await
        .unwrap();

    client
        .revoke_device(&device)
        .await
        .expect("revoke should succeed");

    let listed = client
        .list_devices(&client.account_id())
        .await
        .unwrap()
        .unwrap();
    assert!(listed.devices.is_empty());
}

#[tokio::test]
async fn recovery_blob_round_trips_within_one_client_session() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let identity = account_key.identity();
    let client = AccountsClient::new(base_url, account_key);

    assert_eq!(client.get_recovery_blob().await.unwrap(), None);

    client
        .put_recovery_blob(b"opaque-ciphertext".to_vec())
        .await
        .expect("put should succeed");

    let fetched = client.get_recovery_blob().await.unwrap();
    assert_eq!(fetched, Some(b"opaque-ciphertext".to_vec()));
    assert_eq!(client.account_id(), identity.id.0);
}

#[tokio::test]
async fn recovery_blob_survives_a_real_client_session_rebuilt_from_the_recovery_phrase() {
    let base_url = spawn_test_server().await;
    let (account_key, phrase) = AccountRootKeyPair::generate();
    let first_session = AccountsClient::new(base_url.clone(), account_key);
    first_session
        .put_recovery_blob(b"my-encrypted-backup".to_vec())
        .await
        .unwrap();
    let account_id = first_session.account_id();
    drop(first_session);

    // A genuinely new AccountsClient, on a "new device" that never talked
    // to the server before, reconstructed purely from the 24-word phrase —
    // the actual break-glass recovery path this ADR proposes.
    let restored_key =
        AccountRootKeyPair::from_mnemonic(&phrase).expect("phrase should restore the same key");
    let recovered_session = AccountsClient::new(base_url, restored_key);
    assert_eq!(recovered_session.account_id(), account_id);

    let fetched = recovered_session.get_recovery_blob().await.unwrap();
    assert_eq!(fetched, Some(b"my-encrypted-backup".to_vec()));
}

// ---- Rejection paths ----

#[tokio::test]
async fn unauthenticated_add_device_is_rejected() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let account_id = account_key.identity().id.0;

    // Real registration bytes, but posted raw without ever going through a
    // real AccountsClient — proves the server itself enforces this, not
    // just the client being well-behaved. This case is actually still
    // *authenticated* by the embedded DeviceRegistration signature (see
    // crate::auth's doc comment on why add_device doesn't use the generic
    // envelope) — so the real "unauthenticated" case for this endpoint is
    // a garbage/unsigned body, exercised in the next test.
    let raw = reqwest::Client::new();
    let resp = raw
        .post(format!("{base_url}/v1/accounts/{account_id}/devices"))
        .body("not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn add_device_with_a_forged_registration_signature_is_rejected() {
    let base_url = spawn_test_server().await;
    let (real_account, _phrase) = AccountRootKeyPair::generate();
    let (impostor_account, _phrase2) = AccountRootKeyPair::generate();

    // The impostor signs a registration with *their own* key but claims
    // it's for the real account (tampering the `account` field after
    // signing, the same way an attacker with no private key access would
    // have to).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut forged_registration = sign_device_registration(
        &impostor_account,
        DeviceId("stolen-device".to_string()),
        device_public_key(9),
        now,
    );
    forged_registration.account = real_account.identity().id;

    #[derive(serde::Serialize)]
    struct Body<'a> {
        account_public_key_hex: String,
        registration: &'a ankai_core::account::DeviceRegistration,
    }
    let body = serde_json::to_vec(&Body {
        account_public_key_hex: encode_hex(&real_account.identity().public_key),
        registration: &forged_registration,
    })
    .unwrap();

    let raw = reqwest::Client::new();
    let resp = raw
        .post(format!(
            "{base_url}/v1/accounts/{}/devices",
            real_account.identity().id.0
        ))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Confirm nothing was actually stored.
    let client = AccountsClient::new(base_url, real_account);
    let listed = client.list_devices(&client.account_id()).await.unwrap();
    assert!(
        listed.is_none(),
        "a forged registration must not have created the account"
    );
}

#[tokio::test]
async fn a_different_account_cannot_steal_an_already_registered_device_id() {
    let base_url = spawn_test_server().await;
    let (owner, _phrase) = AccountRootKeyPair::generate();
    let owner_client = AccountsClient::new(base_url.clone(), owner);
    let device = DeviceId("shared-device-id".to_string());
    owner_client
        .add_device(device.clone(), device_public_key(1))
        .await
        .expect("original owner should register the device");

    let (impostor, _phrase2) = AccountRootKeyPair::generate();
    let impostor_client = AccountsClient::new(base_url, impostor);
    let result = impostor_client
        .add_device(device, device_public_key(2))
        .await;
    assert!(
        result.is_err(),
        "a different account claiming an already-registered device id must be rejected"
    );

    // The device is still listed only under its real owner.
    let listed = owner_client
        .list_devices(&owner_client.account_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listed.devices.len(), 1);
    let impostor_listed = impostor_client
        .list_devices(&impostor_client.account_id())
        .await
        .unwrap();
    assert!(impostor_listed.is_none());
}

#[tokio::test]
async fn unauthenticated_revoke_is_rejected() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let account_id = account_key.identity().id.0;
    let client = AccountsClient::new(base_url.clone(), account_key);
    client
        .add_device(DeviceId("device-1".to_string()), device_public_key(1))
        .await
        .unwrap();

    let raw = reqwest::Client::new();
    let resp = raw
        .post(format!(
            "{base_url}/v1/accounts/{account_id}/devices/device-1/revoke"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Still listed — the unauthenticated revoke attempt had no effect.
    let listed = client
        .list_devices(&client.account_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listed.devices.len(), 1);
}

#[tokio::test]
async fn revoking_someone_elses_device_without_their_key_is_rejected() {
    let base_url = spawn_test_server().await;
    let (owner, _phrase) = AccountRootKeyPair::generate();
    let owner_client = AccountsClient::new(base_url.clone(), owner);
    let device = DeviceId("device-1".to_string());
    owner_client
        .add_device(device.clone(), device_public_key(1))
        .await
        .unwrap();

    // An attacker who holds a real, different account's key produces a
    // genuinely valid signature — just not one that hashes to the victim's
    // account id — and attempts to present it against the victim's real
    // revoke URL. The server must reject this by checking the *path's*
    // account id against whatever key the request actually claims, not
    // just "is this a well-formed signature for *some* account."
    let (attacker, _phrase2) = AccountRootKeyPair::generate();
    let path = format!(
        "/v1/accounts/{}/devices/device-1/revoke",
        owner_client.account_id()
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let attacker_pubkey_hex = encode_hex(&attacker.identity().public_key);
    // The attacker signs the message as if the URL's account id were their
    // own — the only message they're capable of producing a real
    // signature for.
    let msg = auth::signing_message("POST", &path, &attacker.identity().id.0, now, &[]);
    let forged_signature_hex = encode_hex(&attacker.sign(&msg));

    let raw = reqwest::Client::new();
    let resp = raw
        .post(format!("{base_url}{path}"))
        .header(auth::ACCOUNT_PUBKEY_HEADER, attacker_pubkey_hex)
        .header(auth::TIMESTAMP_HEADER, now.to_string())
        .header(auth::SIGNATURE_HEADER, forged_signature_hex)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    let listed = owner_client
        .list_devices(&owner_client.account_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        listed.devices.len(),
        1,
        "the forged revoke attempt must not have removed the real device"
    );
}

#[tokio::test]
async fn unauthenticated_recovery_blob_access_is_rejected() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let account_id = account_key.identity().id.0;

    let raw = reqwest::Client::new();
    let put_resp = raw
        .put(format!("{base_url}/v1/accounts/{account_id}/recovery-blob"))
        .body("sneaky-plaintext")
        .send()
        .await
        .unwrap();
    assert_eq!(put_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let get_resp = raw
        .get(format!("{base_url}/v1/accounts/{account_id}/recovery-blob"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oversized_recovery_blob_is_rejected() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let client = AccountsClient::new(base_url, account_key);

    let too_big = vec![0u8; ankai_accounts_server::db::MAX_RECOVERY_BLOB_BYTES + 1];
    let result = client.put_recovery_blob(too_big).await;
    assert!(
        result.is_err(),
        "an oversized recovery blob must be rejected"
    );

    assert_eq!(client.get_recovery_blob().await.unwrap(), None);
}

#[tokio::test]
async fn missing_recovery_blob_is_none_not_an_error() {
    let base_url = spawn_test_server().await;
    let (account_key, _phrase) = AccountRootKeyPair::generate();
    let client = AccountsClient::new(base_url, account_key);

    assert_eq!(client.get_recovery_blob().await.unwrap(), None);
}
