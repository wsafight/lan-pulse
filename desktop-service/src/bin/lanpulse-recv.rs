use std::{env, net::SocketAddr, time::Instant};

use anyhow::{Result, anyhow};
use lanpulse_service::rtp::{RTP_HEADER_LEN, parse_header};
use tokio::net::UdpSocket;

#[tokio::main]
async fn main() -> Result<()> {
    let bind = parse_bind_arg(env::args().nth(1))?;

    let socket = UdpSocket::bind(bind).await?;
    println!("LanPulse RTP receiver listening on {}", bind);

    let mut buffer = vec![0_u8; 4096];
    let started = Instant::now();
    let mut stats = ReceiverStats::default();

    loop {
        let (n, peer) = socket.recv_from(&mut buffer).await?;
        let header = match parse_header(&buffer[..n]) {
            Ok(header) => header,
            Err(err) => {
                eprintln!("drop packet from {}: {}", peer, err);
                continue;
            }
        };

        let summary = stats.observe_packet(n, header.sequence_number);
        if summary.should_print {
            println!(
                "from={} packets={} lost={} bytes={} seq={} ts={} payload={} elapsed_ms={}",
                peer,
                summary.packets,
                summary.lost,
                summary.bytes,
                summary.sequence_number,
                header.timestamp,
                summary.payload_len,
                started.elapsed().as_millis()
            );
        }
    }
}

#[derive(Debug, Default)]
struct ReceiverStats {
    packets: u64,
    bytes: u64,
    last_seq: Option<u16>,
    lost: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct PacketSummary {
    packets: u64,
    bytes: u64,
    lost: u64,
    sequence_number: u16,
    payload_len: usize,
    should_print: bool,
}

impl ReceiverStats {
    fn observe_packet(&mut self, packet_len: usize, sequence_number: u16) -> PacketSummary {
        if let Some(prev) = self.last_seq {
            let expected = prev.wrapping_add(1);
            if sequence_number != expected {
                self.lost += sequence_number.wrapping_sub(expected) as u64;
            }
        }
        self.last_seq = Some(sequence_number);
        self.packets += 1;
        self.bytes += packet_len as u64;

        PacketSummary {
            packets: self.packets,
            bytes: self.bytes,
            lost: self.lost,
            sequence_number,
            payload_len: payload_len(packet_len),
            should_print: should_print_packet(self.packets),
        }
    }
}

fn parse_bind_arg(arg: Option<String>) -> Result<SocketAddr> {
    arg.unwrap_or_else(|| "0.0.0.0:5004".to_string())
        .parse::<SocketAddr>()
        .map_err(|err| anyhow!("invalid bind address: {}", err))
}

fn payload_len(packet_len: usize) -> usize {
    packet_len.saturating_sub(RTP_HEADER_LEN)
}

fn should_print_packet(packets: u64) -> bool {
    packets == 1 || packets.is_multiple_of(100)
}

#[cfg(test)]
mod tests {
    use lanpulse_service::rtp::RTP_HEADER_LEN;

    use super::{ReceiverStats, parse_bind_arg, payload_len, should_print_packet};

    #[test]
    fn parse_bind_uses_default_and_rejects_invalid_input() {
        assert_eq!(parse_bind_arg(None).unwrap().to_string(), "0.0.0.0:5004");
        assert_eq!(
            parse_bind_arg(Some("127.0.0.1:5504".to_string()))
                .unwrap()
                .to_string(),
            "127.0.0.1:5504"
        );
        assert!(parse_bind_arg(Some("not-an-address".to_string())).is_err());
    }

    #[test]
    fn receiver_stats_track_bytes_payload_and_sequence_loss() {
        let mut stats = ReceiverStats::default();
        let first = stats.observe_packet(RTP_HEADER_LEN + 960, 10);
        let second = stats.observe_packet(RTP_HEADER_LEN + 960, 12);

        assert_eq!(first.packets, 1);
        assert_eq!(first.bytes, (RTP_HEADER_LEN + 960) as u64);
        assert_eq!(first.payload_len, 960);
        assert_eq!(first.lost, 0);
        assert!(first.should_print);
        assert_eq!(second.packets, 2);
        assert_eq!(second.bytes, ((RTP_HEADER_LEN + 960) * 2) as u64);
        assert_eq!(second.lost, 1);
        assert!(!second.should_print);
    }

    #[test]
    fn receiver_stats_handle_wrapping_sequence_numbers() {
        let mut stats = ReceiverStats::default();

        stats.observe_packet(RTP_HEADER_LEN, u16::MAX);
        let summary = stats.observe_packet(RTP_HEADER_LEN, 1);

        assert_eq!(summary.lost, 1);
    }

    #[test]
    fn payload_len_saturates_and_printing_is_first_or_every_hundredth_packet() {
        assert_eq!(payload_len(RTP_HEADER_LEN - 1), 0);
        assert_eq!(payload_len(RTP_HEADER_LEN + 12), 12);
        assert!(should_print_packet(1));
        assert!(!should_print_packet(99));
        assert!(should_print_packet(100));
    }
}
