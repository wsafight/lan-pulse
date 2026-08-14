use std::path::PathBuf;

use crate::{
    i18n::Language,
    service::{LogEntry, LogEvent, ServiceActivity, ServiceError, ServiceInfo, ServiceSnapshot},
};

use super::{
    ACCENT, BACKGROUND, UiState, app_icon_rgba, configure_style, control_host, diagnostics_text,
    format_audio, format_bytes, format_log_event, format_service_error, render_dashboard,
};

fn service_info() -> ServiceInfo {
    serde_json::from_str(
        r#"{"event":"ready","control_url":"http://192.168.1.5:4100","control_port":4100,"discovery_port":41000,"pin":"123456","audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1},"source":"configured:tone","direct_target":null}"#,
    )
    .unwrap()
}

#[test]
fn formats_byte_counts_at_unit_boundaries() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(1023), "1023 B");
    assert_eq!(format_bytes(1024), "1.0 KB");
    assert_eq!(format_bytes(1536), "1.5 KB");
    assert_eq!(format_bytes(1_048_576), "1.00 MB");
}

#[test]
fn formats_audio_and_control_host_from_service_info() {
    let info = service_info();

    assert_eq!(format_audio(&info), "48000 Hz / 2 ch / 5 ms");
    assert_eq!(control_host(&info), "192.168.1.5");
}

#[test]
fn control_host_falls_back_for_non_http_or_portless_url() {
    let mut info = service_info();
    info.control_url = "lanpulse.local".to_string();
    assert_eq!(control_host(&info), "lanpulse.local");

    info.control_url = "https://lanpulse.local:4100".to_string();
    assert_eq!(control_host(&info), "https://lanpulse.local");
}

#[test]
fn formats_all_log_event_variants() {
    let events = [
        LogEvent::ServicePath("/tmp/lanpulse-service".to_string()),
        LogEvent::Ready("http://127.0.0.1:4100".to_string()),
        LogEvent::Stopped,
        LogEvent::Exited("exit status: 0".to_string()),
        LogEvent::StatusError("offline".to_string()),
        LogEvent::StatusRestored,
        LogEvent::StartFailed(ServiceError::NotFound),
        LogEvent::RequestFailed("HTTP 409".to_string()),
        LogEvent::ServiceOutput("line".to_string()),
        LogEvent::AddressCopied,
        LogEvent::DiagnosticsCopied,
        LogEvent::DeviceDisconnected,
    ];

    let formatted: Vec<String> = events
        .iter()
        .map(|event| format_log_event(event, Language::En))
        .collect();

    assert!(formatted.iter().any(|line| line.contains("Service")));
    assert!(formatted.iter().any(|line| line.contains("Ready")));
    assert!(
        formatted
            .iter()
            .any(|line| line.contains("Service stopped"))
    );
    assert!(formatted.iter().any(|line| line.contains("exit status: 0")));
    assert!(formatted.iter().any(|line| line.contains("offline")));
    assert!(formatted.iter().any(|line| line.contains("restored")));
    assert!(
        formatted
            .iter()
            .any(|line| line.contains("lanpulse-service was not found"))
    );
    assert!(formatted.iter().any(|line| line.contains("HTTP 409")));
    assert!(formatted.iter().any(|line| line.contains("line")));
    assert!(formatted.iter().any(|line| line.contains("copied")));
    assert!(formatted.iter().any(|line| line.contains("disconnected")));
}

#[test]
fn formats_service_errors_for_each_error_kind() {
    assert!(format_service_error(&ServiceError::NotFound, Language::En).contains("not found"));
    assert!(
        format_service_error(
            &ServiceError::Spawn {
                path: PathBuf::from("/tmp/service"),
                error: "permission denied".to_string(),
            },
            Language::En,
        )
        .contains("permission denied")
    );
    assert!(
        format_service_error(&ServiceError::StdoutUnavailable, Language::En).contains("stdout")
    );
    assert!(
        format_service_error(
            &ServiceError::ReadyTimeout("timeout".to_string()),
            Language::En
        )
        .contains("timeout")
    );
    assert_eq!(
        format_service_error(&ServiceError::Request("offline".to_string()), Language::En),
        "offline"
    );
}

#[test]
fn diagnostics_text_includes_status_device_and_logs() {
    let snapshot = ServiceSnapshot {
        running: true,
        info: Some(service_info()),
        status: Some(serde_json::from_str(
            r#"{"ok":true,"audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1},"stats":{"target":"192.168.1.50:5504","device":{"name":"Phone","target":"192.168.1.50:5504","connected_at_ms":123},"media_source":"tone","packets_sent":7,"bytes_sent":2048,"capture_packets_dropped":2,"capture_restarts":1,"last_capture_error":"pipewire failed","rtp_send_errors":1,"last_rtp_error":"network unreachable","media_restarts":1,"last_media_error":"capture failed","media_started_ms":10,"last_packet_at_ms":20}}"#,
        )
        .unwrap()),
        status_error: Some("degraded".to_string()),
        activity: ServiceActivity::Idle,
        logs: vec![LogEntry {
            timestamp: "12:00:00".to_string(),
            event: LogEvent::Ready("http://127.0.0.1:4100".to_string()),
        }],
    };

    let diagnostics = diagnostics_text(&snapshot, Language::En);

    assert!(diagnostics.contains("running: true"));
    assert!(diagnostics.contains("status_error: degraded"));
    assert!(diagnostics.contains("control_url: http://192.168.1.5:4100"));
    assert!(diagnostics.contains("packets_sent: 7"));
    assert!(diagnostics.contains("capture_restarts: 1"));
    assert!(diagnostics.contains("last_capture_error: pipewire failed"));
    assert!(diagnostics.contains("rtp_send_errors: 1"));
    assert!(diagnostics.contains("last_rtp_error: network unreachable"));
    assert!(diagnostics.contains("last_media_error: capture failed"));
    assert!(diagnostics.contains("device: Phone (192.168.1.50:5504)"));
    assert!(diagnostics.contains("12:00:00  Ready"));
}

#[test]
fn ui_state_drafts_settings_when_opening_settings() {
    let settings = crate::settings::AppSettings {
        packet_ms: 20,
        ..crate::settings::AppSettings::default()
    };
    let mut state = UiState::new(&crate::settings::AppSettings::default());

    state.open_settings(&settings);

    assert!(state.show_settings);
    assert_eq!(state.settings_draft.packet_ms, 20);
}

#[test]
fn app_icon_has_expected_rgba_shape_and_transparency() {
    let icon = app_icon_rgba(16);
    let center_alpha = icon[(8 * 16 + 8) * 4 + 3];
    let corner_alpha = icon[3];

    assert_eq!(icon.len(), 16 * 16 * 4);
    assert_eq!(center_alpha, 255);
    assert_eq!(corner_alpha, 0);
}

#[test]
fn configure_style_applies_dark_spacing_type_and_palette() {
    let ctx = eframe::egui::Context::default();

    configure_style(&ctx);

    let style = ctx.style_of(eframe::egui::Theme::Dark);
    assert_eq!(
        style.spacing.item_spacing,
        eframe::egui::Vec2::new(8.0, 8.0)
    );
    assert_eq!(
        style.spacing.button_padding,
        eframe::egui::Vec2::new(12.0, 7.0)
    );
    assert_eq!(
        style
            .text_styles
            .get(&eframe::egui::TextStyle::Body)
            .unwrap()
            .size,
        14.0
    );
    assert_eq!(style.visuals.panel_fill, BACKGROUND);
    assert_eq!(style.visuals.window_fill, BACKGROUND);
    assert_eq!(style.visuals.selection.bg_fill, ACCENT);
    assert_eq!(style.visuals.widgets.active.bg_fill, ACCENT);
}

#[test]
fn renders_dashboard_with_status_settings_pairing_and_notice() {
    let ctx = eframe::egui::Context::default();
    let settings = crate::settings::AppSettings::default();
    let mut ui_state = UiState::new(&settings);
    ui_state.show_settings = true;
    ui_state.show_pairing = true;
    let snapshot = ServiceSnapshot {
        running: true,
        info: Some(service_info()),
        status: Some(serde_json::from_str(
            r#"{"ok":true,"audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1},"stats":{"target":"192.168.1.50:5504","device":{"name":"Phone","target":"192.168.1.50:5504","connected_at_ms":123},"media_source":"tone","packets_sent":7,"bytes_sent":2048,"capture_packets_dropped":2,"media_restarts":1,"last_media_error":null,"media_started_ms":10,"last_packet_at_ms":20}}"#,
        )
        .unwrap()),
        status_error: None,
        activity: ServiceActivity::Idle,
        logs: vec![LogEntry {
            timestamp: "12:00:00".to_string(),
            event: LogEvent::Ready("http://127.0.0.1:4100".to_string()),
        }],
    };

    let mut language_changed = None;
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let mut language = Language::En;
        let response = render_dashboard(
            ui,
            &snapshot,
            &mut language,
            &settings,
            &mut ui_state,
            Some("saved"),
        );
        language_changed = Some(response.language_changed);
    });
    output.textures_delta.clear();

    assert_eq!(language_changed, Some(false));
}

#[test]
fn renders_dashboard_for_stopped_and_degraded_states() {
    let ctx = eframe::egui::Context::default();
    let settings = crate::settings::AppSettings::default();
    let mut ui_state = UiState::new(&settings);
    let snapshot = ServiceSnapshot {
        running: true,
        info: None,
        status: None,
        status_error: Some("offline".to_string()),
        activity: ServiceActivity::Starting,
        logs: Vec::new(),
    };

    let mut output = ctx.run_ui(Default::default(), |ui| {
        let mut language = Language::ZhCn;
        let response =
            render_dashboard(ui, &snapshot, &mut language, &settings, &mut ui_state, None);
        assert!(!response.language_changed);
    });
    output.textures_delta.clear();
}
