//! `webrtc-audio-probe` — ADR-0003 spike 2: "WebRTC audio/E2EE prototype."
//! See `docs/adr/0003-p2p-networking-stack.md`, "Spikes to run before this is
//! fully load-bearing," item 2.
//!
//! **What this is:** a real, working two-process `webrtc-rs` v0.20 voice-call
//! prototype. Both processes build a real `RTCPeerConnection` (real ICE, real
//! DTLS-SRTP), exchange a manual copy-pasted SDP offer/answer (there is no
//! signalling service to carry this yet — same "no signalling server, do it
//! by hand" pragmatism `tools/nat-probe` uses for peer addresses, per
//! ADR-0003's own note that signalling can later ride an iroh stream or the
//! central server, neither wired up today), and once connected each side
//! encodes a synthetic sine tone to real Opus, sends it as real RTP over the
//! real DTLS-SRTP-secured connection, and decodes what the *other* side sent
//! back — then checks programmatically (RMS energy + a zero-crossing
//! frequency estimate) that the decoded audio actually resembles the known
//! input, not silence or garbage.
//!
//! **What this proves:** the mechanism works end-to-end — connection setup,
//! DTLS-SRTP encryption actually active (confirmed via `webrtc-rs`'s own
//! `get_stats()`, not assumed), real Opus-encoded frames flowing over real
//! RTP, and correct decode on the far end. Verified by repeated two-process
//! runs on this session's macOS box: both sides reach
//! `RTCPeerConnectionState::Connected`, `get_stats()` reports
//! `dtls_state=Connected`/`srtp_cipher=SRTP_AEAD_AES_128_GCM`, and both
//! sides' `audio_verification` line reports `result=PASS` (decoded tone's
//! estimated frequency within `FREQUENCY_TOLERANCE_HZ` of the known input,
//! RMS above the silence floor). See `docs/adr/0003-p2p-networking-stack.md`,
//! "Spike 2 status," for the full run evidence.
//!
//! **A pitfall if you relay the SDP by hand through a shell:** the printed
//! `ANKAI-WEBRTC-PROBE-OFFER`/`-ANSWER` lines contain literal `\r\n`
//! sequences (SDP's line-ending convention, JSON-escaped). If you pipe a
//! pasted line into the peer process's stdin using `zsh`'s `echo` builtin
//! (which interprets backslash escapes by default, unlike `printf`), it will
//! silently rewrite those into real carriage-return/newline bytes and
//! truncate/corrupt the SDP — this can manifest as both sides reaching
//! `Connecting` but neither ever reaching `Connected` (ICE succeeds on the
//! surviving candidate lines, DTLS then fails on a corrupted fingerprint
//! further down the SDP), which looks exactly like a real connectivity bug
//! but isn't one. Use `printf '%s\n' "$LINE"` (or a real terminal paste, not
//! a shell variable round-trip) instead.
//!
//! **What this does NOT prove, and does not claim to:**
//!
//! - **Subjective audio quality.** Whether `webrtc-rs`'s AEC/noise
//!   suppression is *good enough* on ANKAI's target desktop OSes is a
//!   judgment call for a human listening on real hardware in a real acoustic
//!   environment — an agent has no ears and a sine-wave loopback test proves
//!   nothing about how a real voice, in a real room, with real background
//!   noise, actually sounds. See `PROGRESS.md` for the same bar ADR-0002's
//!   glass/blur and accessibility spikes were held to.
//! - **Frame-level E2E encryption through an SFU.** Spike 2 also asks for
//!   frame-level E2E encryption (an Insertable-Streams-equivalent) layered on
//!   top of DTLS-SRTP so a semi-trusted SFU can forward media it cannot
//!   decrypt. This tool does not attempt that — there is no SFU to forward
//!   through (this is a direct two-peer test, no relay), and building one is
//!   separate, harder work. Still entirely unstarted.
//! - **Real microphone capture.** This process's environment has no way to
//!   record from a real microphone in an automated, non-interactive session
//!   (no `arecord`/`parecord`/`sox`/`ffmpeg` available, no OS mic-permission
//!   flow to grant without a human present) — see this crate's own `--help`
//!   output and the session's PROGRESS.md entry for the full reasoning. A
//!   synthetic sine tone through the exact same `webrtc-rs` audio-track API a
//!   real mic capture would use is equally valid for proving the *pipeline*
//!   works; it says nothing about mic capture quality specifically, which
//!   isn't part of this spike anyway (ADR-0003 spike 2 is about the
//!   send/decode/AEC/NS pipeline, not capture).
//! - **Multi-OS validation.** Only run/verified on macOS so far (see this
//!   session's smoke test). Windows/Linux are unstarted.
//!
//! **Why this doesn't reuse anything from `core`/`client`:** there is
//! nothing in `core` this tool needs (no MLS, no DB, no identity), and this
//! is deliberately standalone spike tooling outside the shipping crates —
//! same shape as `tools/nat-probe`.
//!
//! **Usage:**
//! ```text
//! # terminal A
//! cargo run -p webrtc-audio-probe -- offer
//! # prints an ANKAI-WEBRTC-PROBE-OFFER: line; copy it
//!
//! # terminal B
//! cargo run -p webrtc-audio-probe -- answer
//! # paste the offer line when prompted; it prints an
//! # ANKAI-WEBRTC-PROBE-ANSWER: line back
//!
//! # terminal A
//! # paste the answer line back when prompted
//! ```
//! Both sides then wait for the connection to reach `Connected` (real ICE +
//! DTLS handshake), send a few seconds of a synthetic tone to each other over
//! real Opus/RTP, decode what they receive, and print a structured report:
//! transport encryption state/ciphers, RTP packet counts, and whether the
//! decoded audio matches the known tone the peer was sending.

use std::io::BufRead;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use opus::{Application, Channels as OpusChannels, Decoder as OpusDecoder, Encoder as OpusEncoder};
use rtc::interceptor::Registry;
use rtc::media::Sample;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::{MediaEngine, MIME_TYPE_OPUS};
use rtc::peer_connection::configuration::RTCConfigurationBuilder;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RtpCodecKind,
};
use rtc::statistics::StatsSelector;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
    RTCPeerConnectionState,
};
use webrtc::runtime::{channel, Mutex as RtMutex, Runtime, Sender};

/// Opus operates at 48kHz internally regardless of the "native" source rate;
/// using it directly avoids a resampling step this probe has no need for.
const SAMPLE_RATE: u32 = 48_000;
const OPUS_CHANNELS: OpusChannels = OpusChannels::Stereo;
/// Matches `OPUS_CHANNELS` — the actual channel *count*, for buffer sizing.
const CHANNEL_COUNT: usize = 2;
/// Standard 20ms Opus frame at 48kHz.
const FRAME_MS: u64 = 20;
const FRAME_SAMPLES_PER_CHANNEL: usize = (SAMPLE_RATE as u64 * FRAME_MS / 1000) as usize;
/// Large enough for any Opus frame size this probe could ever decode (Opus
/// supports up to 120ms frames at 48kHz = 5760 samples/channel); using a
/// fixed, generous buffer avoids needing to know the frame size up front.
const MAX_DECODE_SAMPLES_PER_CHANNEL: usize = 5760;
/// Arbitrary but fixed dynamic payload type, agreed by both sides simply by
/// both being this same binary — no negotiation needed for a two-process
/// spike tool.
const OPUS_PAYLOAD_TYPE: u8 = 111;
/// How many seconds of tone each side sends once connected.
const SEND_SECONDS: u64 = 3;
/// Extra time to let the last few frames arrive/decode after sending stops,
/// before taking the final measurement.
const DRAIN_GRACE_PERIOD: Duration = Duration::from_millis(500);
/// How long to wait for the peer connection to reach `Connected` before
/// giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for ICE gathering to finish before giving up (this is a
/// localhost/LAN test — gathering host candidates should be near-instant).
const GATHER_TIMEOUT: Duration = Duration::from_secs(10);

/// The offer side sends this tone; the answer side checks its *received*
/// audio against it.
const OFFER_TONE_HZ: f64 = 440.0; // A4
/// The answer side sends this tone; the offer side checks its *received*
/// audio against it. Deliberately different from the offer's tone so a
/// side's own receive buffer can't accidentally pass by matching what it
/// sent rather than what it heard back.
const ANSWER_TONE_HZ: f64 = 660.0; // E5

const TONE_AMPLITUDE: f32 = 0.3;
/// How far off the estimated dominant frequency is allowed to be from the
/// known input and still count as "matches" — generous, because a
/// zero-crossing estimate over a short, lossily-encoded buffer is a coarse
/// instrument, not a spectrum analyzer. This only needs to distinguish "real
/// tone decoded" from "silence or noise," not measure Opus fidelity.
const FREQUENCY_TOLERANCE_HZ: f64 = 40.0;
/// RMS floor a decoded buffer must clear to count as "not silence."
const RMS_SILENCE_FLOOR: f64 = 0.02;

#[derive(Clone, Copy)]
enum Role {
    Offer,
    Answer,
}

impl Role {
    fn send_tone_hz(self) -> f64 {
        match self {
            Role::Offer => OFFER_TONE_HZ,
            Role::Answer => ANSWER_TONE_HZ,
        }
    }

    fn expect_receive_tone_hz(self) -> f64 {
        match self {
            Role::Offer => ANSWER_TONE_HZ,
            Role::Answer => OFFER_TONE_HZ,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Role::Offer => "offer",
            Role::Answer => "answer",
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let role = match args.get(1).map(String::as_str) {
        Some("offer") => Role::Offer,
        Some("answer") => Role::Answer,
        _ => {
            eprintln!(
                "usage:\n  webrtc-audio-probe offer\n  webrtc-audio-probe answer\n\n\
                 `offer` prints an SDP offer line, then waits on stdin for the pasted answer.\n\
                 `answer` reads an SDP offer line (as an argument, or pasted on stdin), then \
                 prints an SDP answer line.\n\n\
                 No microphone is used — both sides send a synthetic sine tone (offer: \
                 {OFFER_TONE_HZ}Hz, answer: {ANSWER_TONE_HZ}Hz) through a real Opus/RTP/DTLS-SRTP \
                 pipeline and verify what they decode from the other side. See this binary's \
                 module doc comment (`tools/webrtc-audio-probe/src/main.rs`) for what this does \
                 and does not prove."
            );
            return Err("missing or unrecognized subcommand".to_string());
        }
    };

    run(role).await
}

async fn run(role: Role) -> Result<(), String> {
    let runtime = webrtc::runtime::default_runtime()
        .ok_or("no runtime feature enabled (expected runtime-tokio)")?;

    let mut media_engine = MediaEngine::default();
    let opus_codec = RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: SAMPLE_RATE,
            channels: CHANNEL_COUNT as u16,
            sdp_fmtp_line: "".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: OPUS_PAYLOAD_TYPE,
    };
    media_engine
        .register_codec(opus_codec.clone(), RtpCodecKind::Audio)
        .map_err(|e| format!("failed to register Opus codec: {e}"))?;

    let registry = register_default_interceptors(Registry::new(), &mut media_engine)
        .map_err(|e| format!("failed to register default interceptors: {e}"))?;

    let config = RTCConfigurationBuilder::new().build();

    let (gather_tx, mut gather_rx) = channel::<()>(1);
    let (connected_tx, mut connected_rx) = channel::<()>(1);
    let received_pcm: Arc<RtMutex<Vec<f32>>> = Arc::new(RtMutex::new(Vec::new()));
    let rtp_received_count = Arc::new(AtomicUsize::new(0));

    let handler = Arc::new(Handler {
        gather_tx,
        connected_tx,
        received_pcm: received_pcm.clone(),
        rtp_received_count: rtp_received_count.clone(),
        runtime: runtime.clone(),
    });

    let pc: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_configuration(config)
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_handler(handler)
            .with_runtime(runtime.clone())
            .with_udp_addrs(vec!["127.0.0.1:0".to_string()])
            .build()
            .await
            .map_err(|e| format!("failed to build peer connection: {e}"))?,
    );

    let ssrc = rand::random::<u32>();
    let track = Arc::new(
        TrackLocalStaticSample::new(MediaStreamTrack::new(
            "ankai-webrtc-audio-probe-stream".to_string(),
            "ankai-webrtc-audio-probe-track".to_string(),
            format!("ankai-webrtc-audio-probe-{}", role.label()),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec: opus_codec.rtp_codec.clone(),
                ..Default::default()
            }],
        ))
        .map_err(|e| format!("failed to build local audio track: {e}"))?,
    );

    pc.add_track(track.clone() as Arc<dyn TrackLocal>)
        .await
        .map_err(|e| format!("failed to add local audio track: {e}"))?;

    println!("role={} local_ssrc={ssrc}", role.label());

    match role {
        Role::Offer => {
            let offer = pc
                .create_offer(None)
                .await
                .map_err(|e| format!("failed to create offer: {e}"))?;
            pc.set_local_description(offer)
                .await
                .map_err(|e| format!("failed to set local description: {e}"))?;
            wait_gather_complete(&mut gather_rx).await?;
            print_local_description(pc.as_ref(), "OFFER").await?;

            println!("paste the peer's ANKAI-WEBRTC-PROBE-ANSWER: line, then press Enter:");
            let answer = read_sdp_line("ANKAI-WEBRTC-PROBE-ANSWER:")?;
            pc.set_remote_description(answer)
                .await
                .map_err(|e| format!("failed to set remote description: {e}"))?;
        }
        Role::Answer => {
            println!("paste the peer's ANKAI-WEBRTC-PROBE-OFFER: line, then press Enter:");
            let offer = read_sdp_line("ANKAI-WEBRTC-PROBE-OFFER:")?;
            pc.set_remote_description(offer)
                .await
                .map_err(|e| format!("failed to set remote description: {e}"))?;
            let answer = pc
                .create_answer(None)
                .await
                .map_err(|e| format!("failed to create answer: {e}"))?;
            pc.set_local_description(answer)
                .await
                .map_err(|e| format!("failed to set local description: {e}"))?;
            wait_gather_complete(&mut gather_rx).await?;
            print_local_description(pc.as_ref(), "ANSWER").await?;
        }
    }

    println!(
        "waiting up to {}s for the connection to reach Connected...",
        CONNECT_TIMEOUT.as_secs()
    );
    tokio::time::timeout(CONNECT_TIMEOUT, connected_rx.recv())
        .await
        .map_err(|_| "timed out waiting for peer connection state Connected".to_string())?
        .ok_or("connected-state channel closed unexpectedly")?;
    println!("event=peer_connection_connected");

    // Send our tone now that DTLS-SRTP is confirmed established (reaching
    // `Connected` requires both ICE and DTLS to be up) rather than during
    // negotiation, so every frame we send is provably going out over the
    // encrypted transport, not racing its setup.
    send_tone(&track, ssrc, role.send_tone_hz()).await?;

    println!(
        "sent {SEND_SECONDS}s of a {}Hz tone; waiting {}ms for the last frames to arrive/decode...",
        role.send_tone_hz(),
        DRAIN_GRACE_PERIOD.as_millis()
    );
    tokio::time::sleep(DRAIN_GRACE_PERIOD).await;

    report_transport_stats(pc.as_ref()).await;
    report_audio_verification(&received_pcm, rtp_received_count.as_ref(), role).await;

    pc.close()
        .await
        .map_err(|e| format!("failed to close peer connection: {e}"))?;
    println!("event=closed");

    Ok(())
}

async fn wait_gather_complete(rx: &mut webrtc::runtime::Receiver<()>) -> Result<(), String> {
    tokio::time::timeout(GATHER_TIMEOUT, rx.recv())
        .await
        .map_err(|_| "timed out waiting for ICE gathering to complete".to_string())?
        .ok_or("gather-complete channel closed unexpectedly".to_string())?;
    Ok(())
}

async fn print_local_description(pc: &dyn PeerConnection, kind: &str) -> Result<(), String> {
    let desc = pc
        .local_description()
        .await
        .ok_or_else(|| format!("no local description available after {kind} creation"))?;
    let json = serde_json::to_string(&desc)
        .map_err(|e| format!("failed to serialize local description: {e}"))?;
    println!("ANKAI-WEBRTC-PROBE-{kind}: {json}");
    Ok(())
}

/// Reads one line from either the second CLI argument or stdin (mirroring
/// `tools/nat-probe`'s `parse_peer_addr` convention: accept either the raw
/// pasted line, or the same line still carrying its printed label prefix).
fn read_sdp_line(expected_prefix: &str) -> Result<RTCSessionDescription, String> {
    let args: Vec<String> = std::env::args().collect();
    let raw = match args.get(2) {
        Some(arg) => arg.clone(),
        None => {
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(|e| format!("failed to read SDP line from stdin: {e}"))?;
            line
        }
    };
    let trimmed = raw.trim();
    let json = trimmed
        .strip_prefix(expected_prefix)
        .map(str::trim)
        .unwrap_or(trimmed);
    if json.is_empty() {
        return Err("no SDP given (empty argument/stdin line)".to_string());
    }
    serde_json::from_str(json).map_err(|e| format!("failed to parse SDP: {e}"))
}

/// Generates and sends [`SEND_SECONDS`] of a sine tone at `freq_hz`, real
/// Opus-encoded, over `track`.
async fn send_tone(
    track: &Arc<TrackLocalStaticSample>,
    ssrc: u32,
    freq_hz: f64,
) -> Result<(), String> {
    let mut encoder = OpusEncoder::new(SAMPLE_RATE, OPUS_CHANNELS, Application::Voip)
        .map_err(|e| format!("failed to create Opus encoder: {e}"))?;

    let frame_count = (SEND_SECONDS * 1000) / FRAME_MS;
    let mut phase = 0.0f64;
    let frame_duration = Duration::from_millis(FRAME_MS);
    let mut ticker = tokio::time::interval(frame_duration);

    let mut frames_sent = 0u64;
    for _ in 0..frame_count {
        ticker.tick().await;

        let mut pcm = Vec::with_capacity(FRAME_SAMPLES_PER_CHANNEL * CHANNEL_COUNT);
        for _ in 0..FRAME_SAMPLES_PER_CHANNEL {
            let s = (phase.sin() as f32 * TONE_AMPLITUDE * i16::MAX as f32) as i16;
            pcm.push(s); // left
            pcm.push(s); // right
            phase += 2.0 * std::f64::consts::PI * freq_hz / SAMPLE_RATE as f64;
            if phase > 2.0 * std::f64::consts::PI {
                phase -= 2.0 * std::f64::consts::PI;
            }
        }

        let encoded = encoder
            .encode_vec(&pcm, 4000)
            .map_err(|e| format!("failed to Opus-encode frame: {e}"))?;

        let sample = Sample {
            data: Bytes::from(encoded),
            duration: frame_duration,
            ..Default::default()
        };
        track
            .sample_writer(ssrc, OPUS_PAYLOAD_TYPE)
            .write_sample(&sample)
            .await
            .map_err(|e| format!("failed to write sample to track: {e}"))?;
        frames_sent += 1;
    }

    println!("event=tone_send_complete frames_sent={frames_sent} freq_hz={freq_hz}");
    Ok(())
}

/// Prints `get_stats()`'s transport entry: the real, structured evidence
/// that DTLS-SRTP is actually active on this connection, not merely assumed
/// because `webrtc-rs` is "supposed to" always use it.
async fn report_transport_stats(pc: &dyn PeerConnection) {
    let report = pc.get_stats(Instant::now(), StatsSelector::None).await;
    match report.transport() {
        Some(t) => {
            println!(
                "transport_stats dtls_state={:?} dtls_role={:?} tls_version={:?} \
                 dtls_cipher={:?} srtp_cipher={:?} ice_state={:?} \
                 packets_sent={} packets_received={} bytes_sent={} bytes_received={}",
                t.dtls_state,
                t.dtls_role,
                t.tls_version,
                t.dtls_cipher,
                t.srtp_cipher,
                t.ice_state,
                t.packets_sent,
                t.packets_received,
                t.bytes_sent,
                t.bytes_received,
            );
        }
        None => println!("transport_stats unavailable=true"),
    }
}

/// Compares the decoded receive buffer against the tone we expect the peer
/// to have sent us, and prints a structured pass/fail-shaped report line.
/// This — not the transport stats above — is the actual proof that real
/// audio moved through the pipeline and decoded correctly, as opposed to
/// packets merely being counted.
async fn report_audio_verification(
    received_pcm: &RtMutex<Vec<f32>>,
    rtp_received_count: &AtomicUsize,
    role: Role,
) {
    let buf = received_pcm.lock().await;
    let rtp_packets = rtp_received_count.load(Ordering::Relaxed);
    let expected_hz = role.expect_receive_tone_hz();

    if buf.len() < 2 {
        println!(
            "audio_verification decoded_samples={} rtp_packets_received={rtp_packets} \
             result=FAIL reason=no_audio_decoded expected_freq_hz={expected_hz}",
            buf.len()
        );
        return;
    }

    let rms = rms(&buf);
    let estimated_hz = estimate_frequency_hz(&buf, SAMPLE_RATE);
    let freq_diff = (estimated_hz - expected_hz).abs();
    let not_silent = rms >= RMS_SILENCE_FLOOR;
    let freq_matches = freq_diff <= FREQUENCY_TOLERANCE_HZ;
    let result = if not_silent && freq_matches {
        "PASS"
    } else {
        "FAIL"
    };

    println!(
        "audio_verification decoded_samples={} rtp_packets_received={rtp_packets} \
         rms={rms:.4} rms_floor={RMS_SILENCE_FLOOR} estimated_freq_hz={estimated_hz:.1} \
         expected_freq_hz={expected_hz} freq_tolerance_hz={FREQUENCY_TOLERANCE_HZ} \
         not_silent={not_silent} freq_matches={freq_matches} result={result}",
        buf.len()
    );
}

/// Root-mean-square amplitude of normalized (`[-1.0, 1.0]`) samples — the
/// simplest available "is this actually silence?" check.
fn rms(samples: &[f32]) -> f64 {
    let sum_sq: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Estimates the dominant frequency of a (near-)single-tone signal by
/// counting zero crossings. Deliberately not an FFT: this only needs to
/// distinguish "the known sine tone decoded intact" from "silence or
/// garbage," not measure Opus's fidelity precisely, and a zero-crossing
/// count is honest about being a coarse instrument rather than dressing this
/// probe up with more precision than the claim it's making needs.
fn estimate_frequency_hz(samples: &[f32], sample_rate: u32) -> f64 {
    let mut crossings = 0usize;
    for w in samples.windows(2) {
        if (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0) {
            crossings += 1;
        }
    }
    let duration_s = samples.len() as f64 / f64::from(sample_rate);
    if duration_s <= 0.0 {
        return 0.0;
    }
    // Two zero crossings per full cycle.
    (crossings as f64 / 2.0) / duration_s
}

struct Handler {
    gather_tx: Sender<()>,
    connected_tx: Sender<()>,
    received_pcm: Arc<RtMutex<Vec<f32>>>,
    rtp_received_count: Arc<AtomicUsize>,
    runtime: Arc<dyn Runtime>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        println!("event=ice_gathering_state_change state={state:?}");
        if state == RTCIceGatheringState::Complete {
            let _ = self.gather_tx.try_send(());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        println!("event=peer_connection_state_change state={state:?}");
        if state == RTCPeerConnectionState::Connected {
            let _ = self.connected_tx.try_send(());
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        println!("event=on_track kind={:?}", track.kind().await);
        let received_pcm = self.received_pcm.clone();
        let rtp_received_count = self.rtp_received_count.clone();
        self.runtime.spawn(Box::pin(async move {
            let mut decoder = match OpusDecoder::new(SAMPLE_RATE, OPUS_CHANNELS) {
                Ok(d) => d,
                Err(e) => {
                    println!("event=opus_decoder_init_failed error={e}");
                    return;
                }
            };
            let mut pcm_buf = vec![0i16; MAX_DECODE_SAMPLES_PER_CHANNEL * CHANNEL_COUNT];

            while let Some(evt) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(pkt) = evt {
                    rtp_received_count.fetch_add(1, Ordering::Relaxed);
                    match decoder.decode(&pkt.payload, &mut pcm_buf, false) {
                        Ok(samples_per_channel) => {
                            let mut buf = received_pcm.lock().await;
                            for frame in pcm_buf[..samples_per_channel * CHANNEL_COUNT]
                                .chunks_exact(CHANNEL_COUNT)
                            {
                                // Left channel only — both channels carry the
                                // same synthetic tone, so this is not a loss
                                // of signal, just avoiding redundant samples.
                                buf.push(f32::from(frame[0]) / f32::from(i16::MAX));
                            }
                        }
                        Err(e) => println!("event=opus_decode_error error={e}"),
                    }
                }
            }
            println!("event=on_track_ended");
        }));
    }
}
