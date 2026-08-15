//! Generic P2P data transport, per `docs/adr/0003-p2p-networking-stack.md`'s
//! choice of iroh (QUIC-native, dial-by-public-key) as ANKAI's core P2P
//! layer for everything that isn't real-time voice/video (WebRTC handles
//! that, per the same ADR — not implemented here).
//!
//! **Phase 1 scope — read this before extending this module.** ADR-0003
//! itself is still `Proposed`, not settled, pending five real-prototype
//! spikes (see its "Spikes to run" section) — NAT-traversal success rates,
//! WebRTC/E2EE-through-SFU, group-call topology, an ANKAI Node reference
//! implementation, and QUIC-datagram netplay. This module exists to give
//! those spikes something to build on, not to implement any of them. It
//! deliberately does NOT implement:
//! - **Peer discovery** — how a caller learns a remote's `EndpointAddr` in
//!   the first place is ANKAI's centralized identity/discovery service
//!   (the "thin cloud" side), which doesn't exist yet. This module only
//!   covers what happens once you already have one.
//! - **Privacy-mode gating** — the ADR's hard requirement that direct
//!   connections to non-contacts must be opt-in, enforced by an
//!   application-layer connection-manager policy. That policy layer isn't
//!   built yet; nothing here decides whether a connection *should* happen.
//! - **Relay tiers, `iroh-blobs`, `iroh-gossip`** — all separate, unbuilt
//!   pieces of the same ADR.
//! - **Discovery/relay network services** (`iroh`'s `N0` preset, which
//!   talks to n0.computer's DNS/relay infrastructure) — this module uses
//!   the `Minimal` preset instead, which only binds a local socket and
//!   sets a crypto provider. It makes no assumptions about, and depends on
//!   no availability of, any external discovery/relay service. Swapping in
//!   `N0` (or ANKAI's own relay) is exactly the kind of thing the NAT-
//!   traversal and ANKAI Node spikes above need to validate first.
//!
//! iroh usage is confined to this module so the rest of `core` doesn't
//! depend on iroh's types directly, per the ADR's call for an internal,
//! swappable P2P-layer boundary (in case iroh needs replacing later, per
//! the ADR's own ecosystem-maturity risk note).

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, EndpointId};

use crate::error::Error;

/// ALPN identifying ANKAI's (currently placeholder) generic-data protocol.
/// Real protocol framing/versioning belongs to whichever traffic type from
/// ADR-0003's traffic-type table lands first on top of this; this
/// scaffolding only needs *a* fixed string both sides agree on so iroh's
/// handshake doesn't reject the connection.
const ALPN: &[u8] = b"ankai/scaffold/0";

/// A bound iroh endpoint — an ANKAI node's dial-by-public-key address on
/// the generic P2P data transport.
pub struct P2pNode {
    endpoint: Endpoint,
}

impl P2pNode {
    /// Binds a new endpoint, ready to accept connections on ANKAI's
    /// scaffold ALPN and to dial other endpoints. Local-only: no
    /// discovery/relay service is contacted — see this module's doc
    /// comment.
    pub async fn bind() -> Result<Self, Error> {
        let endpoint = Endpoint::builder(presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| Error::Net(format!("failed to bind P2P endpoint: {e}")))?;
        Ok(Self { endpoint })
    }

    /// This node's dialable address (public key plus any locally known
    /// direct network paths). Handing this to a peer out-of-band
    /// (discovery — out of scope here) is how they'd be able to call
    /// [`P2pNode::send`] against this node.
    pub fn addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// Dials `addr` and sends `message` over a single bidirectional QUIC
    /// stream, returning whatever bytes the remote sends back before it
    /// finishes its send side. Phase-1 scaffolding only — no framing beyond
    /// "read until the response stream closes."
    pub async fn send(&self, addr: EndpointAddr, message: &[u8]) -> Result<Vec<u8>, Error> {
        let conn = self
            .endpoint
            .connect(addr, ALPN)
            .await
            .map_err(|e| Error::Net(format!("failed to connect to peer: {e}")))?;

        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| Error::Net(format!("failed to open stream: {e}")))?;

        send.write_all(message)
            .await
            .map_err(|e| Error::Net(format!("failed to send message: {e}")))?;
        send.finish()
            .map_err(|e| Error::Net(format!("failed to finish send stream: {e}")))?;

        let response = recv
            .read_to_end(64 * 1024)
            .await
            .map_err(|e| Error::Net(format!("failed to read response: {e}")))?;

        conn.close(0u32.into(), b"done");
        Ok(response)
    }

    /// Accepts a single incoming connection and echoes back whatever bytes
    /// it receives on the first stream, then returns. Phase-1 scaffolding
    /// only — a real accept loop, ALPN-based protocol dispatch, and
    /// multi-connection handling all belong to whatever replaces this once
    /// a real traffic type lands on top (see the module doc comment).
    pub async fn accept_and_echo_once(&self) -> Result<(), Error> {
        let incoming =
            self.endpoint.accept().await.ok_or_else(|| {
                Error::Net("endpoint closed before accepting a connection".into())
            })?;

        let conn = incoming
            .await
            .map_err(|e| Error::Net(format!("failed to accept connection: {e}")))?;

        let (mut send, mut recv) = conn
            .accept_bi()
            .await
            .map_err(|e| Error::Net(format!("failed to accept stream: {e}")))?;

        tokio::io::copy(&mut recv, &mut send)
            .await
            .map_err(|e| Error::Net(format!("failed to echo stream: {e}")))?;

        send.finish()
            .map_err(|e| Error::Net(format!("failed to finish send stream: {e}")))?;

        // Wait for the remote to close the connection (it does so once it's
        // received the full echoed response) before returning. Skipping
        // this races a caller's `P2pNode::close` against the remote still
        // draining the stream, which can sever the connection mid-read on
        // their end ("connection lost") instead of a clean close.
        conn.closed().await;

        Ok(())
    }

    /// Runs an accept loop that keeps going until the endpoint is closed
    /// (`accept()` returns `None`): for each incoming connection, reads
    /// whatever bytes arrive on its first bidirectional stream, invokes
    /// `on_message` with the sender's `EndpointId` and those bytes, then
    /// finishes its own send side with no bytes (a bare ack, not an echo —
    /// unlike `accept_and_echo_once`) and waits for the peer to close before
    /// accepting the next connection.
    ///
    /// This extends `accept_and_echo_once`'s explicitly one-shot scaffolding
    /// into something a caller can run for the app's whole lifetime — added
    /// for `crate::messaging`'s receive side, which needs to keep listening
    /// rather than handle a single connection and return. Still Phase 1
    /// scope per this module's doc comment: connections are handled one at a
    /// time (no concurrency across connections), only the first stream on
    /// each connection is read, and there's still only one ALPN/protocol. A
    /// connection that fails partway through (bad handshake, stream error,
    /// ...) is silently skipped rather than ending the whole loop or being
    /// reported to `on_message` — there's no logging/tracing story in this
    /// codebase yet to report it through; a real implementation would want
    /// one.
    pub async fn accept_loop<F>(&self, mut on_message: F) -> Result<(), Error>
    where
        F: FnMut(EndpointId, Vec<u8>),
    {
        loop {
            let Some(incoming) = self.endpoint.accept().await else {
                return Ok(());
            };

            let Ok(conn) = incoming.await else {
                continue;
            };

            let remote = conn.remote_id();

            let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                continue;
            };

            let Ok(message) = recv.read_to_end(64 * 1024).await else {
                continue;
            };

            on_message(remote, message);

            if send.finish().is_err() {
                continue;
            }
            conn.closed().await;
        }
    }

    /// Shuts the endpoint down, waiting for queued close messages to be
    /// sent so peers see a clean disconnect rather than a timeout.
    pub async fn close(self) {
        self.endpoint.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn two_nodes_exchange_a_message_over_a_direct_quic_stream() {
        let accepting = P2pNode::bind().await.unwrap();
        let accepting_addr = accepting.addr();

        let accept_task = tokio::spawn(async move {
            accepting.accept_and_echo_once().await.unwrap();
            accepting
        });

        let connecting = P2pNode::bind().await.unwrap();
        let response = connecting
            .send(accepting_addr, b"hello from ankai")
            .await
            .unwrap();
        assert_eq!(response, b"hello from ankai");

        let accepting = accept_task.await.unwrap();
        accepting.close().await;
        connecting.close().await;
    }

    #[tokio::test]
    async fn accept_loop_receives_multiple_messages_in_order() {
        use std::sync::{Arc, Mutex};

        let accepting = P2pNode::bind().await.unwrap();
        let accepting_addr = accepting.addr();

        let received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let received_for_loop = received.clone();
        let accept_task = tokio::spawn(async move {
            accepting
                .accept_loop(move |_from, bytes| {
                    received_for_loop.lock().unwrap().push(bytes);
                })
                .await
                .unwrap();
        });

        let connecting = P2pNode::bind().await.unwrap();
        // Each `send` call is its own connection; `send` only returns once
        // it has read the accept side's ack, which `accept_loop` only sends
        // after invoking `on_message` — so by the time these two calls
        // return, both messages are guaranteed to already be in `received`.
        connecting
            .send(accepting_addr.clone(), b"first")
            .await
            .unwrap();
        connecting.send(accepting_addr, b"second").await.unwrap();

        assert_eq!(
            *received.lock().unwrap(),
            vec![b"first".to_vec(), b"second".to_vec()]
        );

        // `accept_loop` only returns once its endpoint closes, and it's
        // borrowed for the duration of `accept_task` — aborting is the
        // simplest clean-enough teardown for a test (dropping the endpoint
        // still closes the underlying socket).
        accept_task.abort();
        connecting.close().await;
    }
}
