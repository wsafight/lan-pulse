pub const PROTOCOL_VERSION: u16 = 1;
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u16 = 1;

pub const CAPABILITY_PCM_S16LE: &str = "pcm-s16le";
pub const CAPABILITY_RTP_UNICAST: &str = "rtp-unicast";
pub const CAPABILITY_SESSION_ID: &str = "session-id";
pub const CAPABILITY_CLIENT_ID: &str = "client-id";
pub const CAPABILITY_LEASE_HEARTBEAT: &str = "lease-heartbeat";
pub const CAPABILITY_RTP_NACK_V1: &str = "rtp-nack-v1";

pub fn capabilities() -> Vec<String> {
    vec![
        CAPABILITY_PCM_S16LE.to_string(),
        CAPABILITY_RTP_UNICAST.to_string(),
        CAPABILITY_SESSION_ID.to_string(),
        CAPABILITY_CLIENT_ID.to_string(),
        CAPABILITY_LEASE_HEARTBEAT.to_string(),
        CAPABILITY_RTP_NACK_V1.to_string(),
    ]
}

pub fn versions_are_compatible(peer_version: u16, peer_min_supported_version: u16) -> bool {
    peer_version >= MIN_SUPPORTED_PROTOCOL_VERSION && peer_min_supported_version <= PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::{
        CAPABILITY_CLIENT_ID, CAPABILITY_LEASE_HEARTBEAT, CAPABILITY_PCM_S16LE,
        CAPABILITY_RTP_NACK_V1, CAPABILITY_RTP_UNICAST, CAPABILITY_SESSION_ID,
        MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_VERSION, capabilities, versions_are_compatible,
    };

    #[test]
    fn capabilities_are_stable_and_non_empty() {
        assert_eq!(
            capabilities(),
            vec![
                CAPABILITY_PCM_S16LE.to_string(),
                CAPABILITY_RTP_UNICAST.to_string(),
                CAPABILITY_SESSION_ID.to_string(),
                CAPABILITY_CLIENT_ID.to_string(),
                CAPABILITY_LEASE_HEARTBEAT.to_string(),
                CAPABILITY_RTP_NACK_V1.to_string(),
            ]
        );
    }

    #[test]
    fn version_ranges_overlap_when_either_side_can_speak_common_protocol() {
        assert!(versions_are_compatible(
            PROTOCOL_VERSION,
            MIN_SUPPORTED_PROTOCOL_VERSION
        ));
        assert!(!versions_are_compatible(0, 1));
        assert!(!versions_are_compatible(
            PROTOCOL_VERSION,
            PROTOCOL_VERSION + 1
        ));
    }
}
