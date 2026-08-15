use std::{io::ErrorKind, net::SocketAddr, sync::Arc, time::Duration, time::Instant};

use anyhow::Result;
use tokio::net::UdpSocket;

use crate::{
    config::{AudioConfig, AudioSourceMode},
    rtp::{RTP_HEADER_LEN, RTP_NACK_LEN, RtpPacketizer, parse_nack},
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
    if let Err(error) = socket.set_tos_v4(RTP_IP_TOS) {
        tracing::warn!(%error, "failed to configure RTP UDP QoS");
    }
    let frames_per_packet = frames_per_packet(audio.sample_rate, audio.packet_ms);
    let mut packetizer = RtpPacketizer::new(audio.payload_type, audio.ssrc, frames_per_packet);
    let mut packet = Vec::with_capacity(RTP_HEADER_LEN + packet_bytes(&audio));
    let mut retransmit_cache = RetransmitCache::new(
        RETRANSMIT_CACHE_PACKETS,
        RTP_HEADER_LEN + packet_bytes(&audio),
    );
    let mut nack_buffer = [0_u8; RTP_NACK_LEN];
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
            );
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
                );
                return Err(err);
            }
        };
        if dropped > 0 {
            state.record_capture_dropped(dropped).await;
        }
        packetizer.skip_packets(dropped);
        packetizer.packetize_into(&payload, &mut packet);
        retransmit_cache.store(&packet);
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
            );
            state.record_rtp_send_error(error.to_string()).await;
            if let Some(worker) = capture.take() {
                worker.shutdown().await;
            }
            return Err(error.into());
        }
        service_retransmit_requests(
            &socket,
            target,
            audio.ssrc,
            &retransmit_cache,
            &mut nack_buffer,
        );

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
            );
            last_stats_flush = Instant::now();
        }
    }
}

fn flush_packet_stats(
    state: &Arc<SessionState>,
    packets: &mut u64,
    bytes: &mut u64,
    started: Instant,
) {
    if *packets == 0 {
        return;
    }
    state.record_packets(*packets, *bytes, started.elapsed());
    *packets = 0;
    *bytes = 0;
}

const STATS_FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const RTP_IP_TOS: u32 = 46 << 2;
const RETRANSMIT_CACHE_PACKETS: usize = 64;
const MAX_RETRANSMITS_PER_MEDIA_PACKET: usize = 2;

struct CachedRtpPacket {
    sequence_number: u16,
    valid: bool,
    bytes: Vec<u8>,
}

struct RetransmitCache {
    slots: Vec<CachedRtpPacket>,
}

impl RetransmitCache {
    fn new(capacity: usize, packet_bytes: usize) -> Self {
        assert!(capacity.is_power_of_two());
        Self {
            slots: (0..capacity)
                .map(|_| CachedRtpPacket {
                    sequence_number: 0,
                    valid: false,
                    bytes: Vec::with_capacity(packet_bytes),
                })
                .collect(),
        }
    }

    fn store(&mut self, packet: &[u8]) {
        if packet.len() < RTP_HEADER_LEN {
            return;
        }
        let sequence_number = u16::from_be_bytes([packet[2], packet[3]]);
        let slot_mask = self.slots.len() - 1;
        let slot = &mut self.slots[sequence_number as usize & slot_mask];
        slot.sequence_number = sequence_number;
        slot.valid = true;
        slot.bytes.clear();
        slot.bytes.extend_from_slice(packet);
    }

    fn get(&self, sequence_number: u16) -> Option<&[u8]> {
        let slot = &self.slots[sequence_number as usize & (self.slots.len() - 1)];
        (slot.valid && slot.sequence_number == sequence_number).then_some(slot.bytes.as_slice())
    }
}

fn service_retransmit_requests(
    socket: &UdpSocket,
    target: SocketAddr,
    expected_ssrc: u32,
    cache: &RetransmitCache,
    nack_buffer: &mut [u8; RTP_NACK_LEN],
) {
    for _ in 0..MAX_RETRANSMITS_PER_MEDIA_PACKET {
        let (length, source) = match socket.try_recv_from(nack_buffer) {
            Ok(received) => received,
            Err(error) if error.kind() == ErrorKind::WouldBlock => return,
            Err(error) => {
                tracing::debug!(%error, "failed to receive RTP retransmit request");
                return;
            }
        };
        if source != target {
            continue;
        }
        let Some(nack) = parse_nack(&nack_buffer[..length]) else {
            continue;
        };
        if nack.ssrc != expected_ssrc {
            continue;
        }
        let Some(packet) = cache.get(nack.sequence_number) else {
            continue;
        };
        if let Err(error) = socket.try_send_to(packet, target)
            && error.kind() != ErrorKind::WouldBlock
        {
            tracing::debug!(%error, sequence = nack.sequence_number, "failed to retransmit RTP packet");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use tokio::{net::UdpSocket, time::timeout};

    use super::{RETRANSMIT_CACHE_PACKETS, RetransmitCache, run_media_sender};
    use crate::{
        config::{AudioConfig, AudioSourceMode},
        rtp::{RTP_HEADER_LEN, RTP_NACK_LEN, RtpPacketizer, parse_header},
        state::SessionState,
    };

    #[test]
    fn retransmit_cache_reuses_slots_and_rejects_evicted_sequences() {
        let mut packetizer = RtpPacketizer::new(96, 1, 240);
        let mut cache = RetransmitCache::new(4, RTP_HEADER_LEN + 4);
        let first = packetizer.packetize(&[1, 2, 3, 4]);
        cache.store(&first);
        assert_eq!(cache.get(0), Some(first.as_slice()));

        for value in 1..=4 {
            cache.store(&packetizer.packetize(&[value; 4]));
        }

        assert_eq!(cache.get(0), None);
        assert!(cache.get(4).is_some());
    }

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

    #[tokio::test]
    async fn retransmits_recent_packet_for_valid_nack_from_active_target() {
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = receiver.local_addr().unwrap();
        let audio = AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "s16le".to_string(),
            packet_ms: 5,
            payload_type: 96,
            ssrc: 0x1122_3344,
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

        let mut buffer = [0_u8; 2048];
        let (first_length, sender_address) = receiver.recv_from(&mut buffer).await.unwrap();
        let requested = parse_header(&buffer[..first_length]).unwrap();
        let mut nack = [0_u8; RTP_NACK_LEN];
        nack[..4].copy_from_slice(b"LPNK");
        nack[4] = 1;
        nack[6..8].copy_from_slice(&requested.sequence_number.to_be_bytes());
        nack[8..12].copy_from_slice(&requested.ssrc.to_be_bytes());
        receiver.send_to(&nack, sender_address).await.unwrap();

        let recovered = timeout(Duration::from_millis(250), async {
            loop {
                let (length, _) = receiver.recv_from(&mut buffer).await.unwrap();
                if parse_header(&buffer[..length]).unwrap().sequence_number
                    == requested.sequence_number
                {
                    return true;
                }
            }
        })
        .await
        .expect("timed out waiting for retransmitted RTP packet");

        sender.abort();
        let _ = sender.await;
        assert!(recovered);
        assert_eq!(RETRANSMIT_CACHE_PACKETS, 64);
    }
}
