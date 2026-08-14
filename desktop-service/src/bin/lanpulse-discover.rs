use std::{
    env,
    net::{IpAddr, SocketAddr},
};

use anyhow::{Result, anyhow};
use lanpulse_service::discovery::{DiscoveryResponse, discover_once};

#[tokio::main]
async fn main() -> Result<()> {
    let target = parse_target_arg(env::args().nth(1))?;

    let responses = discover_once(target, 1_500).await?;
    if should_try_localhost(target, responses.is_empty()) {
        for line in format_response_lines(&discover_once(localhost_target(target)?, 500).await?)? {
            println!("{line}");
        }
        return Ok(());
    }

    for line in format_response_lines(&responses)? {
        println!("{line}");
    }

    Ok(())
}

fn parse_target_arg(arg: Option<String>) -> Result<SocketAddr> {
    arg.unwrap_or_else(|| "255.255.255.255:41000".to_string())
        .parse::<SocketAddr>()
        .map_err(|err| anyhow!("invalid discovery target: {}", err))
}

fn should_try_localhost(target: SocketAddr, responses_empty: bool) -> bool {
    responses_empty && matches!(target.ip(), IpAddr::V4(ip) if ip.is_broadcast())
}

fn localhost_target(target: SocketAddr) -> Result<SocketAddr> {
    Ok(SocketAddr::new("127.0.0.1".parse()?, target.port()))
}

fn format_response_lines(responses: &[DiscoveryResponse]) -> Result<Vec<String>> {
    responses
        .iter()
        .map(serde_json::to_string_pretty)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use lanpulse_service::{config::AudioConfig, discovery::DiscoveryResponse};

    use super::{format_response_lines, localhost_target, parse_target_arg, should_try_localhost};

    fn response() -> DiscoveryResponse {
        DiscoveryResponse {
            r#type: "lanpulse.desktop.v1".to_string(),
            name: "Studio".to_string(),
            control_url: "http://127.0.0.1:4100".to_string(),
            control_port: 4100,
            pin_required: true,
            audio: AudioConfig {
                sample_rate: 48_000,
                channels: 2,
                sample_format: "s16le".to_string(),
                packet_ms: 5,
                payload_type: 96,
                ssrc: 1,
            },
            protocol_version: 1,
            min_supported_protocol_version: 1,
            capabilities: vec!["rtp-unicast".to_string()],
        }
    }

    #[test]
    fn parse_target_uses_broadcast_default_and_rejects_invalid_input() {
        assert_eq!(
            parse_target_arg(None).unwrap().to_string(),
            "255.255.255.255:41000"
        );
        assert_eq!(
            parse_target_arg(Some("127.0.0.1:41000".to_string()))
                .unwrap()
                .to_string(),
            "127.0.0.1:41000"
        );
        assert!(parse_target_arg(Some("not-an-address".to_string())).is_err());
    }

    #[test]
    fn localhost_retry_is_only_for_empty_broadcast_results() {
        let broadcast = parse_target_arg(None).unwrap();
        let unicast = parse_target_arg(Some("192.168.1.10:41000".to_string())).unwrap();

        assert!(should_try_localhost(broadcast, true));
        assert!(!should_try_localhost(broadcast, false));
        assert!(!should_try_localhost(unicast, true));
        assert_eq!(
            localhost_target(broadcast).unwrap().to_string(),
            "127.0.0.1:41000"
        );
    }

    #[test]
    fn format_response_lines_outputs_pretty_json() {
        let lines = format_response_lines(&[response()]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('\n'));
        assert_eq!(value["type"], "lanpulse.desktop.v1");
        assert_eq!(value["control_port"], 4100);
        assert_eq!(value["protocol_version"], 1);
    }
}
