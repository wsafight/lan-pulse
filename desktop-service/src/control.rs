use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Result, anyhow};
use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::{
    protocol::{
        CAPABILITY_RTP_UNICAST, MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_VERSION, capabilities,
        versions_are_compatible,
    },
    state::{ConnectedDevice, PairingPinResult, SessionState},
};

pub fn router(state: Arc<SessionState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/status", get(status))
        .route("/api/connect", post(connect))
        .route("/api/heartbeat", post(heartbeat))
        .route("/api/disconnect", post(disconnect))
        .with_state(state)
}

pub async fn bind_first_available(host: &str, start: u16, end: u16) -> Result<(TcpListener, u16)> {
    for port in start..=end {
        let addr = format!("{}:{}", host, port);
        match TcpListener::bind(&addr).await {
            Ok(listener) => return Ok((listener, port)),
            Err(err) => tracing::warn!(%addr, %err, "control port unavailable"),
        }
    }

    anyhow::bail!("no available control port in range {}..={}", start, end)
}

async fn index() -> &'static str {
    "LanPulse service is running. Pair from the mobile app with the printed PIN.\n"
}

async fn status(
    State(state): State<Arc<SessionState>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
) -> Json<StatusResponse> {
    if remote.ip().is_loopback() {
        state.refresh_pairing_pin();
    }
    let stats = state.snapshot().await;
    Json(StatusResponse {
        ok: true,
        audio: state.audio_config().clone(),
        stats,
    })
}

async fn connect(
    State(state): State<Arc<SessionState>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(request): Json<ConnectRequest>,
) -> impl IntoResponse {
    if !request.protocol_is_compatible() {
        tracing::warn!(
            %remote,
            protocol_version = ?request.protocol_version,
            min_supported_protocol_version = ?request.min_supported_protocol_version,
            "rejected incompatible mobile protocol"
        );
        return (
            StatusCode::UPGRADE_REQUIRED,
            Json(ConnectResponse::protocol_incompatible()),
        )
            .into_response();
    }

    let target = SocketAddr::new(remote.ip(), request.udp_port);
    let client_id = request
        .client_id
        .clone()
        .unwrap_or_else(|| format!("legacy:{}", remote.ip()));
    let device_name = request
        .device_name
        .clone()
        .unwrap_or_else(|| "device".to_string());
    if let Some(session_id) = request.session_id.as_deref().filter(|id| !id.is_empty())
        && state
            .resume_device(&client_id, session_id, device_name.clone(), target)
            .await
    {
        tracing::info!(
            %remote,
            %client_id,
            %session_id,
            device = %device_name,
            %target,
            "resumed mobile session"
        );
        let response = ConnectResponse {
            ok: true,
            message: format!("resumed {}", device_name),
            session_id: Some(session_id.to_string()),
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            capabilities: capabilities(),
            media: Some(MediaConfig {
                target_ip: remote.ip().to_string(),
                target_port: request.udp_port,
                audio: state.audio_config().clone(),
            }),
        };
        return (StatusCode::OK, Json(response)).into_response();
    }

    let pairing_result = state.authorize_pairing_pin(&request.pin).await;
    match pairing_result {
        PairingPinResult::Accepted => {}
        PairingPinResult::Invalid => {
            tracing::warn!(%remote, %client_id, "rejected invalid pairing PIN");
            return (StatusCode::UNAUTHORIZED, Json(ConnectResponse::denied())).into_response();
        }
        PairingPinResult::Expired => {
            tracing::warn!(%remote, %client_id, "rejected expired pairing PIN");
            return (
                StatusCode::UNAUTHORIZED,
                Json(ConnectResponse::pin_expired()),
            )
                .into_response();
        }
        PairingPinResult::Blocked => {
            tracing::warn!(%remote, %client_id, "blocked mobile pairing attempt");
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ConnectResponse::too_many_attempts()),
            )
                .into_response();
        }
    }

    let session_id = match next_session_id() {
        Ok(session_id) => session_id,
        Err(error) => {
            tracing::error!(%error, "failed to create session id");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ConnectResponse::temporary_failure()),
            )
                .into_response();
        }
    };
    let device = ConnectedDevice::new(client_id, session_id.clone(), device_name.clone(), target);
    if !state.connect_device(device).await {
        tracing::warn!(%remote, %session_id, device = %device_name, "mobile session rejected as busy");
        return (StatusCode::CONFLICT, Json(ConnectResponse::device_busy())).into_response();
    }
    tracing::info!(
        %remote,
        %session_id,
        device = %device_name,
        %target,
        "connected mobile session"
    );

    let response = ConnectResponse {
        ok: true,
        message: format!("connected {}", device_name),
        session_id: Some(session_id),
        protocol_version: PROTOCOL_VERSION,
        min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
        capabilities: capabilities(),
        media: Some(MediaConfig {
            target_ip: remote.ip().to_string(),
            target_port: request.udp_port,
            audio: state.audio_config().clone(),
        }),
    };

    (StatusCode::OK, Json(response)).into_response()
}

async fn disconnect(
    State(state): State<Arc<SessionState>>,
    Json(request): Json<DisconnectRequest>,
) -> impl IntoResponse {
    if !state.pin_matches(&request.pin) {
        return (StatusCode::UNAUTHORIZED, "access denied").into_response();
    }

    let disconnected = state.disconnect_device(request.session_id.as_deref()).await;
    if disconnected {
        tracing::info!(session_id = ?request.session_id, "disconnected mobile session");
    } else {
        tracing::warn!(session_id = ?request.session_id, "ignored stale disconnect request");
    }
    let message = if disconnected {
        "disconnected"
    } else {
        "session is no longer active"
    };
    (StatusCode::OK, message).into_response()
}

async fn heartbeat(
    State(state): State<Arc<SessionState>>,
    Json(request): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    if !state.pin_matches(&request.pin) {
        return (StatusCode::UNAUTHORIZED, "access denied");
    }
    if state.refresh_session(&request.session_id).await {
        (StatusCode::OK, "ok")
    } else {
        tracing::warn!(session_id = %request.session_id, "rejected heartbeat for inactive session");
        (StatusCode::CONFLICT, "session is no longer active")
    }
}

#[derive(Debug, Deserialize)]
struct ConnectRequest {
    pin: String,
    udp_port: u16,
    client_id: Option<String>,
    device_name: Option<String>,
    session_id: Option<String>,
    protocol_version: Option<u16>,
    min_supported_protocol_version: Option<u16>,
    #[serde(default)]
    capabilities: Vec<String>,
}

impl ConnectRequest {
    fn protocol_is_compatible(&self) -> bool {
        let version_is_compatible =
            match (self.protocol_version, self.min_supported_protocol_version) {
                (Some(version), Some(min_supported_version)) => {
                    versions_are_compatible(version, min_supported_version)
                }
                _ => true,
            };
        version_is_compatible
            && (self.capabilities.is_empty()
                || self
                    .capabilities
                    .iter()
                    .any(|capability| capability == CAPABILITY_RTP_UNICAST))
    }
}

#[derive(Debug, Deserialize)]
struct DisconnectRequest {
    pin: String,
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    pin: String,
    session_id: String,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    ok: bool,
    audio: crate::config::AudioConfig,
    stats: crate::state::StatsSnapshot,
}

#[derive(Debug, Serialize)]
struct MediaConfig {
    target_ip: String,
    target_port: u16,
    audio: crate::config::AudioConfig,
}

#[derive(Debug, Serialize)]
struct ConnectResponse {
    ok: bool,
    message: String,
    session_id: Option<String>,
    protocol_version: u16,
    min_supported_protocol_version: u16,
    capabilities: Vec<String>,
    media: Option<MediaConfig>,
}

impl ConnectResponse {
    fn denied() -> Self {
        Self {
            ok: false,
            message: "invalid pin".to_string(),
            session_id: None,
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            capabilities: capabilities(),
            media: None,
        }
    }

    fn device_busy() -> Self {
        Self {
            ok: false,
            message: "another device is already connected".to_string(),
            session_id: None,
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            capabilities: capabilities(),
            media: None,
        }
    }

    fn protocol_incompatible() -> Self {
        Self {
            ok: false,
            message: "protocol version is not compatible".to_string(),
            session_id: None,
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            capabilities: capabilities(),
            media: None,
        }
    }

    fn pin_expired() -> Self {
        Self {
            ok: false,
            message: "pin expired".to_string(),
            session_id: None,
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            capabilities: capabilities(),
            media: None,
        }
    }

    fn too_many_attempts() -> Self {
        Self {
            ok: false,
            message: "too many failed pairing attempts".to_string(),
            session_id: None,
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            capabilities: capabilities(),
            media: None,
        }
    }

    fn temporary_failure() -> Self {
        Self {
            ok: false,
            message: "temporary desktop failure".to_string(),
            session_id: None,
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            capabilities: capabilities(),
            media: None,
        }
    }
}

fn next_session_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| anyhow!("generate session id: {error}"))?;
    static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let sequence_bytes = sequence.to_be_bytes();
    for (index, byte) in sequence_bytes.iter().enumerate() {
        bytes[8 + index] ^= byte;
    }
    Ok(hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::Arc,
        time::Duration,
    };

    use axum::{
        Json,
        body::to_bytes,
        extract::{ConnectInfo, State},
        http::StatusCode,
        response::{IntoResponse, Response},
    };

    use super::{
        ConnectRequest, DisconnectRequest, HeartbeatRequest, connect, disconnect, heartbeat,
        next_session_id, status,
    };
    use crate::{
        config::AudioConfig,
        state::{ConnectedDevice, SessionState},
    };

    fn state() -> Arc<SessionState> {
        Arc::new(SessionState::new(
            "123456".to_string(),
            AudioConfig {
                sample_rate: 48_000,
                channels: 2,
                sample_format: "s16le".to_string(),
                packet_ms: 5,
                payload_type: 96,
                ssrc: 1,
            },
        ))
    }

    fn remote() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)), 49152)
    }

    fn loopback() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152)
    }

    fn connect_request(pin: &str) -> ConnectRequest {
        ConnectRequest {
            pin: pin.to_string(),
            udp_port: 5504,
            client_id: Some("phone-a".to_string()),
            device_name: Some("Phone".to_string()),
            session_id: None,
            protocol_version: Some(1),
            min_supported_protocol_version: Some(1),
            capabilities: vec!["rtp-unicast".to_string()],
        }
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable");
        serde_json::from_slice(&bytes).expect("response body should be JSON")
    }

    #[tokio::test]
    async fn status_response_reflects_session_state() {
        let state = state();
        state
            .record_packet(100, std::time::Duration::from_millis(5))
            .await;

        let Json(response) = status(State(Arc::clone(&state)), ConnectInfo(loopback())).await;

        assert!(response.ok);
        assert_eq!(response.audio.packet_ms, 5);
        assert_eq!(response.stats.packets_sent, 1);
        assert_eq!(response.stats.bytes_sent, 100);
    }

    #[tokio::test]
    async fn local_status_poll_renews_pairing_pin() {
        let state = Arc::new(SessionState::new_for_tests(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_secs(15),
            Duration::from_millis(10),
            5,
            Duration::from_secs(1),
        ));
        tokio::time::sleep(Duration::from_millis(20)).await;

        let _ = status(State(Arc::clone(&state)), ConnectInfo(loopback())).await;

        assert_eq!(
            state.authorize_pairing_pin("123456").await,
            crate::state::PairingPinResult::Accepted
        );
    }

    #[tokio::test]
    async fn remote_status_poll_does_not_renew_pairing_pin() {
        let state = Arc::new(SessionState::new_for_tests(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_secs(15),
            Duration::from_millis(10),
            5,
            Duration::from_secs(1),
        ));
        tokio::time::sleep(Duration::from_millis(20)).await;

        let _ = status(State(Arc::clone(&state)), ConnectInfo(remote())).await;

        assert_eq!(
            state.authorize_pairing_pin("123456").await,
            crate::state::PairingPinResult::Expired
        );
    }

    #[tokio::test]
    async fn connect_rejects_bad_pin_without_setting_target() {
        let state = state();
        let response = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(connect_request("bad-pin")),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(state.target().await, None);
    }

    #[tokio::test]
    async fn connect_sets_mobile_udp_target_from_remote_ip() {
        let state = state();
        let response = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(connect_request("123456")),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            state.target().await,
            Some(SocketAddr::new(remote().ip(), 5504))
        );
        assert_eq!(state.device().await.unwrap().name, "Phone");
    }

    #[tokio::test]
    async fn connect_rejects_expired_pairing_pin_without_setting_target() {
        let state = Arc::new(SessionState::new_for_tests(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_secs(15),
            Duration::from_millis(10),
            5,
            Duration::from_secs(1),
        ));
        tokio::time::sleep(Duration::from_millis(20)).await;

        let response = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(connect_request("123456")),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(state.target().await, None);
    }

    #[tokio::test]
    async fn connect_resumes_active_session_after_pairing_pin_expires() {
        let state = Arc::new(SessionState::new_for_tests(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_secs(15),
            Duration::from_millis(10),
            5,
            Duration::from_secs(1),
        ));
        let initial = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(connect_request("123456")),
        )
        .await
        .into_response();
        assert_eq!(initial.status(), StatusCode::OK);
        let session_id = response_json(initial)
            .await
            .get("session_id")
            .and_then(|value| value.as_str())
            .expect("connect should return a session id")
            .to_string();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let mut request = connect_request("123456");
        request.udp_port = 5505;
        request.session_id = Some(session_id.clone());
        let resumed = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(request),
        )
        .await
        .into_response();

        assert_eq!(resumed.status(), StatusCode::OK);
        assert_eq!(
            response_json(resumed).await["session_id"].as_str(),
            Some(session_id.as_str())
        );
        assert_eq!(
            state.target().await,
            Some(SocketAddr::new(remote().ip(), 5505))
        );
    }

    #[tokio::test]
    async fn connect_rate_limits_repeated_bad_pairing_pins() {
        let state = Arc::new(SessionState::new_for_tests(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_secs(15),
            Duration::from_secs(60),
            2,
            Duration::from_millis(50),
        ));

        let first = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(connect_request("000000")),
        )
        .await
        .into_response();
        let second = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(connect_request("111111")),
        )
        .await
        .into_response();
        let blocked_valid_pin = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(connect_request("123456")),
        )
        .await
        .into_response();

        assert_eq!(first.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(blocked_valid_pin.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(state.target().await, None);
    }

    #[tokio::test]
    async fn connect_rejects_explicitly_incompatible_protocol() {
        let state = state();
        let response = connect(
            State(state),
            ConnectInfo(remote()),
            Json(ConnectRequest {
                pin: "123456".to_string(),
                udp_port: 5504,
                client_id: Some("phone-a".to_string()),
                device_name: Some("Phone".to_string()),
                session_id: None,
                protocol_version: Some(1),
                min_supported_protocol_version: Some(2),
                capabilities: Vec::new(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    }

    #[tokio::test]
    async fn connect_rejects_client_without_required_media_capability() {
        let state = state();
        let mut request = connect_request("123456");
        request.capabilities = vec!["unknown-media".to_string()];

        let response = connect(State(state), ConnectInfo(remote()), Json(request))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    }

    #[tokio::test]
    async fn connect_defaults_legacy_client_metadata_when_omitted() {
        let state = state();
        let response = connect(
            State(Arc::clone(&state)),
            ConnectInfo(remote()),
            Json(ConnectRequest {
                pin: "123456".to_string(),
                udp_port: 5504,
                client_id: None,
                device_name: None,
                session_id: None,
                protocol_version: None,
                min_supported_protocol_version: None,
                capabilities: Vec::new(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let device = state.device().await.unwrap();
        assert_eq!(device.client_id, "legacy:192.168.1.50");
        assert_eq!(device.name, "device");
    }

    #[tokio::test]
    async fn connect_reports_conflict_when_another_client_is_active() {
        let state = state();
        let active = ConnectedDevice::new(
            "phone-a".to_string(),
            "session-a".to_string(),
            "Phone A".to_string(),
            SocketAddr::new(remote().ip(), 5504),
        );
        assert!(state.connect_device(active).await);

        let response = connect(
            State(state),
            ConnectInfo(remote()),
            Json(ConnectRequest {
                pin: "123456".to_string(),
                udp_port: 5505,
                client_id: Some("phone-b".to_string()),
                device_name: Some("Phone B".to_string()),
                session_id: None,
                protocol_version: Some(1),
                min_supported_protocol_version: Some(1),
                capabilities: Vec::new(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn heartbeat_accepts_active_session_and_rejects_stale_session() {
        let state = state();
        assert!(
            state
                .connect_device(ConnectedDevice::new(
                    "phone-a".to_string(),
                    "session-a".to_string(),
                    "Phone A".to_string(),
                    SocketAddr::new(remote().ip(), 5504),
                ))
                .await
        );

        let ok = heartbeat(
            State(Arc::clone(&state)),
            Json(HeartbeatRequest {
                pin: "123456".to_string(),
                session_id: "session-a".to_string(),
            }),
        )
        .await
        .into_response();
        let stale = heartbeat(
            State(state),
            Json(HeartbeatRequest {
                pin: "123456".to_string(),
                session_id: "session-old".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(stale.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn heartbeat_rejects_bad_pin() {
        let state = state();
        let response = heartbeat(
            State(state),
            Json(HeartbeatRequest {
                pin: "bad-pin".to_string(),
                session_id: "session-a".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn disconnect_rejects_bad_pin_and_clears_matching_session() {
        let state = state();
        assert!(
            state
                .connect_device(ConnectedDevice::new(
                    "phone-a".to_string(),
                    "session-a".to_string(),
                    "Phone A".to_string(),
                    SocketAddr::new(remote().ip(), 5504),
                ))
                .await
        );

        let denied = disconnect(
            State(Arc::clone(&state)),
            Json(DisconnectRequest {
                pin: "000000".to_string(),
                session_id: Some("session-a".to_string()),
            }),
        )
        .await
        .into_response();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert!(state.device().await.is_some());

        let ok = disconnect(
            State(Arc::clone(&state)),
            Json(DisconnectRequest {
                pin: "123456".to_string(),
                session_id: Some("session-a".to_string()),
            }),
        )
        .await
        .into_response();
        assert_eq!(ok.status(), StatusCode::OK);
        assert!(state.device().await.is_none());
        assert!(state.target().await.is_none());
    }

    #[tokio::test]
    async fn disconnect_keeps_device_when_session_does_not_match() {
        let state = state();
        assert!(
            state
                .connect_device(ConnectedDevice::new(
                    "phone-a".to_string(),
                    "session-a".to_string(),
                    "Phone A".to_string(),
                    SocketAddr::new(remote().ip(), 5504),
                ))
                .await
        );

        let response = disconnect(
            State(Arc::clone(&state)),
            Json(DisconnectRequest {
                pin: "123456".to_string(),
                session_id: Some("session-old".to_string()),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.device().await.is_some());
        assert_eq!(
            state.target().await,
            Some(SocketAddr::new(remote().ip(), 5504))
        );
    }

    #[test]
    fn session_ids_are_unique_random_hex_values() {
        let first = next_session_id().unwrap();
        let second = next_session_id().unwrap();

        assert_ne!(first, second);
        assert_eq!(first.len(), 32);
        assert_eq!(second.len(), 32);
        assert!(first.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert!(second.chars().all(|ch| ch.is_ascii_hexdigit()));
    }
}
