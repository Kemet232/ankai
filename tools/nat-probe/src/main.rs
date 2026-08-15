//! `nat-probe` — a standalone instrument for ADR-0003's spike 1: "NAT
//! traversal success-rate measurement." See
//! `docs/adr/0003-p2p-networking-stack.md`, "Spikes to run before this is
//! fully load-bearing," item 1.
//!
//! **What this is:** a minimal two-role iroh ping/connect tool. One side
//! runs `nat-probe listen`, prints its own dialable address, and waits; the
//! other side runs `nat-probe connect <that address>` (or pastes it on
//! stdin) and attempts to reach it. Once connected, both sides ask iroh's
//! own connection-introspection API (`Connection::paths()`) whether the
//! resulting path is a direct/hole-punched IP path or a relayed one, and the
//! connecting side also does one round of a trivial ping/echo to get a
//! real, application-level RTT number alongside iroh's own per-path RTT
//! estimate. Output is plain structured `stdout` lines (`key=value` pairs) —
//! no telemetry pipeline, no aggregation, no storage. The ADR's spike wants
//! signal, not infrastructure.
//!
//! **What this is NOT:** a completed measurement. Spike 1 calls for running
//! this "across a diverse beta cohort (mobile carrier-grade NAT, symmetric
//! home routers, corporate/campus networks, VPNs)" and comparing direct- vs
//! relay-fallback rates against the ADR's cited ~90%/~70% figures. That is
//! real-world data collection across networks this agent does not have
//! access to, and it has NOT been done as part of building this tool — see
//! `docs/adr/0003-p2p-networking-stack.md`'s spike-1 note (added alongside
//! this file) and `PROGRESS.md`. Running this tool once on localhost (see
//! this crate's own smoke test) proves the instrument works; it says
//! nothing about real-world NAT traversal rates, because loopback has no
//! NAT to traverse.
//!
//! **Why this doesn't reuse `ankai-core::p2p`:** that module deliberately
//! binds iroh's `Minimal` preset — no discovery, no relay, local-socket-only
//! — because it's Phase-1 scaffolding that must not assume any external
//! service is reachable (see its own module doc comment). This tool needs
//! the opposite: real hole-punching and relay fallback, which iroh only
//! attempts when configured with a relay-and-discovery-capable preset
//! (`iroh::endpoint::presets::N0` — n0.computer's default relay servers plus
//! DNS-based address lookup). Bolting that onto `p2p.rs` would silently
//! change production scaffolding's networking assumptions; keeping this
//! tool separate keeps that scaffolding honest about what it does and
//! doesn't contact. This crate does not depend on `ankai-core` at all —
//! there's nothing in `core` this tool needs, and pulling it in would drag
//! along OpenMLS/SQLCipher/keyring dependencies a NAT probe has no business
//! needing.
//!
//! **Usage:**
//! ```text
//! # terminal A
//! cargo run -p nat-probe -- listen
//! # prints its own endpoint id and a JSON address line; copy the address line
//!
//! # terminal B
//! cargo run -p nat-probe -- connect '<paste the address JSON here>'
//! # or, to avoid shell-quoting the JSON: cargo run -p nat-probe -- connect
//! # and paste the address line at the stdin prompt instead
//! ```

use std::io::BufRead;
use std::time::{Duration, Instant};

use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointAddr};

/// ALPN identifying this tool's trivial ping/echo protocol. Separate from
/// `ankai-core::p2p`'s scaffold ALPN — a `nat-probe` instance and an
/// `ankai-core::P2pNode` instance are not meant to talk to each other.
const ALPN: &[u8] = b"ankai/nat-probe/1";

/// The fixed ping payload the `connect` role sends and the `listen` role
/// echoes back verbatim. Content is unimportant; only its round trip is
/// measured.
const PING_PAYLOAD: &[u8] = b"ankai-nat-probe-ping";

/// How long to wait for the endpoint's home relay connection to come up
/// before giving up and proceeding anyway. Prevents `Endpoint::online()`
/// from hanging forever in an environment where the relay/DNS lookup is
/// unreachable (e.g. a sandboxed CI runner, or a genuinely offline dev
/// machine) — this tool should fail fast and say so, not hang silently.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait after a connection is established before taking the
/// "settled" path snapshot. iroh's hole-punch upgrade (relay-first, then
/// direct once punching succeeds) isn't instantaneous, so snapshotting
/// immediately after `connect()` returns would understate how often direct
/// paths are eventually reached.
const HOLE_PUNCH_GRACE_PERIOD: Duration = Duration::from_secs(3);

/// How long to wait for the connect/ping/close steps before giving up.
const OP_TIMEOUT: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("listen") => run_listen().await,
        Some("connect") => {
            let addr_arg = args.get(2).cloned();
            run_connect(addr_arg).await
        }
        _ => {
            eprintln!(
                "usage:\n  nat-probe listen\n  nat-probe connect [<peer address JSON>]\n\n\
                 If the peer address isn't given as an argument, `connect` reads one line from stdin."
            );
            Err("missing or unrecognized subcommand".to_string())
        }
    }
}

/// Binds an endpoint on iroh's `N0` preset: real DNS-based address lookup
/// plus n0.computer's default relay servers, so both hole-punching and
/// relay fallback are actually exercised — the whole point of this tool.
/// See the crate doc comment for why this differs from `ankai-core::p2p`'s
/// `Minimal` preset.
async fn bind_endpoint() -> Result<Endpoint, String> {
    Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| format!("failed to bind endpoint: {e}"))
}

/// Waits (bounded by [`ONLINE_TIMEOUT`]) for the endpoint's home relay
/// connection to come up, logging either way rather than hanging forever if
/// the relay/DNS lookup is unreachable from this network.
async fn wait_online(endpoint: &Endpoint) {
    match tokio::time::timeout(ONLINE_TIMEOUT, endpoint.online()).await {
        Ok(()) => println!("relay_status=connected"),
        Err(_) => println!(
            "relay_status=timed_out_after_{}s (continuing anyway; direct-only reachability \
             is still possible if the peer address includes usable IP candidates)",
            ONLINE_TIMEOUT.as_secs()
        ),
    }
}

async fn run_listen() -> Result<(), String> {
    let endpoint = bind_endpoint().await?;
    wait_online(&endpoint).await;

    let id = endpoint.id();
    let addr = endpoint.addr();
    let addr_json = serde_json::to_string(&addr)
        .map_err(|e| format!("failed to serialize own address: {e}"))?;

    println!("role=listen endpoint_id={id}");
    println!("copy the line below and pass it to the `connect` role:");
    println!("ANKAI-NAT-PROBE-ADDR: {addr_json}");
    println!("waiting for a connection (Ctrl-C to stop)...");

    loop {
        let Some(incoming) = endpoint.accept().await else {
            println!("event=endpoint_closed");
            return Ok(());
        };
        let accepting = match incoming.accept() {
            Ok(accepting) => accepting,
            Err(e) => {
                println!("event=incoming_rejected error={e}");
                continue;
            }
        };
        let conn = match accepting.await {
            Ok(conn) => conn,
            Err(e) => {
                println!("event=accept_failed error={e}");
                continue;
            }
        };

        let remote = conn.remote_id();
        println!("event=connection_accepted remote_id={remote}");
        for line in describe_paths(&conn, "on_accept") {
            println!("{line}");
        }

        match handle_echo(&conn).await {
            Ok(()) => println!("event=echo_completed remote_id={remote}"),
            Err(e) => println!("event=echo_failed remote_id={remote} error={e}"),
        }

        for line in describe_paths(&conn, "after_echo") {
            println!("{line}");
        }

        conn.closed().await;
        println!("event=connection_closed remote_id={remote}");
    }
}

/// Accepts a single bidirectional stream, reads it to completion, and
/// echoes the same bytes back — the `listen` side's half of the trivial
/// ping/echo protocol.
async fn handle_echo(conn: &Connection) -> Result<(), String> {
    let (mut send, mut recv) = tokio::time::timeout(OP_TIMEOUT, conn.accept_bi())
        .await
        .map_err(|_| "timed out waiting for a stream".to_string())?
        .map_err(|e| format!("failed to accept stream: {e}"))?;

    let received = tokio::time::timeout(OP_TIMEOUT, recv.read_to_end(64 * 1024))
        .await
        .map_err(|_| "timed out reading ping payload".to_string())?
        .map_err(|e| format!("failed to read ping payload: {e}"))?;

    tokio::time::timeout(OP_TIMEOUT, send.write_all(&received))
        .await
        .map_err(|_| "timed out echoing payload back".to_string())?
        .map_err(|e| format!("failed to echo payload: {e}"))?;

    send.finish()
        .map_err(|e| format!("failed to finish send stream: {e}"))?;

    Ok(())
}

async fn run_connect(addr_arg: Option<String>) -> Result<(), String> {
    let addr_json = match addr_arg {
        Some(arg) => arg,
        None => {
            println!("paste the peer's `ANKAI-NAT-PROBE-ADDR:` JSON, then press Enter:");
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(|e| format!("failed to read peer address from stdin: {e}"))?;
            line
        }
    };
    let peer_addr = parse_peer_addr(&addr_json)?;

    let endpoint = bind_endpoint().await?;
    wait_online(&endpoint).await;
    println!("role=connect endpoint_id={}", endpoint.id());

    let connect_start = Instant::now();
    let conn = tokio::time::timeout(OP_TIMEOUT, endpoint.connect(peer_addr, ALPN))
        .await
        .map_err(|_| "timed out connecting to peer".to_string())?
        .map_err(|e| format!("failed to connect: {e}"))?;
    println!(
        "event=connected remote_id={} connect_time_ms={:.1}",
        conn.remote_id(),
        connect_start.elapsed().as_secs_f64() * 1000.0
    );
    for line in describe_paths(&conn, "on_connect") {
        println!("{line}");
    }

    println!(
        "waiting {}s for iroh's hole-punch upgrade to settle before the ping...",
        HOLE_PUNCH_GRACE_PERIOD.as_secs()
    );
    tokio::time::sleep(HOLE_PUNCH_GRACE_PERIOD).await;
    for line in describe_paths(&conn, "after_grace_period") {
        println!("{line}");
    }

    let app_rtt = ping(&conn).await?;
    println!(
        "event=ping_completed app_rtt_ms={:.1}",
        app_rtt.as_secs_f64() * 1000.0
    );

    for line in describe_paths(&conn, "final") {
        println!("{line}");
    }
    println!("connection_type={}", connection_type(&conn));

    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok(())
}

/// Sends [`PING_PAYLOAD`] over a fresh bidirectional stream and waits for it
/// to be echoed back, returning the wall-clock round trip time. This is a
/// real application-level measurement (send → remote echo → receive), not
/// iroh's own transport-level RTT estimate — [`describe_paths`] reports that
/// one separately, from `Path::rtt()`.
async fn ping(conn: &Connection) -> Result<Duration, String> {
    let start = Instant::now();

    let (mut send, mut recv) = tokio::time::timeout(OP_TIMEOUT, conn.open_bi())
        .await
        .map_err(|_| "timed out opening ping stream".to_string())?
        .map_err(|e| format!("failed to open stream: {e}"))?;

    tokio::time::timeout(OP_TIMEOUT, send.write_all(PING_PAYLOAD))
        .await
        .map_err(|_| "timed out sending ping".to_string())?
        .map_err(|e| format!("failed to send ping: {e}"))?;
    send.finish()
        .map_err(|e| format!("failed to finish send stream: {e}"))?;

    let echoed = tokio::time::timeout(OP_TIMEOUT, recv.read_to_end(64 * 1024))
        .await
        .map_err(|_| "timed out waiting for echo".to_string())?
        .map_err(|e| format!("failed to read echo: {e}"))?;

    if echoed != PING_PAYLOAD {
        return Err(format!(
            "echo mismatch: sent {} bytes, got {} back",
            PING_PAYLOAD.len(),
            echoed.len()
        ));
    }

    Ok(start.elapsed())
}

/// Formats one structured line per currently-open path on `conn`, tagged
/// with `stage` so a reader can compare snapshots taken at different points
/// (right after connect, after the hole-punch grace period, after the
/// ping) to see whether/when the connection upgraded from relay to direct.
fn describe_paths(conn: &Connection, stage: &str) -> Vec<String> {
    let paths = conn.paths();
    if paths.is_empty() {
        return vec![format!(
            "path_snapshot stage={stage} remote_id={} paths=0",
            conn.remote_id()
        )];
    }
    paths
        .iter()
        .map(|path| {
            let kind = if path.is_ip() {
                "direct"
            } else if path.is_relay() {
                "relay"
            } else {
                "custom"
            };
            format!(
                "path_snapshot stage={stage} remote_id={} kind={kind} addr={} selected={} rtt_ms={:.1}",
                conn.remote_id(),
                path.remote_addr(),
                path.is_selected(),
                path.rtt().as_secs_f64() * 1000.0,
            )
        })
        .collect()
}

/// This is the headline number ADR-0003 spike 1 wants: did the connection
/// actually in use end up direct/hole-punched, or relayed? Based on which
/// path is currently *selected* for application data (not merely open —
/// iroh commonly keeps a relay path around alongside a direct one).
fn connection_type(conn: &Connection) -> &'static str {
    let paths = conn.paths();
    for path in paths.iter() {
        if path.is_selected() {
            return if path.is_ip() {
                "direct"
            } else if path.is_relay() {
                "relayed"
            } else {
                "custom"
            };
        }
    }
    "unknown"
}

/// Parses a peer address from either raw JSON (as printed after
/// `ANKAI-NAT-PROBE-ADDR:` by the `listen` role) or a full copy-pasted line
/// still carrying that prefix — trims the prefix if present so users don't
/// have to hand-edit what they copied.
fn parse_peer_addr(input: &str) -> Result<EndpointAddr, String> {
    let trimmed = input.trim();
    let json = trimmed
        .strip_prefix("ANKAI-NAT-PROBE-ADDR:")
        .map(str::trim)
        .unwrap_or(trimmed);
    if json.is_empty() {
        return Err("no peer address given (empty argument/stdin line)".to_string());
    }
    serde_json::from_str(json).map_err(|e| format!("failed to parse peer address: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real (validly-generated, not hand-typed) `EndpointAddr` with no
    /// network addresses attached — enough to exercise JSON round-tripping
    /// without contacting any network. Binding on the `Minimal` preset only
    /// generates a local key and opens a local socket; unlike `N0`, it
    /// never attempts to reach a relay, so this is safe to do inside a unit
    /// test.
    async fn sample_addr_json() -> String {
        let endpoint = Endpoint::builder(presets::Minimal)
            .bind()
            .await
            .expect("binding on the Minimal preset should never touch the network");
        let addr = EndpointAddr::new(endpoint.id());
        endpoint.close().await;
        serde_json::to_string(&addr).unwrap()
    }

    #[tokio::test]
    async fn parses_raw_json_address() {
        let json = sample_addr_json().await;
        let addr = parse_peer_addr(&json).unwrap();
        assert_eq!(addr.addrs.len(), 0);
    }

    #[tokio::test]
    async fn strips_the_printed_label_prefix_before_parsing() {
        let json = sample_addr_json().await;
        let labeled = format!("ANKAI-NAT-PROBE-ADDR: {json}\n");
        let addr = parse_peer_addr(&labeled).unwrap();
        assert_eq!(addr.addrs.len(), 0);
    }

    #[test]
    fn rejects_empty_input() {
        assert!(parse_peer_addr("   ").is_err());
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_peer_addr("not json").is_err());
    }
}
