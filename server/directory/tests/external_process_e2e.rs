//! Manual, ignored-by-default end-to-end check that the *exact compiled*
//! `ankai-directory-server` binary — not an in-process `axum::serve`
//! stand-in like `tests/client_lookup_e2e.rs` uses — works when run as a
//! genuinely separate OS process listening on a real TCP port, the way it
//! actually runs outside tests. Otherwise identical to
//! `client_lookup_e2e.rs`'s main scenario: device A publishes, device B
//! looks A up purely by `DeviceId` against the real subprocess over real
//! HTTP, and sends a real MLS-encrypted message that A decrypts.
//!
//! `#[ignore]`d by default so `cargo test --workspace`/CI doesn't depend on
//! grabbing a specific free TCP port or spawning a subprocess — run
//! explicitly with:
//!
//! ```text
//! cargo test -p ankai-directory-server --test external_process_e2e -- --ignored --nocapture
//! ```

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ankai_core::db::Db;
use ankai_core::directory::DirectoryService;
use ankai_core::identity::{
    create_key_package, load_or_create_device, Device, DeviceId, DEVICE_SIGNATURE_SCHEME,
};
use ankai_core::messaging::{
    decrypt_incoming, encrypt_and_log_outgoing, receive_messages, send_message, PeerInvite,
    ReceivedMessage,
};
use ankai_core::mls_provider::AnkaiMlsProvider;
use ankai_core::p2p::P2pNode;
use ankai_directory_server::HttpDirectoryClient;
use openmls_basic_credential::SignatureKeyPair;
use openmls_traits::OpenMlsProvider;

/// Kills the spawned server subprocess when the test ends (pass or fail) —
/// without this, a panicking assertion would leak a listening process.
struct ServerProcess {
    child: Child,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reserves a free local port by binding to it and immediately dropping the
/// listener — the standard (small-race-window) way to hand a specific port
/// to a subprocess without the subprocess itself reporting back which one
/// the OS picked (`main.rs` only echoes back whatever `ANKAI_DIRECTORY_ADDR`
/// string it was given, not the actual bound address).
fn reserve_free_local_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("should reserve an ephemeral port");
    listener.local_addr().unwrap().to_string()
}

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

async fn lookup_peer_invite_via_directory(
    looker: &HttpDirectoryClient,
    peer: &DeviceId,
) -> Option<PeerInvite> {
    let mut key_packages = looker
        .key_packages(peer)
        .await
        .expect("lookup should succeed");
    if key_packages.is_empty() {
        return None;
    }
    let key_package = key_packages.remove(0);
    let addr = looker
        .endpoint_addr(peer)
        .await
        .expect("lookup should succeed")?;
    Some(PeerInvite { addr, key_package })
}

#[tokio::test]
#[ignore = "spawns the real compiled ankai-directory-server binary as a subprocess; run explicitly"]
async fn real_compiled_server_binary_serves_a_real_cross_process_lookup() {
    let bind_addr = reserve_free_local_addr();
    let db_path = std::env::temp_dir().join(format!(
        "ankai-directory-external-e2e-{}.sqlite3",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);

    println!(
        "[harness] spawning real ankai-directory-server binary on {bind_addr}, db {db_path:?}"
    );
    let child = Command::new(env!("CARGO_BIN_EXE_ankai-directory-server"))
        .env("ANKAI_DIRECTORY_ADDR", &bind_addr)
        .env("ANKAI_DIRECTORY_DB", &db_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("should spawn the real compiled server binary");
    let server = ServerProcess { child };

    let base_url = format!("http://{bind_addr}");
    let mut ready = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(&bind_addr).is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        ready,
        "real server subprocess should start listening within 5s"
    );
    println!("[harness] real server subprocess is accepting connections at {base_url}");

    // --- Device A: publishes to the real subprocess's real server, then listens. ---
    let a = spin_up_installation();
    let a_node = P2pNode::bind().await.expect("A's node should bind");
    let a_addr = a_node.addr();

    let a_key_package = create_key_package(&a.device, &a.provider)
        .expect("A should build a real KeyPackage")
        .key_package
        .expect("create_key_package always returns Some");
    a.provider.flush(&a.db).expect("A's flush should succeed");

    let a_client = HttpDirectoryClient::new(base_url.clone(), a.device.id.clone(), a.signer);
    a_client
        .publish_key_package(&a.device.id, a_key_package.clone())
        .await
        .expect("A's real signed publish should succeed against the real subprocess");
    a_client
        .publish_endpoint_addr(&a.device.id, a_addr.clone())
        .await
        .expect("A's real signed publish should succeed against the real subprocess");
    println!("[harness] device A published KeyPackage + EndpointAddr to the real subprocess");

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

    // --- Device B: knows only A's DeviceId, looks everything else up
    // against the real subprocess over real HTTP. ---
    let b = spin_up_installation();
    let b_client = HttpDirectoryClient::new(base_url.clone(), b.device.id.clone(), b.signer);

    let invite = lookup_peer_invite_via_directory(&b_client, &a.device.id)
        .await
        .expect("B should find a real PeerInvite for A via the real subprocess");
    assert_eq!(invite.addr, a_addr);
    assert_eq!(invite.key_package, a_key_package);
    println!(
        "[harness] device B found A purely via DeviceId {} against the real subprocess",
        a.device.id.0
    );

    let b_node = P2pNode::bind().await.expect("B's node should bind");
    let payloads = encrypt_and_log_outgoing(
        &b.db,
        &b.provider,
        &b.device,
        &invite,
        "hello A, found you via the real subprocess directory server",
    )
    .expect("B should be able to encrypt a real MLS message to A");
    b.provider.flush(&b.db).expect("B's flush should succeed");

    for payload in &payloads {
        send_message(&b_node, invite.addr.clone(), payload)
            .await
            .expect("B's real P2P send to A should succeed");
    }

    let received = a_received.lock().unwrap().clone();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].text,
        "hello A, found you via the real subprocess directory server"
    );
    assert_eq!(received[0].from, b_node.addr().id);
    println!(
        "[harness] device A decrypted: \"{}\" (from B's real EndpointId) — full path verified against a real separate OS process",
        received[0].text
    );

    a_accept_task.abort();
    b_node.close().await;
    drop(server);
    let _ = std::fs::remove_file(&db_path);
}
