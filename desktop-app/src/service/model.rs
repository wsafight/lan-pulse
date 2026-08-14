use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub event: LogEvent,
}

#[derive(Debug, Clone)]
pub enum LogEvent {
    ServicePath(String),
    Ready(String),
    Stopped,
    Exited(String),
    StatusError(String),
    StatusRestored,
    StartFailed(ServiceError),
    RequestFailed(String),
    ServiceOutput(String),
    AddressCopied,
    DiagnosticsCopied,
    DeviceDisconnected,
}

#[derive(Debug, Clone)]
pub enum ServiceError {
    NotFound,
    Spawn { path: PathBuf, error: String },
    StdoutUnavailable,
    ReadyTimeout(String),
    Request(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ServiceActivity {
    #[default]
    Idle,
    Starting,
    Stopping,
    Disconnecting,
}

#[derive(Debug, Clone)]
pub enum ServiceNotice {
    Started,
    Stopped,
    Disconnected,
    Failed(ServiceError),
}

#[derive(Debug, Clone, Default)]
pub struct ServiceSnapshot {
    pub running: bool,
    pub info: Option<ServiceInfo>,
    pub status: Option<StatusResponse>,
    pub status_error: Option<String>,
    pub activity: ServiceActivity,
    pub logs: Vec<LogEntry>,
}

impl ServiceSnapshot {
    pub fn is_busy(&self) -> bool {
        self.activity != ServiceActivity::Idle
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    event: String,
    pub control_url: String,
    pub control_port: u16,
    pub discovery_port: Option<u16>,
    pub pin: String,
    pub audio: AudioConfig,
    pub source: String,
    direct_target: Option<String>,
}

impl ServiceInfo {
    pub(super) fn is_ready_event(&self) -> bool {
        self.event == "ready"
    }

    pub fn pairing_uri(&self) -> String {
        format!(
            "lanpulse://pair?url={}&pin={}",
            urlencoding::encode(&self.control_url),
            urlencoding::encode(&self.pin)
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    sample_format: String,
    pub packet_ms: u16,
    payload_type: u8,
    ssrc: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusResponse {
    #[serde(rename = "ok")]
    _ok: bool,
    #[serde(rename = "audio")]
    _audio: AudioConfig,
    pub stats: StatsSnapshot,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatsSnapshot {
    pub target: Option<String>,
    pub device: Option<ConnectedDevice>,
    pub media_source: String,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    #[serde(default)]
    pub capture_packets_dropped: u64,
    #[serde(default)]
    pub capture_restarts: u64,
    #[serde(default)]
    pub last_capture_error: Option<String>,
    #[serde(default)]
    pub rtp_send_errors: u64,
    #[serde(default)]
    pub last_rtp_error: Option<String>,
    #[serde(default)]
    pub media_restarts: u64,
    #[serde(default)]
    pub last_media_error: Option<String>,
    #[serde(rename = "media_started_ms")]
    _media_started_ms: Option<u64>,
    #[serde(rename = "last_packet_at_ms")]
    _last_packet_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectedDevice {
    pub name: String,
    pub target: String,
    pub connected_at_ms: u64,
}

pub(super) fn format_error(error: &ServiceError) -> String {
    match error {
        ServiceError::NotFound => "lanpulse-service was not found".to_string(),
        ServiceError::Spawn { path, error } => {
            format!("unable to start {}: {error}", path.display())
        }
        ServiceError::StdoutUnavailable => "service stdout is unavailable".to_string(),
        ServiceError::ReadyTimeout(error) => format!("service startup timed out: {error}"),
        ServiceError::Request(error) => error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ServiceActivity, ServiceError, ServiceInfo, ServiceSnapshot, StatusResponse, format_error,
    };

    #[test]
    fn pairing_uri_encodes_the_control_url() {
        let info: ServiceInfo = serde_json::from_str(
            r#"{"event":"ready","control_url":"http://192.168.1.5:4100","control_port":4100,"discovery_port":41000,"pin":"123456","audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":10,"payload_type":96,"ssrc":1},"source":"auto","direct_target":null}"#,
        )
        .unwrap();

        assert!(info.is_ready_event());
        assert_eq!(
            info.pairing_uri(),
            "lanpulse://pair?url=http%3A%2F%2F192.168.1.5%3A4100&pin=123456"
        );
    }

    #[test]
    fn snapshot_is_busy_only_for_active_operations() {
        assert!(!ServiceSnapshot::default().is_busy());

        let snapshot = ServiceSnapshot {
            activity: ServiceActivity::Starting,
            ..ServiceSnapshot::default()
        };
        assert!(snapshot.is_busy());
    }

    #[test]
    fn formats_worker_errors_for_logs() {
        assert_eq!(
            format_error(&ServiceError::Spawn {
                path: PathBuf::from("/tmp/lanpulse-service"),
                error: "permission denied".to_string()
            }),
            "unable to start /tmp/lanpulse-service: permission denied"
        );
        assert_eq!(
            format_error(&ServiceError::Request("offline".to_string())),
            "offline"
        );
    }

    #[test]
    fn status_response_defaults_newer_stats_fields_when_absent() {
        let status: StatusResponse = serde_json::from_str(
            r#"{"ok":true,"audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1},"stats":{"target":null,"device":null,"media_source":"tone","packets_sent":3,"bytes_sent":2880,"media_started_ms":10,"last_packet_at_ms":20}}"#,
        )
        .unwrap();

        assert_eq!(status.stats.media_source, "tone");
        assert_eq!(status.stats.packets_sent, 3);
        assert_eq!(status.stats.bytes_sent, 2880);
        assert_eq!(status.stats.capture_packets_dropped, 0);
        assert_eq!(status.stats.capture_restarts, 0);
        assert!(status.stats.last_capture_error.is_none());
        assert_eq!(status.stats.rtp_send_errors, 0);
        assert!(status.stats.last_rtp_error.is_none());
        assert_eq!(status.stats.media_restarts, 0);
        assert!(status.stats.last_media_error.is_none());
    }
}
