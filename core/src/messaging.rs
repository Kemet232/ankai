//! Real end-to-end-encrypted 1:1 messaging over `core::p2p`, persisted to
//! the encrypted local DB. PROGRESS.md's "Immediate next steps" candidates
//! 1 and 2 (MLS encryption, DB persistence) — the two things the original
//! plaintext/in-memory stub explicitly deferred.
//!
//! **What's real now:**
//! - **MLS encryption.** Per `docs/adr/0004-e2ee-stack.md`, a 1:1 DM is a
//!   2-member OpenMLS group (RFC 9420) — not a separate double-ratchet.
//!   [`encrypt_and_log_outgoing`]/[`decrypt_incoming`] drive OpenMLS's real
//!   `MlsGroup` API (`MlsGroup::new`, `add_members`, `merge_pending_commit`,
//!   `create_message`, `process_message`, `StagedWelcome`) — genuine group
//!   state, not a simulation. That state is persisted through
//!   `AnkaiMlsProvider` exactly like `identity.rs`'s signature keys and key
//!   packages: callers must call `provider.flush(db)` after either function
//!   returns `Ok`, same contract as the rest of this codebase's MLS code.
//! - **Persistence.** Sent/received plaintext (already decrypted — MLS
//!   ciphertext is never itself stored) is written to the `messages` table
//!   (migration 5), and which OpenMLS group id backs which peer is recorded
//!   in the `conversations` table. "Encrypted at rest" is satisfied by the
//!   whole DB being SQLCipher-encrypted already, same as `settings`/
//!   `communities` — this is not a second encryption layer.
//!
//! **Still deliberately not done — read before extending:**
//! - **Peer discovery beyond "paste an id you already know."** `client` can
//!   now *optionally* source a [`PeerInvite`]'s two fields (an
//!   `EndpointAddr` and a `KeyPackage`) from a live
//!   `ankai-directory-server` instead of a manually pasted blob — see
//!   "Optional: sourcing a PeerInvite from a directory server" below. That
//!   is still not real discovery: a user still has to already know, and
//!   paste, the exact `DeviceId` (a hex string) they want to reach. There is
//!   no search, no contact list, no presence, and no notion of "who do I
//!   know" — the directory only answers "does this exact id have anything
//!   published," same as `crate::directory`'s trait always described.
//! - **Multi-conversation UI.** [`list_messages`] returns this device's
//!   entire flat message history across every peer it's ever talked to,
//!   not scoped per-conversation — there's still no contact list/thread
//!   switcher, matching the original stub's "one peer at a time" scope.
//! - **Delivery guarantees / offline queueing / read receipts** — unchanged
//!   from the original stub.
//! - **Multi-device, safety numbers, key rotation, group (>2 member)
//!   channels** — ADR-0004 territory not attempted here.
//!
//! ## Group setup without a directory service
//!
//! ADR-0004 calls for "each device publishes `KeyPackage`s to a directory
//! ahead of time" so a sender can start a 2-member MLS group with an
//! offline peer. That directory doesn't exist (see `crate::directory`'s doc
//! comment — deliberately not wired in, no real server chosen yet). So,
//! matching exactly how the original stub already handed peers a pasted
//! `EndpointAddr` out-of-band, [`PeerInvite`] bundles a device's dialable
//! `EndpointAddr` *and* a freshly built `KeyPackage` into one blob a user
//! copies and hands to a peer through some other channel (chat app, QR
//! code, whatever) — the same paste-a-blob mechanism, just carrying one
//! more field. This was chosen over piggybacking the `KeyPackage` on the
//! P2P connection handshake itself because the handshake is a bare QUIC
//! dial (`iroh::Endpoint::connect`), with no protocol-level place to attach
//! extra data before a stream is open — extending it would mean inventing
//! new framing on top of iroh, whereas the invite blob reuses the exact
//! JSON-over-paste mechanism this module already had for `EndpointAddr`.
//!
//! Whoever pastes the other's `PeerInvite` first and sends becomes the MLS
//! group's creator: they call `MlsGroup::new` (self as sole member), then
//! `add_members` with the peer's pasted `KeyPackage`, producing a `Welcome`.
//! That `Welcome` is sent to the peer as its own message over
//! `core::p2p::P2pNode::send` (first), immediately followed by the actual
//! first application message (second) — two ordinary P2P sends, no new
//! wire framing needed, since OpenMLS's own `MlsMessageIn::extract()`
//! already tags a deserialized message as `Welcome` vs. `PublicMessage`/
//! `PrivateMessage`. The receiving peer's `accept_loop` callback branches on
//! that tag: a `Welcome` joins a brand-new group (nothing user-visible
//! yet); a protocol message decrypts a real application message. Once a
//! group exists for a peer (recorded in the `conversations` table, keyed by
//! [`peer_id_for`]'s stable hex encoding of their `EndpointId`), later
//! messages in either direction are just `create_message`/`process_message`
//! against the existing loaded group — no more `Welcome`s.
//!
//! This is a genuine, if manual, MLS first-contact flow — not a shortcut
//! around it. The manual part is entirely the out-of-band exchange (same
//! gap `EndpointAddr` sharing already had); the MLS mechanics themselves
//! are OpenMLS's real API end to end.
//!
//! ## Optional: sourcing a `PeerInvite` from a directory server
//!
//! `docs/adr/0008-identity-discovery-service.md` (Status: **Proposed**, not
//! Accepted) plus `server/directory/`'s reference implementation and its
//! `HttpDirectoryClient` now give a real, running alternative to the manual
//! paste above — but this module itself is unchanged by it. `client`
//! (`client/src/main.rs`), when a human has opted in by setting
//! `ANKAI_DIRECTORY_URL`, can call `HttpDirectoryClient::key_packages`/
//! `endpoint_addr` for a peer's pasted `DeviceId` and assemble the exact
//! same [`PeerInvite`] shape this module already accepts — the peer only
//! has to be told a short hex `DeviceId` instead of a whole blob. Nothing in
//! `encrypt_and_log_outgoing`/`parse_peer_invite`/the MLS group-setup path
//! above changes to support this: a directory-sourced `PeerInvite` and a
//! hand-pasted one are indistinguishable to this module by construction,
//! which is deliberate — the crypto/group logic was not forked to add this.
//!
//! `core` cannot depend on `HttpDirectoryClient` directly (it lives in
//! `server/directory`, which depends on `core` — the reverse would be
//! circular), which is why this glue lives in `client`, not here. See that
//! crate for the actual lookup code.
//!
//! This does **not** make peer discovery automatic. It is still opt-in
//! (unset `ANKAI_DIRECTORY_URL` and the client behaves exactly as before —
//! manual-paste-only, no network calls to any directory), still requires
//! already knowing a peer's exact `DeviceId` out-of-band (no search/contact
//! list/presence), and inherits every gap ADR-0008 documents and leaves
//! open rather than resolves: TOFU pubkey pinning (first publish for a
//! `DeviceId` wins that binding permanently, no recovery path), fully
//! unauthenticated `EndpointAddr` lookups (a real, undischarged tension
//! with `docs/threat-model.md`'s IP-address protection goal — anyone who
//! learns/guesses a `DeviceId` can look up that device's current network
//! address), `KeyPackage`s being consumed whole on lookup (a second lookup
//! of the same device returns nothing until it republishes), and no
//! automatic re-publish on `KeyPackage`/`EndpointAddr` rotation — `client`
//! publishes once at startup and never again, so a long-running peer's
//! published `EndpointAddr` goes stale after the server's 10-minute TTL
//! until the app is restarted. There is also no revocation UI: nothing lets
//! a device retract what it already published. None of this is new to this
//! integration — it is exactly what ADR-0008 already flagged as "still
//! risky / open" before this session wired a real caller up to it.

use iroh::{EndpointAddr, EndpointId};
use openmls::credentials::CredentialWithKey;
use openmls::framing::{MlsMessageBodyIn, MlsMessageIn, ProcessedMessageContent};
use openmls::group::{GroupId, MlsGroup, MlsGroupCreateConfig, MlsGroupJoinConfig, StagedWelcome};
use openmls::key_packages::KeyPackage;
use openmls::prelude::tls_codec::Deserialize as TlsDeserialize;
use openmls::prelude::OpenMlsProvider;
use openmls_basic_credential::SignatureKeyPair;
use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::error::Error;
use crate::identity::{Device, CIPHERSUITE};
use crate::mls_provider::AnkaiMlsProvider;
use crate::p2p::P2pNode;
use crate::util::{encode_hex, random_id};

/// A message received from a peer, already decrypted. `from` is only ever
/// an `EndpointId` — there is no display name/identity binding layered on
/// top of it yet (see module doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedMessage {
    pub from: EndpointId,
    pub text: String,
}

/// This module's stable string identifier for a peer/conversation: the hex
/// encoding of their iroh `EndpointId` (public key). Used as the primary
/// key for both the `conversations` and `messages` tables.
pub fn peer_id_for(id: EndpointId) -> String {
    encode_hex(id.as_bytes())
}

/// A shareable invite to start (or join) an MLS-encrypted 1:1 conversation
/// with this device: its dialable `EndpointAddr` plus a freshly built
/// `KeyPackage` (MLS's prekey equivalent). See the module doc comment's
/// "Group setup without a directory" section for why this replaces the
/// original stub's address-only sharing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PeerInvite {
    pub addr: EndpointAddr,
    pub key_package: KeyPackage,
}

/// Parses a peer's invite from the JSON string they shared out-of-band.
pub fn parse_peer_invite(pasted: &str) -> Result<PeerInvite, Error> {
    serde_json::from_str(pasted.trim())
        .map_err(|e| Error::Net(format!("failed to parse peer invite: {e}")))
}

/// Renders `invite` as the same JSON string `parse_peer_invite` accepts, so
/// a user can copy their own device's invite and hand it to a peer
/// out-of-band.
pub fn format_peer_invite(invite: &PeerInvite) -> Result<String, Error> {
    serde_json::to_string(invite)
        .map_err(|e| Error::Net(format!("failed to encode own peer invite: {e}")))
}

/// Encrypts `text` as a real MLS application message for the peer described
/// by `invite`, first creating a fresh 2-member MLS group with them (using
/// `invite.key_package`) if this device has never talked to that peer
/// before. Returns the ordered list of raw payloads the caller must send,
/// in order, to `invite.addr` over `core::p2p` (each as its own
/// `P2pNode::send` call): a serialized `Welcome` first, only present when a
/// new group was just created, then always the encrypted application
/// message.
///
/// `text` is persisted into the `messages` table as a sent message before
/// this returns `Ok`. Callers must call `provider.flush(db)` afterwards —
/// this always mutates MLS state (either a new group plus a ratcheted
/// application message, or just the latter).
pub fn encrypt_and_log_outgoing(
    db: &Db,
    provider: &AnkaiMlsProvider,
    device: &Device,
    invite: &PeerInvite,
    text: &str,
) -> Result<Vec<Vec<u8>>, Error> {
    let peer_id = peer_id_for(invite.addr.id);
    let signer = crate::identity::device_signer(device, provider)?;

    let mut payloads = Vec::new();

    let mut group = match conversation_group_id(db, &peer_id)? {
        Some(group_id) => load_group(provider, &group_id)?,
        None => {
            let (group, welcome_bytes) =
                create_group_and_add_peer(provider, device, &signer, invite.key_package.clone())?;
            set_conversation_group_id(db, &peer_id, group.group_id())?;
            payloads.push(welcome_bytes);
            group
        }
    };

    let ciphertext = group
        .create_message(provider, &signer, text.as_bytes())
        .map_err(|e| Error::Net(format!("failed to encrypt MLS application message: {e}")))?
        .to_bytes()
        .map_err(|e| Error::Net(format!("failed to serialize MLS application message: {e}")))?;
    payloads.push(ciphertext);

    save_message(db, &peer_id, Direction::Sent, text)?;

    Ok(payloads)
}

/// Handles one raw payload received from `from` over `core::p2p`: either an
/// MLS `Welcome` establishing a brand-new conversation with `from` (returns
/// `Ok(None)` — group setup only, nothing user-visible) or a real MLS
/// application message, which is decrypted, persisted as a received
/// message, and returned.
///
/// Mirrors `encrypt_and_log_outgoing`'s contract: callers must call
/// `provider.flush(db)` afterwards, since both branches mutate MLS state.
pub fn decrypt_incoming(
    db: &Db,
    provider: &AnkaiMlsProvider,
    from: EndpointId,
    bytes: Vec<u8>,
) -> Result<Option<ReceivedMessage>, Error> {
    let peer_id = peer_id_for(from);

    let message = MlsMessageIn::tls_deserialize_exact(bytes)
        .map_err(|e| Error::Net(format!("failed to parse incoming MLS message: {e}")))?;

    match message.extract() {
        MlsMessageBodyIn::Welcome(welcome) => {
            let join_config = MlsGroupJoinConfig::builder()
                .use_ratchet_tree_extension(true)
                .build();
            let staged = StagedWelcome::new_from_welcome(provider, &join_config, welcome, None)
                .map_err(|e| Error::Net(format!("failed to process MLS welcome: {e}")))?;
            let group = staged
                .into_group(provider)
                .map_err(|e| Error::Net(format!("failed to join MLS group: {e}")))?;
            set_conversation_group_id(db, &peer_id, group.group_id())?;
            Ok(None)
        }
        MlsMessageBodyIn::PrivateMessage(m) => {
            decrypt_application_message(db, provider, &peer_id, from, m.into())
        }
        MlsMessageBodyIn::PublicMessage(m) => {
            decrypt_application_message(db, provider, &peer_id, from, m.into())
        }
        MlsMessageBodyIn::GroupInfo(_) | MlsMessageBodyIn::KeyPackage(_) => Err(Error::Net(
            "received an MLS message type this module doesn't expect (GroupInfo/KeyPackage)"
                .to_string(),
        )),
    }
}

fn decrypt_application_message(
    db: &Db,
    provider: &AnkaiMlsProvider,
    peer_id: &str,
    from: EndpointId,
    protocol_message: openmls::framing::ProtocolMessage,
) -> Result<Option<ReceivedMessage>, Error> {
    let group_id = conversation_group_id(db, peer_id)?.ok_or_else(|| {
        Error::Net(
            "received an application message for a peer with no established MLS group".to_string(),
        )
    })?;
    let mut group = load_group(provider, &group_id)?;

    let processed = group
        .process_message(provider, protocol_message)
        .map_err(|e| Error::Net(format!("failed to process incoming MLS message: {e}")))?;

    match processed.into_content() {
        ProcessedMessageContent::ApplicationMessage(app_message) => {
            let text = String::from_utf8_lossy(&app_message.into_bytes()).into_owned();
            save_message(db, peer_id, Direction::Received, &text)?;
            Ok(Some(ReceivedMessage { from, text }))
        }
        // A commit/proposal from a would-be third member — this module only
        // ever creates 2-member groups and never adds anyone else, so this
        // shouldn't happen in practice; ignored rather than erroring since
        // it's not a decryption failure.
        ProcessedMessageContent::ProposalMessage(_)
        | ProcessedMessageContent::ExternalJoinProposalMessage(_)
        | ProcessedMessageContent::StagedCommitMessage(_) => Ok(None),
    }
}

fn create_group_and_add_peer(
    provider: &AnkaiMlsProvider,
    device: &Device,
    signer: &SignatureKeyPair,
    peer_key_package: KeyPackage,
) -> Result<(MlsGroup, Vec<u8>), Error> {
    let credential_with_key = CredentialWithKey {
        credential: device.credential.clone().into(),
        signature_key: device.signature_key.0.clone().into(),
    };
    let config = MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        // Embed the ratchet tree in the Welcome/GroupInfo itself: there's no
        // directory service to fetch it from separately (see module doc
        // comment), so the joiner needs everything in the Welcome alone.
        .use_ratchet_tree_extension(true)
        .build();

    let mut group = MlsGroup::new(provider, signer, &config, credential_with_key)
        .map_err(|e| Error::Net(format!("failed to create MLS group: {e}")))?;

    let (_commit, welcome, _group_info) = group
        .add_members(provider, signer, &[peer_key_package])
        .map_err(|e| Error::Net(format!("failed to add peer to MLS group: {e}")))?;

    // Only this device is a member so far (the peer joins via the Welcome
    // below), so there's no one else to send the commit message to.
    group
        .merge_pending_commit(provider)
        .map_err(|e| Error::Net(format!("failed to merge MLS commit: {e}")))?;

    let welcome_bytes = welcome
        .to_bytes()
        .map_err(|e| Error::Net(format!("failed to serialize MLS welcome: {e}")))?;

    Ok((group, welcome_bytes))
}

fn load_group(provider: &AnkaiMlsProvider, group_id: &GroupId) -> Result<MlsGroup, Error> {
    MlsGroup::load(provider.storage(), group_id)
        .map_err(|e| Error::Net(format!("failed to load MLS group state: {e}")))?
        .ok_or_else(|| Error::Net("no MLS group state found in storage for this peer".to_string()))
}

fn conversation_group_id(db: &Db, peer_id: &str) -> Result<Option<GroupId>, Error> {
    let bytes: Option<Vec<u8>> = db
        .connection()
        .query_row(
            "SELECT group_id FROM conversations WHERE peer_id = ?1",
            rusqlite::params![peer_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| Error::Db(format!("failed to read conversation for {peer_id:?}: {e}")))?;
    Ok(bytes.map(|b| GroupId::from_slice(&b)))
}

fn set_conversation_group_id(db: &Db, peer_id: &str, group_id: &GroupId) -> Result<(), Error> {
    db.connection()
        .execute(
            "INSERT INTO conversations (peer_id, group_id) VALUES (?1, ?2)
             ON CONFLICT(peer_id) DO UPDATE SET group_id = excluded.group_id",
            rusqlite::params![peer_id, group_id.as_slice()],
        )
        .map_err(|e| Error::Db(format!("failed to save conversation for {peer_id:?}: {e}")))?;
    Ok(())
}

/// Whether a persisted message was sent by this device or received from a
/// peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Sent,
    Received,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Sent => "sent",
            Direction::Received => "received",
        }
    }

    fn parse(s: &str) -> Result<Self, Error> {
        match s {
            "sent" => Ok(Direction::Sent),
            "received" => Ok(Direction::Received),
            other => Err(Error::Db(format!("corrupt message direction: {other:?}"))),
        }
    }
}

/// A message read back from the `messages` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    pub peer_id: String,
    pub direction: Direction,
    pub content: String,
}

fn save_message(db: &Db, peer_id: &str, direction: Direction, content: &str) -> Result<(), Error> {
    db.connection()
        .execute(
            "INSERT INTO messages (id, peer_id, direction, content) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![random_id(), peer_id, direction.as_str(), content],
        )
        .map_err(|e| Error::Db(format!("failed to save message: {e}")))?;
    Ok(())
}

/// This device's entire persisted message history across every peer it's
/// ever talked to, oldest first. Not scoped to a single conversation — see
/// module doc comment: there's still no multi-conversation UI to scope it
/// to.
pub fn list_messages(db: &Db) -> Result<Vec<StoredMessage>, Error> {
    let conn = db.connection();
    let mut stmt = conn
        .prepare("SELECT peer_id, direction, content FROM messages ORDER BY rowid ASC")
        .map_err(|e| Error::Db(format!("failed to prepare message list query: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            let peer_id: String = row.get(0)?;
            let direction: String = row.get(1)?;
            let content: String = row.get(2)?;
            Ok((peer_id, direction, content))
        })
        .map_err(|e| Error::Db(format!("failed to query messages: {e}")))?;

    rows.map(|row| {
        let (peer_id, direction, content) =
            row.map_err(|e| Error::Db(format!("failed to read message row: {e}")))?;
        Ok(StoredMessage {
            peer_id,
            direction: Direction::parse(&direction)?,
            content,
        })
    })
    .collect()
}

/// Sends `payload` to `addr` over `core::p2p`, returning once the peer's
/// `accept_loop` has received it and sent back its bare ack. See module doc
/// comment: `encrypt_and_log_outgoing` may return more than one payload for
/// a single logical message (a `Welcome` plus the application message) —
/// callers send each in order with its own call to this function.
pub async fn send_message(node: &P2pNode, addr: EndpointAddr, payload: &[u8]) -> Result<(), Error> {
    node.send(addr, payload).await?;
    Ok(())
}

/// Runs forever (until `node`'s endpoint is closed), calling `on_message`
/// with the raw bytes and sender `EndpointId` of every message a peer sends
/// to `node`. Thin adapter over [`P2pNode::accept_loop`] — decryption is
/// deliberately not done here; see `decrypt_incoming`, which callers should
/// invoke from wherever they keep `Db`/`AnkaiMlsProvider` (this function
/// stays free of both so `core::p2p`'s accept loop doesn't need to know
/// about MLS or persistence at all).
pub async fn receive_messages<F>(node: &P2pNode, mut on_message: F) -> Result<(), Error>
where
    F: FnMut(EndpointId, Vec<u8>),
{
    node.accept_loop(|from, bytes| {
        on_message(from, bytes);
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{create_key_package, load_or_create_device};
    use std::sync::{Arc, Mutex};

    /// Two in-process "devices" (each with their own DB + MLS provider +
    /// identity) form a real 2-member MLS group and exchange a real
    /// encrypted application message over an actual P2P connection —
    /// mirroring the rigor of `identity.rs`'s MLS tests.
    #[tokio::test]
    async fn two_devices_form_an_mls_group_and_exchange_a_real_encrypted_message() {
        let alice_db = Db::open_in_memory("correct horse battery staple").unwrap();
        let alice_provider = AnkaiMlsProvider::load(&alice_db).unwrap();
        let alice_device = load_or_create_device(&alice_db, &alice_provider).unwrap();
        alice_provider.flush(&alice_db).unwrap();

        let bob_db = Db::open_in_memory("correct horse battery staple").unwrap();
        let bob_provider = AnkaiMlsProvider::load(&bob_db).unwrap();
        let bob_device = load_or_create_device(&bob_db, &bob_provider).unwrap();
        bob_provider.flush(&bob_db).unwrap();

        let alice_node = P2pNode::bind().await.unwrap();
        let bob_node = P2pNode::bind().await.unwrap();
        let bob_addr = bob_node.addr();

        // Bob publishes an invite (his address + a real KeyPackage) the
        // same way a user would paste it to Alice out-of-band.
        let bob_key_package = create_key_package(&bob_device, &bob_provider).unwrap();
        bob_provider.flush(&bob_db).unwrap();
        let bob_invite = PeerInvite {
            addr: bob_addr.clone(),
            key_package: bob_key_package.key_package.unwrap(),
        };
        let pasted = format_peer_invite(&bob_invite).unwrap();
        let parsed_invite = parse_peer_invite(&pasted).unwrap();
        assert_eq!(parsed_invite.addr, bob_addr);

        // Bob's receive side: decrypts (or, for the Welcome, just joins)
        // whatever Alice sends and records real decrypted messages.
        let bob_received = Arc::new(Mutex::new(Vec::<ReceivedMessage>::new()));
        let bob_received_for_loop = bob_received.clone();

        let accept_task = tokio::spawn(async move {
            receive_messages(&bob_node, move |from, bytes| {
                match decrypt_incoming(&bob_db, &bob_provider, from, bytes) {
                    Ok(Some(msg)) => {
                        bob_provider.flush(&bob_db).unwrap();
                        bob_received_for_loop.lock().unwrap().push(msg);
                    }
                    Ok(None) => {
                        bob_provider.flush(&bob_db).unwrap();
                    }
                    Err(e) => panic!("bob failed to process incoming message: {e}"),
                }
            })
            .await
            .unwrap();
        });

        // Alice: first message to Bob creates the MLS group (using Bob's
        // pasted KeyPackage), producing a Welcome + the real encrypted
        // application message.
        let payloads = encrypt_and_log_outgoing(
            &alice_db,
            &alice_provider,
            &alice_device,
            &parsed_invite,
            "hello bob, this is real MLS",
        )
        .unwrap();
        alice_provider.flush(&alice_db).unwrap();
        assert_eq!(
            payloads.len(),
            2,
            "first contact should produce a Welcome plus the application message"
        );

        for payload in &payloads {
            send_message(&alice_node, bob_addr.clone(), payload)
                .await
                .unwrap();
        }

        let received = bob_received.lock().unwrap().clone();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].text, "hello bob, this is real MLS");
        assert_eq!(received[0].from, alice_node.addr().id);

        accept_task.abort();
        alice_node.close().await;

        // Alice's own sent message is persisted too.
        let alice_history = list_messages(&alice_db).unwrap();
        assert_eq!(alice_history.len(), 1);
        assert_eq!(alice_history[0].direction, Direction::Sent);
        assert_eq!(alice_history[0].content, "hello bob, this is real MLS");
    }

    #[test]
    fn parse_peer_invite_rejects_garbage() {
        assert!(parse_peer_invite("not json").is_err());
        assert!(parse_peer_invite("").is_err());
    }

    #[test]
    fn messages_persist_and_round_trip_through_list_messages() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        assert_eq!(list_messages(&db).unwrap(), Vec::new());

        save_message(&db, "peer-a", Direction::Sent, "hi").unwrap();
        save_message(&db, "peer-a", Direction::Received, "hey back").unwrap();

        let history = list_messages(&db).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].direction, Direction::Sent);
        assert_eq!(history[0].content, "hi");
        assert_eq!(history[1].direction, Direction::Received);
        assert_eq!(history[1].content, "hey back");
    }
}
