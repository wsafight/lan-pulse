use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;

use crate::{
    config::AudioConfig,
    protocol::{MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_VERSION, capabilities},
    state::SessionState,
};

pub const DISCOVERY_MAGIC: &str = "LANPULSE_DISCOVER_V1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryResponse {
    pub r#type: String,
    pub name: String,
    pub control_url: String,
    pub control_port: u16,
    pub pin_required: bool,
    pub audio: AudioConfig,
    pub protocol_version: u16,
    pub min_supported_protocol_version: u16,
    pub capabilities: Vec<String>,
}

pub async fn run_discovery_responder(
    state: Arc<SessionState>,
    socket: UdpSocket,
    bind_port: u16,
    control_url: String,
    control_port: u16,
) -> Result<()> {
    let response = discovery_response(
        state.audio_config().clone(),
        hostname(),
        control_url,
        control_port,
    );
    let payload = serde_json::to_vec(&response)?;
    let mut buffer = [0_u8; 1024];

    tracing::info!(port = bind_port, "LAN discovery responder listening");

    loop {
        let (n, peer) = socket.recv_from(&mut buffer).await?;
        let message = String::from_utf8_lossy(&buffer[..n]);
        if message.trim() != DISCOVERY_MAGIC {
            continue;
        }

        socket.send_to(&payload, peer).await?;
        tracing::debug!(%peer, "sent discovery response");
    }
}

pub async fn bind_first_available_udp(start: u16, end: u16) -> Result<(UdpSocket, u16)> {
    for port in start..=end {
        match UdpSocket::bind(("0.0.0.0", port)).await {
            Ok(socket) => return Ok((socket, port)),
            Err(err) => tracing::warn!(port, %err, "discovery port unavailable"),
        }
    }

    anyhow::bail!("no available discovery port in range {}..={}", start, end)
}

pub async fn discover_once(target: SocketAddr, timeout_ms: u64) -> Result<Vec<DiscoveryResponse>> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket
        .set_broadcast(true)
        .context("failed to enable UDP broadcast")?;
    socket
        .send_to(DISCOVERY_MAGIC.as_bytes(), target)
        .await
        .context("failed to send discovery probe")?;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut responses = Vec::new();
    let mut buffer = [0_u8; 2048];

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        let recv = tokio::time::timeout(remaining, socket.recv_from(&mut buffer)).await;
        let Ok(Ok((n, _peer))) = recv else {
            break;
        };

        if let Ok(response) = serde_json::from_slice::<DiscoveryResponse>(&buffer[..n]) {
            responses.push(response);
        }
    }

    Ok(responses)
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "LanPulse Desktop".to_string())
}

fn discovery_response(
    audio: AudioConfig,
    name: String,
    control_url: String,
    control_port: u16,
) -> DiscoveryResponse {
    DiscoveryResponse {
        r#type: "lanpulse.desktop.v1".to_string(),
        name,
        control_url,
        control_port,
        pin_required: true,
        audio,
        protocol_version: PROTOCOL_VERSION,
        min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
        capabilities: capabilities(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::net::UdpSocket;

    use super::discovery_response;
    use crate::{
        config::AudioConfig,
        discovery::{discover_once, run_discovery_responder},
        state::SessionState,
    };

    fn audio() -> AudioConfig {
        AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "s16le".to_string(),
            packet_ms: 5,
            payload_type: 96,
            ssrc: 42,
        }
    }

    #[test]
    fn builds_stable_discovery_response_payload() {
        let response = discovery_response(
            audio(),
            "Studio PC".to_string(),
            "http://192.168.1.20:4100".to_string(),
            4100,
        );

        assert_eq!(response.r#type, "lanpulse.desktop.v1");
        assert_eq!(response.name, "Studio PC");
        assert_eq!(response.control_url, "http://192.168.1.20:4100");
        assert_eq!(response.control_port, 4100);
        assert!(response.pin_required);
        assert_eq!(response.audio.ssrc, 42);
        assert_eq!(response.protocol_version, 1);
        assert_eq!(response.min_supported_protocol_version, 1);
        assert!(response.capabilities.contains(&"rtp-unicast".to_string()));
    }

    #[tokio::test]
    async fn discovers_response_from_udp_responder() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let responder_addr = socket.local_addr().unwrap();
        let state = Arc::new(SessionState::new("123456".to_string(), audio()));
        let responder_state = Arc::clone(&state);
        let responder = tokio::spawn(async move {
            run_discovery_responder(
                responder_state,
                socket,
                responder_addr.port(),
                "http://127.0.0.1:4100".to_string(),
                4100,
            )
            .await
        });

        let responses = discover_once(responder_addr, 200).await.unwrap();

        responder.abort();
        let _ = responder.await;
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].control_url, "http://127.0.0.1:4100");
        assert_eq!(responses[0].audio.packet_ms, 5);
    }
}
