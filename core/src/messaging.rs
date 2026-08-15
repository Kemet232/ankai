//! Minimal "send one message to one peer you already have an address for"
//! capability — PROGRESS.md's "Immediate next steps" candidate 1. Built
//! directly on `crate::p2p`'s `P2pNode`.
//!
//! **Deliberately a stub — read this before extending it.** This is not
//! real messaging. It does NOT implement:
//! - **Peer discovery.** There is no directory/lookup service (see
//!   `crate::p2p`'s doc comment for why), so there is no way to find a
//!   peer's address by name/id. The user must obtain a peer's `EndpointAddr`
//!   out-of-band (e.g. the peer copies theirs from their own client and
//!   sends it over some other channel) and paste it in. Correspondingly,
//!   this module only ever talks about one peer at a time — no contact
//!   list, no address book.
//! - **MLS/E2EE encryption.** ADR-0004 calls for messages to eventually be
//!   wrapped as MLS application messages before they're sent. This module
//!   sends and receives plain UTF-8 bytes, protected only by iroh's QUIC
//!   transport encryption (peer-authenticated point-to-point, but not the
//!   application-layer E2EE ANKAI's threat model requires before shipping
//!   real user messages).
//! - **Persistence.** Sent/received messages exist only in the caller's
//!   in-memory state for the lifetime of the process; nothing is written to
//!   `crate::db`. There's no conversation/thread model yet to persist, and
//!   persisting plaintext messages before they're actually encrypted at
//!   rest the way ADR-0004 requires would be a regression, not progress.
//! - **Delivery guarantees.** `send_message` either completes (the peer's
//!   `P2pNode::accept_loop` received the bytes and acked) or returns an
//!   `Err` — no retry, no offline queueing, no read receipts.
//!
//! **Address format:** a peer's `EndpointAddr` (their `EndpointId` plus any
//! locally known direct/relay addresses) serialized as JSON via `serde`.
//! `iroh_base::EndpointAddr` already derives `Serialize`/`Deserialize`, and
//! iroh 1.x doesn't ship a compact ticket/base32 string encoding for the
//! whole struct (only for the bare `EndpointId`/`PublicKey`, which alone
//! isn't enough to dial without a discovery service) — so JSON is genuinely
//! "whatever iroh gives us," not a homegrown format.

use iroh::{EndpointAddr, EndpointId};

use crate::error::Error;
use crate::p2p::P2pNode;

/// A message received from a peer. `from` is only ever an `EndpointId` —
/// there is no display name/identity binding layered on top of it yet (see
/// module doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedMessage {
    pub from: EndpointId,
    pub text: String,
}

/// Parses a peer's address from the JSON string they shared out-of-band
/// (see module doc comment for the format).
pub fn parse_peer_address(pasted: &str) -> Result<EndpointAddr, Error> {
    serde_json::from_str(pasted.trim())
        .map_err(|e| Error::Net(format!("failed to parse peer address: {e}")))
}

/// Renders `addr` as the same JSON string `parse_peer_address` accepts, so a
/// user can copy their own node's address and hand it to a peer out-of-band.
pub fn format_peer_address(addr: &EndpointAddr) -> Result<String, Error> {
    serde_json::to_string(addr)
        .map_err(|e| Error::Net(format!("failed to encode own peer address: {e}")))
}

/// Sends `text` to `addr` as a single plaintext message, returning once the
/// peer's `accept_loop` has received it and sent back its bare ack. See
/// module doc comment: no encryption beyond QUIC transport, no persistence,
/// no retry.
pub async fn send_message(node: &P2pNode, addr: EndpointAddr, text: &str) -> Result<(), Error> {
    node.send(addr, text.as_bytes()).await?;
    Ok(())
}

/// Runs forever (until `node`'s endpoint is closed), calling `on_message`
/// with a [`ReceivedMessage`] for every message a peer sends to `node`.
/// Thin adapter over [`P2pNode::accept_loop`] that decodes each connection's
/// bytes as a UTF-8 string (lossily — nothing yet guarantees peers only ever
/// send valid UTF-8). Intended to be spawned as a background task for the
/// lifetime of the app.
pub async fn receive_messages<F>(node: &P2pNode, mut on_message: F) -> Result<(), Error>
where
    F: FnMut(ReceivedMessage),
{
    node.accept_loop(|from, bytes| {
        on_message(ReceivedMessage {
            from,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        });
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn two_nodes_exchange_a_real_message_through_the_messaging_api() {
        let receiver = P2pNode::bind().await.unwrap();
        let receiver_addr = receiver.addr();

        let received = Arc::new(Mutex::new(Vec::<ReceivedMessage>::new()));
        let received_for_loop = received.clone();
        let accept_task = tokio::spawn(async move {
            receive_messages(&receiver, move |msg| {
                received_for_loop.lock().unwrap().push(msg);
            })
            .await
            .unwrap();
        });

        let sender = P2pNode::bind().await.unwrap();

        // Round-trip the address through the same JSON encoding a real user
        // would paste, rather than reusing the in-memory `EndpointAddr`
        // directly, so this test actually exercises `parse_peer_address`/
        // `format_peer_address`.
        let pasted = format_peer_address(&receiver_addr).unwrap();
        let parsed_addr = parse_peer_address(&pasted).unwrap();
        assert_eq!(parsed_addr, receiver_addr);

        send_message(&sender, parsed_addr, "hello from ankai messaging")
            .await
            .unwrap();

        // `send_message` only returns once the receive side's accept loop
        // has already invoked `on_message` (see `P2pNode::accept_loop`'s
        // doc comment), so this is guaranteed to be populated already.
        // Cloned out of a short-lived lock scope rather than held across the
        // `.await`s below.
        let messages = received.lock().unwrap().clone();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "hello from ankai messaging");
        assert_eq!(messages[0].from, sender.addr().id);

        accept_task.abort();
        sender.close().await;
    }

    #[test]
    fn parse_peer_address_rejects_garbage() {
        assert!(parse_peer_address("not json").is_err());
        assert!(parse_peer_address("").is_err());
    }
}
