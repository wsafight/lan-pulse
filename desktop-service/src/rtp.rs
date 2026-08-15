use anyhow::{Result, anyhow};

pub const RTP_HEADER_LEN: usize = 12;
pub const RTP_NACK_LEN: usize = 12;
const RTP_NACK_MAGIC: [u8; 4] = *b"LPNK";
const RTP_NACK_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpHeader {
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpNack {
    pub sequence_number: u16,
    pub ssrc: u32,
}

pub struct RtpPacketizer {
    payload_type: u8,
    sequence_number: u16,
    timestamp: u32,
    ssrc: u32,
    timestamp_step: u32,
}

impl RtpPacketizer {
    pub fn new(payload_type: u8, ssrc: u32, frames_per_packet: u32) -> Self {
        Self {
            payload_type,
            sequence_number: 0,
            timestamp: 0,
            ssrc,
            timestamp_step: frames_per_packet,
        }
    }

    pub fn packetize(&mut self, payload: &[u8]) -> Vec<u8> {
        let mut packet = Vec::with_capacity(RTP_HEADER_LEN + payload.len());
        self.packetize_into(payload, &mut packet);
        packet
    }

    pub fn packetize_into(&mut self, payload: &[u8], packet: &mut Vec<u8>) {
        packet.clear();
        packet.reserve(RTP_HEADER_LEN + payload.len());

        packet.push(0x80);
        packet.push(self.payload_type & 0x7f);
        packet.extend_from_slice(&self.sequence_number.to_be_bytes());
        packet.extend_from_slice(&self.timestamp.to_be_bytes());
        packet.extend_from_slice(&self.ssrc.to_be_bytes());
        packet.extend_from_slice(payload);

        self.sequence_number = self.sequence_number.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(self.timestamp_step);
    }

    pub fn skip_packets(&mut self, count: u64) {
        self.sequence_number = self.sequence_number.wrapping_add(count as u16);
        self.timestamp = self
            .timestamp
            .wrapping_add(self.timestamp_step.wrapping_mul(count as u32));
    }
}

pub fn parse_header(packet: &[u8]) -> Result<RtpHeader> {
    if packet.len() < RTP_HEADER_LEN {
        return Err(anyhow!("RTP packet too short: {}", packet.len()));
    }

    let version = packet[0] >> 6;
    if version != 2 {
        return Err(anyhow!("unsupported RTP version: {}", version));
    }

    Ok(RtpHeader {
        payload_type: packet[1] & 0x7f,
        sequence_number: u16::from_be_bytes([packet[2], packet[3]]),
        timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
    })
}

pub fn parse_nack(packet: &[u8]) -> Option<RtpNack> {
    if packet.len() != RTP_NACK_LEN
        || packet[..4] != RTP_NACK_MAGIC
        || packet[4] != RTP_NACK_VERSION
        || packet[5] != 0
    {
        return None;
    }
    Some(RtpNack {
        sequence_number: u16::from_be_bytes([packet[6], packet[7]]),
        ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
    })
}

#[cfg(test)]
mod tests {
    use super::{RTP_HEADER_LEN, RTP_NACK_LEN, RtpNack, RtpPacketizer, parse_header, parse_nack};

    #[test]
    fn packetizes_rtp_header_and_payload() {
        let mut packetizer = RtpPacketizer::new(96, 0x11223344, 480);
        let packet = packetizer.packetize(&[1, 2, 3, 4]);
        let header = parse_header(&packet).unwrap();

        assert_eq!(packet.len(), RTP_HEADER_LEN + 4);
        assert_eq!(header.payload_type, 96);
        assert_eq!(header.sequence_number, 0);
        assert_eq!(header.timestamp, 0);
        assert_eq!(header.ssrc, 0x11223344);
        assert_eq!(&packet[RTP_HEADER_LEN..], &[1, 2, 3, 4]);

        let packet = packetizer.packetize(&[5, 6]);
        let header = parse_header(&packet).unwrap();
        assert_eq!(header.sequence_number, 1);
        assert_eq!(header.timestamp, 480);
    }

    #[test]
    fn advances_sequence_and_timestamp_for_dropped_capture_packets() {
        let mut packetizer = RtpPacketizer::new(96, 1, 240);
        packetizer.skip_packets(3);
        let packet = packetizer.packetize(&[1]);
        let header = parse_header(&packet).unwrap();
        assert_eq!(header.sequence_number, 3);
        assert_eq!(header.timestamp, 720);
    }

    #[test]
    fn reuses_packet_buffer_capacity() {
        let mut packetizer = RtpPacketizer::new(96, 1, 240);
        let mut packet = Vec::with_capacity(RTP_HEADER_LEN + 960);
        let capacity = packet.capacity();

        packetizer.packetize_into(&[7; 960], &mut packet);
        packetizer.packetize_into(&[8; 960], &mut packet);

        assert_eq!(packet.capacity(), capacity);
        assert_eq!(packet.len(), RTP_HEADER_LEN + 960);
        assert_eq!(&packet[RTP_HEADER_LEN..], &[8; 960]);
    }

    #[test]
    fn rejects_short_packet_and_wrong_version() {
        assert!(parse_header(&[0; RTP_HEADER_LEN - 1]).is_err());

        let mut packet = [0_u8; RTP_HEADER_LEN];
        packet[0] = 0x40;
        assert!(parse_header(&packet).is_err());
    }

    #[test]
    fn masks_marker_bit_from_payload_type() {
        let packet = [0x80, 0x80 | 96, 0x12, 0x34, 0, 0, 0, 7, 0, 0, 0, 9];

        let header = parse_header(&packet).unwrap();

        assert_eq!(header.payload_type, 96);
        assert_eq!(header.sequence_number, 0x1234);
        assert_eq!(header.timestamp, 7);
        assert_eq!(header.ssrc, 9);
    }

    #[test]
    fn parses_versioned_bounded_retransmit_request() {
        let mut packet = [0_u8; RTP_NACK_LEN];
        packet[..4].copy_from_slice(b"LPNK");
        packet[4] = 1;
        packet[6..8].copy_from_slice(&0x1234_u16.to_be_bytes());
        packet[8..12].copy_from_slice(&0x5566_7788_u32.to_be_bytes());

        assert_eq!(
            parse_nack(&packet),
            Some(RtpNack {
                sequence_number: 0x1234,
                ssrc: 0x5566_7788,
            })
        );

        packet[4] = 2;
        assert_eq!(parse_nack(&packet), None);
        assert_eq!(parse_nack(&packet[..RTP_NACK_LEN - 1]), None);
    }
}
