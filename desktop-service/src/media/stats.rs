use crate::state::StatsSnapshot;

pub fn summarize_stats(stats: &StatsSnapshot) -> String {
    format!(
        "packets={}, bytes={}, target={}, device_session={}, source={}, capture_dropped={}, \
         capture_restarts={}, rtp_send_errors={}, media_restarts={}, last_packet_ms={}",
        stats.packets_sent,
        stats.bytes_sent,
        stats
            .target
            .map(|target| target.to_string())
            .unwrap_or_else(|| "none".to_string()),
        stats
            .device
            .as_ref()
            .map(|device| device.session_id.as_str())
            .unwrap_or("none"),
        stats.media_source,
        stats.capture_packets_dropped,
        stats.capture_restarts,
        stats.rtp_send_errors,
        stats.media_restarts,
        stats
            .last_packet_at_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
    )
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use crate::state::StatsSnapshot;

    use super::summarize_stats;

    #[test]
    fn summarizes_targetless_stats() {
        let stats = StatsSnapshot {
            target: None,
            device: None,
            media_source: "idle".to_string(),
            packets_sent: 7,
            bytes_sent: 128,
            capture_packets_dropped: 0,
            capture_restarts: 0,
            last_capture_error: None,
            rtp_send_errors: 0,
            last_rtp_error: None,
            media_restarts: 0,
            last_media_error: None,
            media_started_ms: None,
            last_packet_at_ms: None,
        };

        assert_eq!(
            summarize_stats(&stats),
            "packets=7, bytes=128, target=none, device_session=none, source=idle, \
             capture_dropped=0, capture_restarts=0, rtp_send_errors=0, media_restarts=0, \
             last_packet_ms=none"
        );
    }

    #[test]
    fn summarizes_target_address_when_present() {
        let stats = StatsSnapshot {
            target: Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
                5504,
            )),
            device: None,
            media_source: "tone".to_string(),
            packets_sent: 3,
            bytes_sent: 960,
            capture_packets_dropped: 0,
            capture_restarts: 0,
            last_capture_error: None,
            rtp_send_errors: 0,
            last_rtp_error: None,
            media_restarts: 0,
            last_media_error: None,
            media_started_ms: None,
            last_packet_at_ms: None,
        };

        assert_eq!(
            summarize_stats(&stats),
            "packets=3, bytes=960, target=192.168.1.50:5504, device_session=none, \
             source=tone, capture_dropped=0, capture_restarts=0, rtp_send_errors=0, \
             media_restarts=0, last_packet_ms=none"
        );
    }
}
