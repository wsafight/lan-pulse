use std::{
    net::SocketAddr,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::sync::RwLock;

use crate::config::AudioConfig;

pub struct SessionState {
    pin: String,
    pin_created_at: Mutex<Instant>,
    pairing_policy: PairingPolicy,
    pairing_security: RwLock<PairingSecurity>,
    audio: AudioConfig,
    target: RwLock<Option<SocketAddr>>,
    target_generation: AtomicU64,
    device: RwLock<Option<ActiveDevice>>,
    media_source: RwLock<String>,
    stats: RwLock<Stats>,
    packets_sent: AtomicU64,
    bytes_sent: AtomicU64,
    media_started_ms: AtomicU64,
    last_packet_at_ms: AtomicU64,
    lease_timeout: Duration,
}

const DEFAULT_LEASE_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_PIN_TTL: Duration = Duration::from_secs(60);
const DEFAULT_PAIRING_BLOCK_DURATION: Duration = Duration::from_secs(30);
const DEFAULT_MAX_PAIRING_FAILURES: u32 = 5;

#[derive(Debug, Clone, Copy)]
struct PairingPolicy {
    pin_ttl: Duration,
    max_failures: u32,
    block_duration: Duration,
}

impl Default for PairingPolicy {
    fn default() -> Self {
        Self {
            pin_ttl: DEFAULT_PIN_TTL,
            max_failures: DEFAULT_MAX_PAIRING_FAILURES,
            block_duration: DEFAULT_PAIRING_BLOCK_DURATION,
        }
    }
}

#[derive(Debug, Default)]
struct PairingSecurity {
    failed_attempts: u32,
    blocked_until: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingPinResult {
    Accepted,
    Invalid,
    Expired,
    Blocked,
}

struct ActiveDevice {
    info: ConnectedDevice,
    refreshed_at: Instant,
}

#[derive(Debug, Clone, Default)]
struct Stats {
    capture_packets_dropped: u64,
    capture_restarts: u64,
    last_capture_error: Option<String>,
    rtp_send_errors: u64,
    last_rtp_error: Option<String>,
    media_restarts: u64,
    last_media_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsSnapshot {
    pub target: Option<SocketAddr>,
    pub device: Option<ConnectedDevice>,
    pub media_source: String,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub capture_packets_dropped: u64,
    pub capture_restarts: u64,
    pub last_capture_error: Option<String>,
    pub rtp_send_errors: u64,
    pub last_rtp_error: Option<String>,
    pub media_restarts: u64,
    pub last_media_error: Option<String>,
    pub media_started_ms: Option<u64>,
    pub last_packet_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectedDevice {
    pub client_id: String,
    pub session_id: String,
    pub name: String,
    pub target: SocketAddr,
    pub connected_at_ms: u64,
}

impl ConnectedDevice {
    pub fn new(client_id: String, session_id: String, name: String, target: SocketAddr) -> Self {
        let connected_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            client_id,
            session_id,
            name,
            target,
            connected_at_ms,
        }
    }
}

impl SessionState {
    pub fn new(pin: String, audio: AudioConfig) -> Self {
        Self::with_lease_timeout(pin, audio, DEFAULT_LEASE_TIMEOUT)
    }

    fn with_lease_timeout(pin: String, audio: AudioConfig, lease_timeout: Duration) -> Self {
        Self::with_policy(pin, audio, lease_timeout, PairingPolicy::default())
    }

    fn with_policy(
        pin: String,
        audio: AudioConfig,
        lease_timeout: Duration,
        pairing_policy: PairingPolicy,
    ) -> Self {
        Self {
            pin,
            pin_created_at: Mutex::new(Instant::now()),
            pairing_policy,
            pairing_security: RwLock::new(PairingSecurity::default()),
            audio,
            target: RwLock::new(None),
            target_generation: AtomicU64::new(0),
            device: RwLock::new(None),
            media_source: RwLock::new("idle".to_string()),
            stats: RwLock::new(Stats::default()),
            packets_sent: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            media_started_ms: AtomicU64::new(0),
            last_packet_at_ms: AtomicU64::new(0),
            lease_timeout,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests(
        pin: String,
        audio: AudioConfig,
        lease_timeout: Duration,
        pin_ttl: Duration,
        max_failures: u32,
        block_duration: Duration,
    ) -> Self {
        Self::with_policy(
            pin,
            audio,
            lease_timeout,
            PairingPolicy {
                pin_ttl,
                max_failures,
                block_duration,
            },
        )
    }

    pub fn pin_matches(&self, pin: &str) -> bool {
        self.pin == pin
    }

    pub fn refresh_pairing_pin(&self) {
        *self
            .pin_created_at
            .lock()
            .expect("pairing PIN timestamp lock poisoned") = Instant::now();
    }

    pub async fn authorize_pairing_pin(&self, pin: &str) -> PairingPinResult {
        let now = Instant::now();
        let mut security = self.pairing_security.write().await;
        if security
            .blocked_until
            .is_some_and(|blocked_until| now < blocked_until)
        {
            return PairingPinResult::Blocked;
        }
        let pin_created_at = *self
            .pin_created_at
            .lock()
            .expect("pairing PIN timestamp lock poisoned");
        if now.duration_since(pin_created_at) >= self.pairing_policy.pin_ttl {
            return PairingPinResult::Expired;
        }
        if self.pin == pin {
            security.failed_attempts = 0;
            security.blocked_until = None;
            return PairingPinResult::Accepted;
        }

        security.failed_attempts += 1;
        if security.failed_attempts >= self.pairing_policy.max_failures {
            security.blocked_until = Some(now + self.pairing_policy.block_duration);
            security.failed_attempts = 0;
            PairingPinResult::Blocked
        } else {
            PairingPinResult::Invalid
        }
    }

    pub fn audio_config(&self) -> &AudioConfig {
        &self.audio
    }

    pub async fn set_target(&self, target: Option<SocketAddr>) {
        *self.target.write().await = target;
        self.bump_target_generation();
    }

    pub async fn target(&self) -> Option<SocketAddr> {
        *self.target.read().await
    }

    pub fn target_generation(&self) -> u64 {
        self.target_generation.load(Ordering::Acquire)
    }

    pub async fn connect_device(&self, device: ConnectedDevice) -> bool {
        let mut current = self.device.write().await;
        if current.as_ref().is_some_and(|active| {
            active.refreshed_at.elapsed() < self.lease_timeout
                && active.info.client_id != device.client_id
        }) {
            return false;
        }

        *self.target.write().await = Some(device.target);
        self.bump_target_generation();
        *current = Some(ActiveDevice {
            info: device,
            refreshed_at: Instant::now(),
        });
        true
    }

    pub async fn resume_device(
        &self,
        client_id: &str,
        session_id: &str,
        name: String,
        target: SocketAddr,
    ) -> bool {
        let mut current = self.device.write().await;
        let Some(active) = current.as_mut() else {
            return false;
        };
        if active.refreshed_at.elapsed() >= self.lease_timeout {
            *self.target.write().await = None;
            self.bump_target_generation();
            *current = None;
            return false;
        }
        if active.info.client_id != client_id || active.info.session_id != session_id {
            return false;
        }

        active.info.name = name;
        active.info.target = target;
        active.refreshed_at = Instant::now();
        *self.target.write().await = Some(target);
        self.bump_target_generation();
        true
    }

    pub async fn refresh_session(&self, session_id: &str) -> bool {
        let mut current = self.device.write().await;
        let Some(active) = current.as_mut() else {
            return false;
        };
        if active.refreshed_at.elapsed() >= self.lease_timeout {
            *self.target.write().await = None;
            self.bump_target_generation();
            *current = None;
            return false;
        }
        if active.info.session_id != session_id {
            return false;
        }

        active.refreshed_at = Instant::now();
        true
    }

    pub async fn expire_stale_device(&self) -> bool {
        let mut current = self.device.write().await;
        if current
            .as_ref()
            .is_none_or(|active| active.refreshed_at.elapsed() < self.lease_timeout)
        {
            return false;
        }

        *self.target.write().await = None;
        self.bump_target_generation();
        *current = None;
        true
    }

    pub async fn disconnect_device(&self, session_id: Option<&str>) -> bool {
        let mut current = self.device.write().await;
        if let Some(session_id) = session_id
            && current
                .as_ref()
                .is_some_and(|active| active.info.session_id != session_id)
        {
            return false;
        }

        *self.target.write().await = None;
        self.bump_target_generation();
        *current = None;
        true
    }

    pub async fn device(&self) -> Option<ConnectedDevice> {
        self.device
            .read()
            .await
            .as_ref()
            .map(|active| active.info.clone())
    }

    pub async fn set_media_source(&self, source: impl Into<String>) {
        *self.media_source.write().await = source.into();
    }

    pub async fn media_source(&self) -> String {
        self.media_source.read().await.clone()
    }

    pub fn record_packet(&self, bytes: u64, elapsed: Duration) {
        self.record_packets(1, bytes, elapsed);
    }

    pub fn record_packets(&self, packets: u64, bytes: u64, elapsed: Duration) {
        if packets == 0 {
            return;
        }
        self.packets_sent.fetch_add(packets, Ordering::Relaxed);
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);

        let encoded_ms = (elapsed.as_millis() as u64).saturating_add(1);
        let _ = self.media_started_ms.compare_exchange(
            0,
            encoded_ms,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        self.last_packet_at_ms.store(encoded_ms, Ordering::Relaxed);
    }

    pub async fn record_capture_dropped(&self, count: u64) {
        self.stats.write().await.capture_packets_dropped += count;
    }

    pub async fn record_capture_restart(&self, error: String) {
        let mut stats = self.stats.write().await;
        stats.capture_restarts += 1;
        stats.last_capture_error = Some(error);
    }

    pub async fn record_rtp_send_error(&self, error: String) {
        let mut stats = self.stats.write().await;
        stats.rtp_send_errors += 1;
        stats.last_rtp_error = Some(error);
    }

    pub async fn record_media_failure(&self, error: String) {
        let mut stats = self.stats.write().await;
        stats.media_restarts += 1;
        stats.last_media_error = Some(error);
    }

    pub async fn mark_media_running(&self) {
        self.stats.write().await.last_media_error = None;
    }

    fn bump_target_generation(&self) {
        self.target_generation.fetch_add(1, Ordering::Release);
    }

    pub async fn snapshot(&self) -> StatsSnapshot {
        let stats = self.stats.read().await.clone();
        StatsSnapshot {
            target: self.target().await,
            device: self.device().await,
            media_source: self.media_source().await,
            packets_sent: self.packets_sent.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            capture_packets_dropped: stats.capture_packets_dropped,
            capture_restarts: stats.capture_restarts,
            last_capture_error: stats.last_capture_error,
            rtp_send_errors: stats.rtp_send_errors,
            last_rtp_error: stats.last_rtp_error,
            media_restarts: stats.media_restarts,
            last_media_error: stats.last_media_error,
            media_started_ms: decode_optional_millis(self.media_started_ms.load(Ordering::Relaxed)),
            last_packet_at_ms: decode_optional_millis(
                self.last_packet_at_ms.load(Ordering::Relaxed),
            ),
        }
    }
}

fn decode_optional_millis(encoded: u64) -> Option<u64> {
    encoded.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        time::Duration,
    };

    use crate::config::AudioConfig;

    use super::{ConnectedDevice, PairingPinResult, PairingPolicy, SessionState};

    fn state() -> SessionState {
        SessionState::new(
            "123456".to_string(),
            AudioConfig {
                sample_rate: 48_000,
                channels: 2,
                sample_format: "s16le".to_string(),
                packet_ms: 5,
                payload_type: 96,
                ssrc: 1,
            },
        )
    }

    fn device(client_id: &str, session_id: &str, port: u16) -> ConnectedDevice {
        ConnectedDevice::new(
            client_id.to_string(),
            session_id.to_string(),
            "phone".to_string(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        )
    }

    #[tokio::test]
    async fn rejects_another_client_but_allows_same_client_to_reconnect() {
        let state = state();
        assert!(
            state
                .connect_device(device("phone-a", "session-1", 5001))
                .await
        );
        assert!(
            !state
                .connect_device(device("phone-b", "session-2", 5002))
                .await
        );
        assert!(
            state
                .connect_device(device("phone-a", "session-3", 5003))
                .await
        );
        assert_eq!(state.device().await.unwrap().session_id, "session-3");
    }

    #[test]
    fn pin_matches_exact_pin_only() {
        let state = state();

        assert!(state.pin_matches("123456"));
        assert!(!state.pin_matches("12345"));
        assert!(!state.pin_matches("123456 "));
    }

    #[tokio::test]
    async fn pairing_pin_authorization_accepts_valid_pin_and_tracks_failures() {
        let state = SessionState::with_policy(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_secs(15),
            PairingPolicy {
                pin_ttl: Duration::from_secs(60),
                max_failures: 2,
                block_duration: Duration::from_millis(50),
            },
        );

        assert_eq!(
            state.authorize_pairing_pin("000000").await,
            PairingPinResult::Invalid
        );
        assert_eq!(
            state.authorize_pairing_pin("111111").await,
            PairingPinResult::Blocked
        );
        assert_eq!(
            state.authorize_pairing_pin("123456").await,
            PairingPinResult::Blocked
        );

        tokio::time::sleep(Duration::from_millis(60)).await;

        assert_eq!(
            state.authorize_pairing_pin("123456").await,
            PairingPinResult::Accepted
        );
    }

    #[tokio::test]
    async fn pairing_pin_expires_for_new_pairing_attempts() {
        let state = SessionState::with_policy(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_secs(15),
            PairingPolicy {
                pin_ttl: Duration::from_millis(10),
                max_failures: 5,
                block_duration: Duration::from_secs(1),
            },
        );

        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(
            state.authorize_pairing_pin("123456").await,
            PairingPinResult::Expired
        );
    }

    #[tokio::test]
    async fn refreshing_pairing_pin_renews_its_lifetime() {
        let state = SessionState::with_policy(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_secs(15),
            PairingPolicy {
                pin_ttl: Duration::from_millis(10),
                max_failures: 5,
                block_duration: Duration::from_secs(1),
            },
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        state.refresh_pairing_pin();

        assert_eq!(
            state.authorize_pairing_pin("123456").await,
            PairingPinResult::Accepted
        );
    }

    #[tokio::test]
    async fn target_and_media_source_are_reflected_in_snapshot() {
        let state = state();
        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)), 5504);

        state.set_target(Some(target)).await;
        state.set_media_source("configured:tone").await;

        let snapshot = state.snapshot().await;
        assert_eq!(snapshot.target, Some(target));
        assert_eq!(snapshot.media_source, "configured:tone");
        assert_eq!(state.media_source().await, "configured:tone");
    }

    #[tokio::test]
    async fn stale_session_cannot_disconnect_replacement_session() {
        let state = state();
        assert!(
            state
                .connect_device(device("phone-a", "session-1", 5001))
                .await
        );
        assert!(
            state
                .connect_device(device("phone-a", "session-2", 5002))
                .await
        );

        assert!(!state.disconnect_device(Some("session-1")).await);
        assert_eq!(state.device().await.unwrap().session_id, "session-2");
        assert!(state.disconnect_device(Some("session-2")).await);
        assert!(state.device().await.is_none());
    }

    #[tokio::test]
    async fn heartbeat_refreshes_active_session_and_rejects_stale_session() {
        let state = state();
        assert!(
            state
                .connect_device(device("phone-a", "session-1", 5001))
                .await
        );

        assert!(state.refresh_session("session-1").await);
        assert!(!state.refresh_session("session-old").await);
        assert_eq!(state.device().await.unwrap().session_id, "session-1");
    }

    #[tokio::test]
    async fn resume_updates_target_for_active_session_without_repairing() {
        let state = state();
        assert!(
            state
                .connect_device(device("phone-a", "session-1", 5001))
                .await
        );
        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5002);

        assert!(
            state
                .resume_device("phone-a", "session-1", "Phone A".to_string(), target)
                .await
        );

        assert_eq!(state.target().await, Some(target));
        assert_eq!(state.device().await.unwrap().target, target);
    }

    #[tokio::test]
    async fn resume_rejects_wrong_or_expired_session() {
        let state = SessionState::with_lease_timeout(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_millis(10),
        );
        assert!(
            state
                .connect_device(device("phone-a", "session-1", 5001))
                .await
        );
        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5002);

        assert!(
            !state
                .resume_device("phone-b", "session-1", "Phone B".to_string(), target)
                .await
        );
        assert_eq!(
            state.target().await,
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5001))
        );

        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(
            !state
                .resume_device("phone-a", "session-1", "Phone A".to_string(), target)
                .await
        );
        assert!(state.device().await.is_none());
        assert!(state.target().await.is_none());
    }

    #[tokio::test]
    async fn expired_lease_releases_target_for_another_client() {
        let state = SessionState::with_lease_timeout(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_millis(10),
        );
        assert!(
            state
                .connect_device(device("phone-a", "session-1", 5001))
                .await
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(state.expire_stale_device().await);
        assert!(state.target().await.is_none());
        assert!(
            state
                .connect_device(device("phone-b", "session-2", 5002))
                .await
        );
    }

    #[tokio::test]
    async fn records_packet_drop_and_media_failure_stats() {
        let state = state();

        state.record_packet(128, Duration::from_millis(25));
        state.record_capture_dropped(3).await;
        state
            .record_capture_restart("pipewire failed".to_string())
            .await;
        state
            .record_rtp_send_error("network unreachable".to_string())
            .await;
        state
            .record_media_failure("capture failed".to_string())
            .await;

        let snapshot = state.snapshot().await;
        assert_eq!(snapshot.packets_sent, 1);
        assert_eq!(snapshot.bytes_sent, 128);
        assert_eq!(snapshot.capture_packets_dropped, 3);
        assert_eq!(snapshot.capture_restarts, 1);
        assert_eq!(
            snapshot.last_capture_error.as_deref(),
            Some("pipewire failed")
        );
        assert_eq!(snapshot.rtp_send_errors, 1);
        assert_eq!(
            snapshot.last_rtp_error.as_deref(),
            Some("network unreachable")
        );
        assert_eq!(snapshot.media_restarts, 1);
        assert_eq!(snapshot.last_media_error.as_deref(), Some("capture failed"));
        assert_eq!(snapshot.media_started_ms, Some(25));
        assert_eq!(snapshot.last_packet_at_ms, Some(25));

        state.mark_media_running().await;
        assert!(state.snapshot().await.last_media_error.is_none());
    }

    #[tokio::test]
    async fn disconnect_without_session_clears_any_active_device() {
        let state = state();
        assert!(
            state
                .connect_device(device("phone-a", "session-1", 5001))
                .await
        );

        assert!(state.disconnect_device(None).await);

        assert!(state.device().await.is_none());
        assert!(state.target().await.is_none());
    }

    #[tokio::test]
    async fn expired_refresh_clears_device_and_target() {
        let state = SessionState::with_lease_timeout(
            "123456".to_string(),
            state().audio_config().clone(),
            Duration::from_millis(10),
        );
        assert!(
            state
                .connect_device(device("phone-a", "session-1", 5001))
                .await
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(!state.refresh_session("session-1").await);
        assert!(state.device().await.is_none());
        assert!(state.target().await.is_none());
    }
}
