use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use lanpulse_service::{
    config::{AudioConfig, AudioSourceMode, Options, detect_lan_ip},
    control::{bind_first_available, router},
    discovery::{bind_first_available_udp, run_discovery_responder},
    media::{run_media_sender, summarize_stats},
    startup::print_ready,
    state::SessionState,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const INITIAL_MEDIA_RETRY_DELAY: Duration = Duration::from_millis(500);
const MAX_MEDIA_RETRY_DELAY: Duration = Duration::from_secs(5);
const MEDIA_RETRY_RESET_AFTER: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lanpulse_service=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    run(Options::from_env()?).await
}

async fn run(options: Options) -> Result<()> {
    let audio = options.audio_config();
    let state = Arc::new(SessionState::new(options.pin.clone(), audio.clone()));
    state
        .set_media_source(configured_media_source(options.source))
        .await;

    spawn_lease_expirer(Arc::clone(&state));
    let (listener, control_port) = bind_first_available(
        &options.host,
        options.control_port_start,
        options.control_port_end,
    )
    .await?;

    let lan_ip = detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let control_url = format_control_url(&lan_ip, control_port);
    spawn_media_supervisor(
        Arc::clone(&state),
        audio,
        options.target,
        options.tone_hz,
        options.source,
        options.pipewire_target.clone(),
    );

    let discovery_port = if options.discovery_enabled {
        let (discovery_socket, discovery_port) =
            bind_first_available_udp(options.discovery_port, options.discovery_port_end).await?;
        let discovery_state = Arc::clone(&state);
        let discovery_control_url = control_url.clone();
        tokio::spawn(async move {
            if let Err(err) = run_discovery_responder(
                discovery_state,
                discovery_socket,
                discovery_port,
                discovery_control_url,
                control_port,
            )
            .await
            {
                tracing::error!(%err, "LAN discovery responder stopped");
            }
        });
        Some(discovery_port)
    } else {
        None
    };

    print_ready(&options, &control_url, control_port, discovery_port)?;

    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("control server failed")?;

    Ok(())
}

fn spawn_lease_expirer(state: Arc<SessionState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if state.expire_stale_device().await {
                tracing::info!("expired inactive mobile session");
            }
        }
    });
}

fn spawn_media_supervisor(
    state: Arc<SessionState>,
    audio: AudioConfig,
    direct_target: Option<SocketAddr>,
    tone_hz: f32,
    source: AudioSourceMode,
    pipewire_target: Option<String>,
) {
    tokio::spawn(async move {
        let mut retry_delay = INITIAL_MEDIA_RETRY_DELAY;
        loop {
            let attempt_started = tokio::time::Instant::now();
            let result = run_media_sender(
                Arc::clone(&state),
                audio.clone(),
                direct_target,
                tone_hz,
                source,
                pipewire_target.clone(),
            )
            .await;
            let error = match result {
                Ok(()) => "media sender exited unexpectedly".to_string(),
                Err(error) => format!("{error:#}"),
            };
            tracing::error!(%error, ?retry_delay, "media sender stopped; restarting");
            state.record_media_failure(error).await;
            retry_delay = retry_delay_for_attempt(retry_delay, attempt_started.elapsed());
            tokio::time::sleep(retry_delay).await;
            retry_delay = next_retry_delay(retry_delay);
        }
    });
}

fn configured_media_source(source: AudioSourceMode) -> String {
    format!("configured:{}", source.as_str())
}

fn format_control_url(lan_ip: &str, control_port: u16) -> String {
    format!("http://{lan_ip}:{control_port}")
}

fn retry_delay_for_attempt(current: Duration, attempt_elapsed: Duration) -> Duration {
    if attempt_elapsed >= MEDIA_RETRY_RESET_AFTER {
        INITIAL_MEDIA_RETRY_DELAY
    } else {
        current
    }
}

fn next_retry_delay(current: Duration) -> Duration {
    (current * 2).min(MAX_MEDIA_RETRY_DELAY)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[allow(dead_code)]
async fn log_stats(state: Arc<SessionState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let stats = state.snapshot().await;
        tracing::info!("{}", summarize_stats(&stats));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lanpulse_service::config::AudioSourceMode;

    use super::{
        INITIAL_MEDIA_RETRY_DELAY, MAX_MEDIA_RETRY_DELAY, configured_media_source,
        format_control_url, next_retry_delay, retry_delay_for_attempt,
    };

    #[test]
    fn formats_configured_media_source_and_control_url() {
        assert_eq!(
            configured_media_source(AudioSourceMode::Tone),
            "configured:tone"
        );
        assert_eq!(
            configured_media_source(AudioSourceMode::ScreenCaptureKit),
            "configured:screencapturekit"
        );
        assert_eq!(
            format_control_url("192.168.1.20", 4100),
            "http://192.168.1.20:4100"
        );
    }

    #[test]
    fn media_retry_delay_backs_off_and_resets_after_long_attempts() {
        assert_eq!(
            retry_delay_for_attempt(Duration::from_secs(2), Duration::from_secs(5)),
            Duration::from_secs(2)
        );
        assert_eq!(
            retry_delay_for_attempt(Duration::from_secs(2), Duration::from_secs(30)),
            INITIAL_MEDIA_RETRY_DELAY
        );
        assert_eq!(
            next_retry_delay(Duration::from_millis(500)),
            Duration::from_secs(1)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(4)),
            MAX_MEDIA_RETRY_DELAY
        );
    }
}
