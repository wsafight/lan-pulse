use crate::{
    i18n::Language,
    service::{LogEvent, ServiceError, ServiceInfo, ServiceSnapshot},
};

pub fn diagnostics_text(snapshot: &ServiceSnapshot, language: Language) -> String {
    let mut lines = vec![
        "LanPulse diagnostics".to_string(),
        format!("running: {}", snapshot.running),
        format!("activity: {:?}", snapshot.activity),
    ];
    if let Some(error) = snapshot.status_error.as_deref() {
        lines.push(format!("status_error: {error}"));
    }
    if let Some(info) = snapshot.info.as_ref() {
        lines.push(format!("control_url: {}", info.control_url));
        lines.push(format!("control_port: {}", info.control_port));
        lines.push(format!("discovery_port: {:?}", info.discovery_port));
        lines.push(format!("source: {}", info.source));
    }
    if let Some(status) = snapshot.status.as_ref() {
        lines.push(format!("target: {:?}", status.stats.target));
        lines.push(format!("media_source: {}", status.stats.media_source));
        lines.push(format!("packets_sent: {}", status.stats.packets_sent));
        lines.push(format!("bytes_sent: {}", status.stats.bytes_sent));
        lines.push(format!(
            "capture_packets_dropped: {}",
            status.stats.capture_packets_dropped
        ));
        lines.push(format!(
            "capture_restarts: {}",
            status.stats.capture_restarts
        ));
        if let Some(error) = status.stats.last_capture_error.as_deref() {
            lines.push(format!("last_capture_error: {error}"));
        }
        lines.push(format!("rtp_send_errors: {}", status.stats.rtp_send_errors));
        if let Some(error) = status.stats.last_rtp_error.as_deref() {
            lines.push(format!("last_rtp_error: {error}"));
        }
        lines.push(format!("media_restarts: {}", status.stats.media_restarts));
        if let Some(error) = status.stats.last_media_error.as_deref() {
            lines.push(format!("last_media_error: {error}"));
        }
        if let Some(device) = status.stats.device.as_ref() {
            lines.push(format!("device: {} ({})", device.name, device.target));
            lines.push(format!("connected_at_ms: {}", device.connected_at_ms));
        }
    }
    lines.push(String::new());
    lines.push("Logs".to_string());
    for entry in &snapshot.logs {
        lines.push(format!(
            "{}  {}",
            entry.timestamp,
            format_log_event(&entry.event, language)
        ));
    }
    lines.join("\n")
}

pub(super) fn format_log_event(event: &LogEvent, language: Language) -> String {
    let strings = language.strings();
    match event {
        LogEvent::ServicePath(path) => format!("{}: {path}", strings.service_path),
        LogEvent::Ready(url) => format!("{}: {url}", strings.service_ready),
        LogEvent::Stopped => strings.service_stopped.to_string(),
        LogEvent::Exited(status) => format!("{}: {status}", strings.service_exited),
        LogEvent::StatusError(error) => format!("{}: {error}", strings.service_status_error),
        LogEvent::StatusRestored => strings.service_status_restored.to_string(),
        LogEvent::StartFailed(error) => format!(
            "{}: {}",
            strings.start_failed,
            format_service_error(error, language)
        ),
        LogEvent::RequestFailed(error) => format!("{}: {error}", strings.operation_failed),
        LogEvent::ServiceOutput(line) => format!("{}: {line}", strings.service_output),
        LogEvent::AddressCopied => strings.address_copied.to_string(),
        LogEvent::DiagnosticsCopied => strings.diagnostics_copied.to_string(),
        LogEvent::DeviceDisconnected => strings.device_disconnected.to_string(),
    }
}

pub fn format_service_error(error: &ServiceError, language: Language) -> String {
    let strings = language.strings();
    match error {
        ServiceError::NotFound => strings.service_not_found.to_string(),
        ServiceError::Spawn { path, error } => {
            format!("{} {}: {error}", strings.unable_to_start, path.display())
        }
        ServiceError::StdoutUnavailable => strings.service_stdout_unavailable.to_string(),
        ServiceError::ReadyTimeout(error) => {
            format!("{}: {error}", strings.service_ready_timeout)
        }
        ServiceError::Request(error) => error.clone(),
    }
}

pub(super) fn format_audio(info: &ServiceInfo) -> String {
    format!(
        "{} Hz / {} ch / {} ms",
        info.audio.sample_rate, info.audio.channels, info.audio.packet_ms
    )
}

pub(super) fn control_host(info: &ServiceInfo) -> String {
    info.control_url
        .strip_prefix("http://")
        .unwrap_or(&info.control_url)
        .rsplit_once(':')
        .map(|(host, _port)| host)
        .unwrap_or(&info.control_url)
        .to_string()
}

pub(super) fn format_bytes(value: u64) -> String {
    if value < 1024 {
        format!("{value} B")
    } else if value < 1024 * 1024 {
        format!("{:.1} KB", value as f64 / 1024.0)
    } else {
        format!("{:.2} MB", value as f64 / 1024.0 / 1024.0)
    }
}
