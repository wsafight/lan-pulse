use crate::config::AudioConfig;

pub fn frames_per_packet(sample_rate: u32, packet_ms: u16) -> u32 {
    sample_rate.saturating_mul(packet_ms as u32) / 1000
}

pub(super) fn packet_bytes(audio: &AudioConfig) -> usize {
    frames_per_packet(audio.sample_rate, audio.packet_ms) as usize * audio.channels as usize * 2
}

#[cfg(test)]
mod tests {
    use crate::config::AudioConfig;

    use super::{frames_per_packet, packet_bytes};

    #[test]
    fn computes_frames_per_packet() {
        assert_eq!(frames_per_packet(48_000, 10), 480);
        assert_eq!(frames_per_packet(48_000, 5), 240);
        assert_eq!(frames_per_packet(44_100, 10), 441);
    }

    #[test]
    fn computes_packet_bytes() {
        let audio = AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "s16le".to_string(),
            packet_ms: 10,
            payload_type: 96,
            ssrc: 1,
        };

        assert_eq!(packet_bytes(&audio), 1920);
    }

    #[test]
    fn default_low_latency_packet_fits_common_mtu() {
        let audio = AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "s16le".to_string(),
            packet_ms: 5,
            payload_type: 96,
            ssrc: 1,
        };

        let rtp_datagram_bytes = packet_bytes(&audio) + crate::rtp::RTP_HEADER_LEN;
        assert!(rtp_datagram_bytes <= 1472);
    }

    #[test]
    fn saturating_frame_calculation_never_wraps() {
        assert_eq!(frames_per_packet(u32::MAX, u16::MAX), u32::MAX / 1000);
    }
}
