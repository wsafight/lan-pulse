use eframe::egui::{
    self, Align2, Button, Color32, ComboBox, CursorIcon, DragValue, FontFamily, FontId, Label,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, Vec2,
};
use qrcode::{QrCode, types::Color as QrColor};

use crate::{
    i18n::Language,
    service::{LogEntry, ServiceActivity, ServiceSnapshot},
    settings::{AppSettings, AudioSource},
};

mod format;
mod style;
#[cfg(test)]
mod tests;

pub use format::{diagnostics_text, format_service_error};
pub use style::{app_icon_rgba, configure_fonts, configure_style};

use format::{control_host, format_audio, format_bytes, format_log_event};

const BACKGROUND: Color32 = Color32::from_rgb(16, 20, 23);
const SURFACE: Color32 = Color32::from_rgb(24, 29, 33);
const SURFACE_RAISED: Color32 = Color32::from_rgb(29, 35, 39);
const BORDER: Color32 = Color32::from_rgb(48, 57, 62);
const TEXT: Color32 = Color32::from_rgb(233, 238, 236);
const TEXT_MUTED: Color32 = Color32::from_rgb(148, 160, 165);
const ACCENT: Color32 = Color32::from_rgb(54, 190, 134);
const ACCENT_DARK: Color32 = Color32::from_rgb(7, 38, 27);
const WARNING: Color32 = Color32::from_rgb(222, 168, 75);
const STOP: Color32 = Color32::from_rgb(181, 75, 87);

#[derive(Debug, Clone)]
pub enum UiAction {
    ToggleService,
    Refresh,
    CopyAddress,
    ClearLogs,
    CopyDiagnostics,
    DisconnectDevice,
    ApplySettings(AppSettings),
}

#[derive(Debug, Clone)]
pub struct UiState {
    show_settings: bool,
    show_pairing: bool,
    settings_draft: AppSettings,
}

impl UiState {
    pub fn new(settings: &AppSettings) -> Self {
        Self {
            show_settings: false,
            show_pairing: false,
            settings_draft: settings.clone(),
        }
    }

    fn open_settings(&mut self, settings: &AppSettings) {
        self.settings_draft = settings.clone();
        self.show_settings = true;
    }
}

#[derive(Debug, Default)]
pub struct DashboardResponse {
    pub language_changed: bool,
    pub action: Option<UiAction>,
}

pub fn render_dashboard(
    root_ui: &mut egui::Ui,
    snapshot: &ServiceSnapshot,
    language: &mut Language,
    settings: &AppSettings,
    ui_state: &mut UiState,
    notice: Option<&str>,
) -> DashboardResponse {
    let previous_language = *language;
    let mut response = DashboardResponse::default();

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BACKGROUND)
                .inner_margin(egui::Margin::symmetric(20, 16)),
        )
        .show(root_ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    header(ui, snapshot, language);
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);

                    response.action = service_toolbar(ui, snapshot, *language, settings, ui_state);

                    if let Some(error) = snapshot.status_error.as_deref() {
                        ui.add_space(10.0);
                        degraded_banner(ui, error, *language);
                    }

                    ui.add_space(16.0);
                    status_grid(ui, snapshot, *language);
                    ui.add_space(16.0);

                    if let Some(action) = connected_device_panel(ui, snapshot, *language) {
                        response.action = Some(action);
                    }
                    ui.add_space(16.0);

                    if let Some(action) = log_panel(ui, &snapshot.logs, *language) {
                        response.action = Some(action);
                    }
                    ui.add_space(2.0);
                });
        });

    if let Some(action) = settings_window(root_ui.ctx(), ui_state, *language) {
        response.action = Some(action);
    }
    if let Some(action) = pairing_window(root_ui.ctx(), snapshot, ui_state, *language) {
        response.action = Some(action);
    }
    if let Some(notice) = notice {
        notice_toast(root_ui.ctx(), notice);
    }

    response.language_changed = *language != previous_language;
    response
}

fn header(ui: &mut egui::Ui, snapshot: &ServiceSnapshot, language: &mut Language) {
    let strings = language.strings();
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 52.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("LanPulse")
                        .font(FontId::new(27.0, FontFamily::Proportional))
                        .strong()
                        .color(TEXT),
                );
                ui.add_space(2.0);
                ui.label(RichText::new(strings.subtitle).size(13.0).color(TEXT_MUTED));
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(222.0, 34.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        status_indicator(ui, snapshot, *language);
                        ui.add_space(10.0);
                        language_selector(ui, language);
                    },
                );
            });
        },
    );
}

fn language_selector(ui: &mut egui::Ui, language: &mut Language) {
    const SELECTOR_WIDTH: f32 = 112.0;
    const SELECTOR_HEIGHT: f32 = 34.0;
    const OPTION_HEIGHT: f32 = 38.0;

    ui.spacing_mut().interact_size.y = SELECTOR_HEIGHT;
    let selected = match language {
        Language::En => "English",
        Language::ZhCn => "中文",
    };
    let (response, _) = egui::containers::menu::MenuButton::from_button(
        Button::new(selected)
            .right_text("  ")
            .min_size(Vec2::new(SELECTOR_WIDTH, SELECTOR_HEIGHT)),
    )
    .ui(ui, |ui| {
        ui.set_min_width(SELECTOR_WIDTH);
        ui.spacing_mut().interact_size.y = OPTION_HEIGHT;
        for (value, label) in [(Language::En, "English"), (Language::ZhCn, "中文")] {
            let option = ui
                .add_sized(
                    Vec2::new(SELECTOR_WIDTH, OPTION_HEIGHT),
                    Button::selectable(*language == value, label),
                )
                .on_hover_cursor(CursorIcon::PointingHand);
            if option.clicked() {
                *language = value;
                ui.close();
            }
        }
    });
    let icon_center = egui::pos2(response.rect.right() - 14.0, response.rect.center().y);
    let icon_stroke = Stroke::new(1.5, ui.style().interact(&response).fg_stroke.color);
    ui.painter().line_segment(
        [
            icon_center + Vec2::new(-4.0, -2.0),
            icon_center + Vec2::new(0.0, 2.0),
        ],
        icon_stroke,
    );
    ui.painter().line_segment(
        [
            icon_center + Vec2::new(0.0, 2.0),
            icon_center + Vec2::new(4.0, -2.0),
        ],
        icon_stroke,
    );
    response.on_hover_cursor(CursorIcon::PointingHand);
}

fn status_indicator(ui: &mut egui::Ui, snapshot: &ServiceSnapshot, language: Language) {
    let strings = language.strings();
    let (label, dot, fill, border) = match snapshot.activity {
        ServiceActivity::Starting => (
            strings.starting,
            WARNING,
            Color32::from_rgb(48, 39, 21),
            WARNING,
        ),
        ServiceActivity::Stopping => (
            strings.stopping,
            WARNING,
            Color32::from_rgb(48, 39, 21),
            WARNING,
        ),
        ServiceActivity::Disconnecting => (
            strings.disconnecting,
            WARNING,
            Color32::from_rgb(48, 39, 21),
            WARNING,
        ),
        ServiceActivity::Idle if snapshot.running && snapshot.status_error.is_some() => (
            strings.degraded,
            WARNING,
            Color32::from_rgb(48, 39, 21),
            WARNING,
        ),
        ServiceActivity::Idle if snapshot.running => (
            strings.running,
            ACCENT,
            Color32::from_rgb(21, 50, 38),
            ACCENT,
        ),
        ServiceActivity::Idle => (strings.stopped, TEXT_MUTED, SURFACE, BORDER),
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(100.0, 34.0), Sense::hover());
    ui.painter()
        .rect(rect, 6, fill, Stroke::new(1.0, border), StrokeKind::Inside);
    let dot_center = egui::pos2(rect.left() + 14.0, rect.center().y);
    ui.painter().circle_filled(dot_center, 4.0, dot);
    ui.painter().text(
        egui::pos2(rect.left() + 25.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::new(12.0, FontFamily::Proportional),
        TEXT,
    );
    if let Some(error) = snapshot.status_error.as_deref() {
        response.on_hover_text(error);
    }
}

fn service_toolbar(
    ui: &mut egui::Ui,
    snapshot: &ServiceSnapshot,
    language: Language,
    settings: &AppSettings,
    ui_state: &mut UiState,
) -> Option<UiAction> {
    let strings = language.strings();
    let mut action = None;
    let toolbar_content_width = (ui.available_width() - 20.0).max(0.0);

    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_width(toolbar_content_width);
            ui.horizontal_wrapped(|ui| {
                let (toggle_text, fill) = match snapshot.activity {
                    ServiceActivity::Starting => (strings.starting, WARNING),
                    ServiceActivity::Stopping => (strings.stopping, WARNING),
                    ServiceActivity::Disconnecting | ServiceActivity::Idle if snapshot.running => {
                        (strings.stop_service, STOP)
                    }
                    ServiceActivity::Disconnecting | ServiceActivity::Idle => {
                        (strings.start_service, ACCENT)
                    }
                };
                let toggle = ui.add_enabled(
                    !snapshot.is_busy(),
                    Button::new(RichText::new(toggle_text).strong().color(Color32::WHITE))
                        .min_size(Vec2::new(120.0, 38.0))
                        .fill(fill)
                        .stroke(Stroke::NONE)
                        .corner_radius(5),
                );
                if interactive(toggle, !snapshot.is_busy()).clicked() {
                    action = Some(UiAction::ToggleService);
                }

                let refresh = ui.add_enabled(
                    snapshot.running && !snapshot.is_busy(),
                    toolbar_button(strings.refresh, 84.0),
                );
                if interactive(refresh, snapshot.running && !snapshot.is_busy()).clicked() {
                    action = Some(UiAction::Refresh);
                }

                let has_info = snapshot.info.is_some();
                let copy = ui.add_enabled(has_info, toolbar_button(strings.copy_address, 108.0));
                if interactive(copy, has_info).clicked() {
                    action = Some(UiAction::CopyAddress);
                }

                let pair = ui.add_enabled(has_info, toolbar_button(strings.pair_device, 104.0));
                if interactive(pair, has_info).clicked() {
                    ui_state.show_pairing = true;
                }

                if interactive(ui.add(toolbar_button(strings.settings, 84.0)), true).clicked() {
                    ui_state.open_settings(settings);
                }
            });
        });

    action
}

fn toolbar_button(text: &str, width: f32) -> Button<'_> {
    Button::new(text)
        .min_size(Vec2::new(width, 38.0))
        .fill(SURFACE_RAISED)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(5)
}

fn interactive(response: egui::Response, enabled: bool) -> egui::Response {
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

fn degraded_banner(ui: &mut egui::Ui, error: &str, language: Language) {
    egui::Frame::new()
        .fill(Color32::from_rgb(48, 39, 21))
        .stroke(Stroke::new(1.0, WARNING))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(language.strings().degraded)
                        .strong()
                        .color(WARNING),
                );
                ui.add(Label::new(RichText::new(error).color(TEXT)).truncate());
            });
        });
}

fn status_grid(ui: &mut egui::Ui, snapshot: &ServiceSnapshot, language: Language) {
    let info = snapshot.info.as_ref();
    let stats = snapshot.status.as_ref().map(|status| &status.stats);
    let strings = language.strings();
    let metrics = [
        (
            "PIN",
            info.map(|info| info.pin.clone())
                .unwrap_or_else(|| "------".to_string()),
            true,
        ),
        (
            strings.discovery_port,
            info.and_then(|info| info.discovery_port)
                .map(|port| format!("UDP {port}"))
                .unwrap_or_else(|| "--".to_string()),
            false,
        ),
        (
            strings.control_address,
            info.map(control_host).unwrap_or_else(|| "--".to_string()),
            false,
        ),
        (
            strings.control_port,
            info.map(|info| format!("TCP {}", info.control_port))
                .unwrap_or_else(|| "--".to_string()),
            false,
        ),
        (
            strings.phone_target,
            stats
                .and_then(|stats| {
                    stats
                        .device
                        .as_ref()
                        .map(|device| device.name.clone())
                        .or_else(|| stats.target.clone())
                })
                .unwrap_or_else(|| strings.waiting_for_phone.to_string()),
            false,
        ),
        (
            strings.audio_source,
            stats
                .map(|stats| language.format_source(&stats.media_source))
                .or_else(|| info.map(|info| language.format_source(&info.source)))
                .unwrap_or_else(|| "--".to_string()),
            false,
        ),
        (
            strings.audio_format,
            info.map(format_audio).unwrap_or_else(|| "--".to_string()),
            false,
        ),
        (
            strings.rtp_packets,
            stats
                .map(|stats| stats.packets_sent.to_string())
                .unwrap_or_else(|| "0".to_string()),
            false,
        ),
        (
            strings.bytes_sent,
            stats
                .map(|stats| format_bytes(stats.bytes_sent))
                .unwrap_or_else(|| "0 B".to_string()),
            false,
        ),
    ];

    let available_width = ui.available_width();
    let columns: usize = if available_width >= 650.0 { 3 } else { 2 };
    let spacing = 10.0;
    let tile_width =
        (available_width - spacing * (columns.saturating_sub(1)) as f32) / columns as f32;
    let metric_count = metrics.len();

    egui::Grid::new("status-grid")
        .num_columns(columns)
        .spacing(Vec2::new(spacing, spacing))
        .show(ui, |ui| {
            for (index, (label, value, emphasized)) in metrics.into_iter().enumerate() {
                metric(ui, tile_width, label, value, emphasized);
                if (index + 1).is_multiple_of(columns) {
                    ui.end_row();
                }
            }
            if !metric_count.is_multiple_of(columns) {
                ui.end_row();
            }
        });
}

fn metric(ui: &mut egui::Ui, width: f32, label: &str, value: String, emphasized: bool) {
    let hover_value = value.clone();
    let response = egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            let content_width = (width - 24.0).max(80.0);
            ui.set_min_size(Vec2::new(content_width, 50.0));
            ui.set_max_width(content_width);
            ui.label(RichText::new(label).size(11.0).color(TEXT_MUTED));
            ui.add_space(4.0);
            ui.add(
                Label::new(
                    RichText::new(value)
                        .size(if emphasized { 20.0 } else { 16.0 })
                        .monospace()
                        .color(if emphasized { ACCENT } else { TEXT }),
                )
                .truncate(),
            );
        });
    response.response.on_hover_text(hover_value);
}

fn connected_device_panel(
    ui: &mut egui::Ui,
    snapshot: &ServiceSnapshot,
    language: Language,
) -> Option<UiAction> {
    let strings = language.strings();
    ui.label(
        RichText::new(strings.connected_device)
            .size(12.0)
            .strong()
            .color(TEXT_MUTED),
    );
    ui.add_space(6.0);

    let device = snapshot
        .status
        .as_ref()
        .and_then(|status| status.stats.device.as_ref());
    let Some(device) = device else {
        ui.label(RichText::new(strings.no_connected_device).color(TEXT_MUTED));
        return None;
    };

    let mut action = None;
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&device.name).strong().color(TEXT));
                    ui.label(RichText::new(&device.target).monospace().color(TEXT_MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let enabled = !snapshot.is_busy();
                    let button = ui.add_enabled(
                        enabled,
                        Button::new(strings.disconnect_device)
                            .min_size(Vec2::new(104.0, 34.0))
                            .fill(Color32::from_rgb(55, 31, 35))
                            .stroke(Stroke::new(1.0, STOP))
                            .corner_radius(5),
                    );
                    if interactive(button, enabled).clicked() {
                        action = Some(UiAction::DisconnectDevice);
                    }
                });
            });
        });
    action
}

fn log_panel(ui: &mut egui::Ui, logs: &[LogEntry], language: Language) -> Option<UiAction> {
    let strings = language.strings();
    let mut action = None;
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 30.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(
                RichText::new(strings.logs)
                    .size(12.0)
                    .strong()
                    .color(TEXT_MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = !logs.is_empty();
                let clear = ui.add_enabled(
                    enabled,
                    Button::new(strings.clear_logs).min_size(Vec2::new(62.0, 28.0)),
                );
                if interactive(clear, enabled).clicked() {
                    action = Some(UiAction::ClearLogs);
                }
                let copy = ui.add_enabled(
                    enabled,
                    Button::new(strings.copy_diagnostics).min_size(Vec2::new(132.0, 28.0)),
                );
                if interactive(copy, enabled).clicked() {
                    action = Some(UiAction::CopyDiagnostics);
                }
            });
        },
    );
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(Color32::from_rgb(13, 17, 19))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_height(100.0);
            ScrollArea::vertical()
                .max_height(104.0)
                .auto_shrink([false, false])
                .stick_to_bottom(false)
                .show(ui, |ui| {
                    for entry in logs {
                        let line = format!(
                            "{}  {}",
                            entry.timestamp,
                            format_log_event(&entry.event, language)
                        );
                        ui.add(Label::new(RichText::new(line).monospace().size(11.0)).truncate());
                    }
                });
        });
    action
}

fn settings_window(
    ctx: &egui::Context,
    ui_state: &mut UiState,
    language: Language,
) -> Option<UiAction> {
    if !ui_state.show_settings {
        return None;
    }
    let strings = language.strings();
    let mut action = None;
    let mut open = ui_state.show_settings;
    let mut close_after = false;

    egui::Window::new(strings.settings)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(390.0)
        .show(ctx, |ui| {
            ui.checkbox(
                &mut ui_state.settings_draft.start_service_on_launch,
                strings.start_on_launch,
            );
            ui.checkbox(
                &mut ui_state.settings_draft.minimize_to_tray,
                strings.minimize_to_tray,
            );
            ui.add_space(8.0);

            egui::Grid::new("settings-grid")
                .num_columns(2)
                .spacing(Vec2::new(14.0, 10.0))
                .show(ui, |ui| {
                    ui.label(strings.audio_source_setting);
                    ComboBox::from_id_salt("audio-source")
                        .selected_text(
                            language.format_source(ui_state.settings_draft.audio_source.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            for &source in AudioSource::available() {
                                ui.selectable_value(
                                    &mut ui_state.settings_draft.audio_source,
                                    source,
                                    language.format_source(source.as_str()),
                                );
                            }
                        });
                    ui.end_row();

                    ui.label(strings.packet_duration);
                    ComboBox::from_id_salt("packet-duration")
                        .selected_text(format!("{} ms", ui_state.settings_draft.packet_ms))
                        .show_ui(ui, |ui| {
                            for packet_ms in [5, 10, 20] {
                                ui.selectable_value(
                                    &mut ui_state.settings_draft.packet_ms,
                                    packet_ms,
                                    format!("{packet_ms} ms"),
                                );
                            }
                        });
                    ui.end_row();

                    port_range_row(
                        ui,
                        strings.control_port_range,
                        &mut ui_state.settings_draft.control_port_start,
                        &mut ui_state.settings_draft.control_port_end,
                        strings.port_range_separator,
                    );
                    port_range_row(
                        ui,
                        strings.discovery_port_range,
                        &mut ui_state.settings_draft.discovery_port_start,
                        &mut ui_state.settings_draft.discovery_port_end,
                        strings.port_range_separator,
                    );
                });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if interactive(
                    ui.add(Button::new(strings.save).min_size(Vec2::new(82.0, 34.0))),
                    true,
                )
                .clicked()
                {
                    let mut settings = ui_state.settings_draft.clone();
                    settings.sanitize();
                    action = Some(UiAction::ApplySettings(settings));
                    close_after = true;
                }
                if interactive(
                    ui.add(Button::new(strings.cancel).min_size(Vec2::new(82.0, 34.0))),
                    true,
                )
                .clicked()
                {
                    close_after = true;
                }
            });
        });

    ui_state.show_settings = open && !close_after;
    action
}

fn port_range_row(ui: &mut egui::Ui, label: &str, start: &mut u16, end: &mut u16, separator: &str) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.add(DragValue::new(start).range(1..=u16::MAX));
        ui.label(separator);
        ui.add(DragValue::new(end).range(1..=u16::MAX));
    });
    ui.end_row();
}

fn pairing_window(
    ctx: &egui::Context,
    snapshot: &ServiceSnapshot,
    ui_state: &mut UiState,
    language: Language,
) -> Option<UiAction> {
    if !ui_state.show_pairing {
        return None;
    }
    let Some(info) = snapshot.info.as_ref() else {
        ui_state.show_pairing = false;
        return None;
    };
    let strings = language.strings();
    let mut action = None;
    let mut open = ui_state.show_pairing;
    let mut close_after = false;

    egui::Window::new(strings.pair_device)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(strings.scan_to_pair).strong());
                ui.add_space(8.0);
                paint_qr(ui, &info.pairing_uri(), 220.0);
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("PIN  {}", info.pin))
                        .monospace()
                        .size(18.0),
                );
                ui.label(RichText::new(strings.pairing_address).color(TEXT_MUTED));
                ui.add(
                    Label::new(RichText::new(&info.control_url).monospace().color(TEXT)).truncate(),
                )
                .on_hover_text(&info.control_url);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if interactive(
                        ui.add(Button::new(strings.copy_address).min_size(Vec2::new(112.0, 34.0))),
                        true,
                    )
                    .clicked()
                    {
                        action = Some(UiAction::CopyAddress);
                    }
                    if interactive(
                        ui.add(Button::new(strings.close).min_size(Vec2::new(82.0, 34.0))),
                        true,
                    )
                    .clicked()
                    {
                        close_after = true;
                    }
                });
            });
        });

    ui_state.show_pairing = open && !close_after;
    action
}

fn paint_qr(ui: &mut egui::Ui, content: &str, size: f32) {
    let Ok(code) = QrCode::new(content.as_bytes()) else {
        return;
    };
    let width = code.width();
    let quiet_zone = 4usize;
    let cells = width + quiet_zone * 2;
    let cell_size = (size / cells as f32).floor().max(1.0);
    let actual_size = cell_size * cells as f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(actual_size), Sense::hover());
    ui.painter().rect_filled(rect, 2, Color32::WHITE);

    for y in 0..width {
        for x in 0..width {
            if code[(x, y)] == QrColor::Dark {
                let min = rect.min
                    + Vec2::new(
                        (x + quiet_zone) as f32 * cell_size,
                        (y + quiet_zone) as f32 * cell_size,
                    );
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(min, Vec2::splat(cell_size)),
                    0,
                    Color32::BLACK,
                );
            }
        }
    }
}

fn notice_toast(ctx: &egui::Context, notice: &str) {
    egui::Area::new(egui::Id::new("notice-toast"))
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-20.0, -20.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgb(24, 48, 38))
                .stroke(Stroke::new(1.0, ACCENT))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.label(RichText::new(notice).strong().color(TEXT));
                });
        });
}
