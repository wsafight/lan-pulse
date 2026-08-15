use std::{net::SocketAddr, sync::Arc, time::Duration, time::Instant};

use anyhow::Result;
use tokio::net::UdpSocket;

use crate::{
    config::{AudioConfig, AudioSourceMode},
    rtp::{RTP_HEADER_LEN, RtpPacketizer},
    state::SessionState,
};

use super::{
    capture::CaptureWorker,
    packet::{frames_per_packet, packet_bytes},
    source::PcmSource,
    tone::ToneSource,
};

pub async fn run_media_sender(
    state: Arc<SessionState>,
    audio: AudioConfig,
    direct_target: Option<SocketAddr>,
    tone_hz: f32,
    source_mode: AudioSourceMode,
    pipewire_target: Option<String>,
) -> Result<()> {
    if let Some(target) = direct_target {
        state.set_target(Some(target)).await;
    }

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let frames_per_packet = frames_per_packet(audio.sample_rate, audio.packet_ms);
    let mut packetizer = RtpPacketizer::new(audio.payload_type, audio.ssrc, frames_per_packet);
    let mut packet = Vec::with_capacity(RTP_HEADER_LEN + packet_bytes(&audio));
    let mut capture: Option<CaptureWorker> = None;
    let mut media_running = false;
    let mut cached_target = None;
    let mut observed_target_generation = u64::MAX;
    let mut pending_stats_packets = 0_u64;
    let mut pending_stats_bytes = 0_u64;
    let mut last_stats_flush = Instant::now();

    let started = Instant::now();

    loop {
        let target_generation = state.target_generation();
        if target_generation != observed_target_generation {
            cached_target = state.target().await;
            observed_target_generation = target_generation;
        }

        let Some(target) = cached_target else {
            if let Some(worker) = capture.take() {
                worker.shutdown().await;
                state.set_media_source("idle").await;
            }
            flush_packet_stats(
                &state,
                &mut pending_stats_packets,
                &mut pending_stats_bytes,
                started,
            )
            .await;
            tokio::time::sleep(Duration::from_millis(audio.packet_ms as u64)).await;
            continue;
        };

        if capture.is_none() {
            let next_source =
                PcmSource::new(source_mode, &audio, tone_hz, pipewire_target.as_deref())?;
            state.set_media_source(next_source.label()).await;
            capture = Some(CaptureWorker::start(next_source, audio.packet_ms)?);
        }

        let frame = capture
            .as_mut()
            .expect("capture worker must exist")
            .receive()
            .await;
        let (dropped, payload) = match frame {
            Ok(frame) => frame,
            Err(err) if source_mode == AudioSourceMode::Auto => {
                tracing::warn!(%err, "audio source failed; falling back to test tone");
                state.record_capture_restart(format!("{err:#}")).await;
                state.set_media_source("tone").await;
                if let Some(worker) = capture.take() {
                    worker.shutdown().await;
                }
                capture = Some(CaptureWorker::start(
                    PcmSource::Tone(ToneSource::new(audio.sample_rate, audio.channels, tone_hz)),
                    audio.packet_ms,
                )?);
                continue;
            }
            Err(err) => {
                if let Some(worker) = capture.take() {
                    worker.shutdown().await;
                }
                flush_packet_stats(
                    &state,
                    &mut pending_stats_packets,
                    &mut pending_stats_bytes,
                    started,
                )
                .await;
                return Err(err);
            }
        };
        if dropped > 0 {
            state.record_capture_dropped(dropped).await;
        }
        packetizer.skip_packets(dropped);
        packetizer.packetize_into(&payload, &mut packet);
        let send_result = socket.send_to(&packet, target).await;
        capture
            .as_mut()
            .expect("capture worker must exist")
            .recycle(payload);
        if let Err(error) = send_result {
            flush_packet_stats(
                &state,
                &mut pending_stats_packets,
                &mut pending_stats_bytes,
                started,
            )
            .await;
            state.record_rtp_send_error(error.to_string()).await;
            if let Some(worker) = capture.take() {
                worker.shutdown().await;
            }
            return Err(error.into());
        }

        if !media_running {
            state.mark_media_running().await;
            media_running = true;
        }

        pending_stats_packets += 1;
        pending_stats_bytes += packet.len() as u64;
        if last_stats_flush.elapsed() >= STATS_FLUSH_INTERVAL {
            flush_packet_stats(
                &state,
                &mut pending_stats_packets,
                &mut pending_stats_bytes,
                started,
            )
            .await;
            last_stats_flush = Instant::now();
        }
    }
}

async fn flush_packet_stats(
    state: &Arc<SessionState>,
    packets: &mut u64,
    bytes: &mut u64,
    started: Instant,
) {
    if *packets == 0 {
        return;
    }
    state
        .record_packets(*packets, *bytes, started.elapsed())
        .await;
    *packets = 0;
    *bytes = 0;
}

const STATS_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use tokio::{net::UdpSocket, time::timeout};

    use super::run_media_sender;
    use crate::{
        config::{AudioConfig, AudioSourceMode},
        state::SessionState,
    };

    #[tokio::test]
    async fn paces_tone_packets_at_configured_interval() {
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = receiver.local_addr().unwrap();
        let audio = AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "s16le".to_string(),
            packet_ms: 10,
            payload_type: 96,
            ssrc: 1,
        };
        let state = Arc::new(SessionState::new("123456".to_string(), audio.clone()));
        let sender_state = Arc::clone(&state);
        let sender = tokio::spawn(async move {
            run_media_sender(
                sender_state,
                audio,
                Some(target),
                440.0,
                AudioSourceMode::Tone,
                None,
            )
            .await
        });

        let started = tokio::time::Instant::now();
        let receive_packets = async {
            let mut buffer = [0_u8; 2048];
            for _ in 0..10 {
                receiver.recv_from(&mut buffer).await.unwrap();
            }
        };
        timeout(Duration::from_secs(1), receive_packets)
            .await
            .expect("timed out waiting for paced RTP packets");
        let elapsed = started.elapsed();

        sender.abort();
        let _ = sender.await;

        assert!(
            elapsed >= Duration::from_millis(60),
            "10 packets were sent too quickly: {elapsed:?}"
        );
    }
}
