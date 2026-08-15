use std::{sync::mpsc, time::Duration, time::Instant};

use eframe::egui::{self, ViewportCommand};

use crate::{
    i18n::Language,
    service::{LogEvent, ServiceController, ServiceNotice},
    settings::AppSettings,
    tray::{self, TrayCommand, TrayUi},
    ui::{self, UiAction, UiState},
};

const LANGUAGE_STORAGE_KEY: &str = "language";
const SETTINGS_STORAGE_KEY: &str = "settings";
const NOTICE_DURATION: Duration = Duration::from_millis(2800);

struct Notice {
    text: String,
    expires_at: Instant,
}

pub struct LanPulseApp {
    service: ServiceController,
    tray: TrayUi,
    tray_rx: mpsc::Receiver<TrayCommand>,
    language: Language,
    settings: AppSettings,
    ui_state: UiState,
    notice: Option<Notice>,
    allow_close: bool,
}

impl LanPulseApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::configure_fonts(&cc.egui_ctx);
        ui::configure_style(&cc.egui_ctx);
        let language = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, LANGUAGE_STORAGE_KEY))
            .unwrap_or_default();
        let mut settings: AppSettings = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, SETTINGS_STORAGE_KEY))
            .unwrap_or_default();
        settings.sanitize();

        let (tray, tray_rx) = tray::install(&cc.egui_ctx, language);
        let mut service = ServiceController::new(cc.egui_ctx.clone());
        if settings.start_service_on_launch {
            service.start(&settings);
        }
        tray.set_state(
            language,
            service.snapshot().running,
            service.snapshot().is_busy(),
        );

        Self {
            service,
            tray,
            tray_rx,
            language,
            ui_state: UiState::new(&settings),
            settings,
            notice: None,
            allow_close: false,
        }
    }

    fn sync_service(&mut self) {
        self.service.pump();
        while let Some(notice) = self.service.take_notice() {
            self.show_notice(notice_text(notice, self.language));
        }
        let snapshot = self.service.snapshot();
        self.tray
            .set_state(self.language, snapshot.running, snapshot.is_busy());
    }

    fn toggle_service(&mut self) {
        if self.service.snapshot().running {
            self.service.stop();
        } else {
            self.service.start(&self.settings);
        }
        self.sync_service();
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        while let Ok(command) = self.tray_rx.try_recv() {
            match command {
                TrayCommand::Show => {
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                TrayCommand::Toggle => self.toggle_service(),
                TrayCommand::Quit => {
                    self.allow_close = true;
                    self.service.shutdown();
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }

    fn handle_ui_action(
        &mut self,
        action: UiAction,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
    ) {
        match action {
            UiAction::ToggleService => self.toggle_service(),
            UiAction::Refresh => {
                self.service.refresh();
            }
            UiAction::CopyAddress => {
                if let Some(url) = self
                    .service
                    .snapshot()
                    .info
                    .as_ref()
                    .map(|info| info.control_url.clone())
                {
                    ctx.copy_text(url);
                    self.service.push_log(LogEvent::AddressCopied);
                    self.show_notice(self.language.strings().address_copied.to_string());
                }
            }
            UiAction::ClearLogs => self.service.clear_logs(),
            UiAction::CopyDiagnostics => {
                let diagnostics = ui::diagnostics_text(self.service.snapshot(), self.language);
                ctx.copy_text(diagnostics);
                self.service.push_log(LogEvent::DiagnosticsCopied);
                self.show_notice(self.language.strings().diagnostics_copied.to_string());
            }
            UiAction::DisconnectDevice => self.service.disconnect_device(),
            UiAction::ApplySettings(settings) => {
                let text = settings_notice_text(
                    &self.settings,
                    &settings,
                    self.service.snapshot().running,
                    self.language,
                );
                self.settings = settings;
                persist_settings(frame, &self.settings);
                self.show_notice(text.to_string());
            }
        }
    }

    fn show_notice(&mut self, text: String) {
        self.notice = Some(Notice {
            text,
            expires_at: Instant::now() + NOTICE_DURATION,
        });
    }

    fn active_notice(&mut self) -> Option<&str> {
        if self
            .notice
            .as_ref()
            .is_some_and(|notice| Instant::now() >= notice.expires_at)
        {
            self.notice = None;
        }
        self.notice.as_ref().map(|notice| notice.text.as_str())
    }
}

impl eframe::App for LanPulseApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        tray::pump_platform_events();
        self.sync_service();
        self.handle_tray(ctx);

        ctx.request_repaint_after(repaint_interval(self.service.snapshot().is_busy()));
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let close_requested = ui.ctx().input(|input| input.viewport().close_requested());
        if should_minimize_to_tray(
            close_requested,
            self.allow_close,
            self.settings.minimize_to_tray,
            self.tray.is_available(),
        ) {
            ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
            ui.ctx().send_viewport_cmd(ViewportCommand::Visible(false));
        }

        let snapshot = self.service.snapshot().clone();
        let notice = self.active_notice().map(str::to_string);
        let response = ui::render_dashboard(
            ui,
            &snapshot,
            &mut self.language,
            &self.settings,
            &mut self.ui_state,
            notice.as_deref(),
        );
        if response.language_changed {
            let snapshot = self.service.snapshot();
            self.tray
                .set_state(self.language, snapshot.running, snapshot.is_busy());
            persist_language(frame, self.language);
        }
        if let Some(action) = response.action {
            self.handle_ui_action(action, ui.ctx(), frame);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LANGUAGE_STORAGE_KEY, &self.language);
        eframe::set_value(storage, SETTINGS_STORAGE_KEY, &self.settings);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.service.shutdown();
    }
}

fn persist_language(frame: &mut eframe::Frame, language: Language) {
    if let Some(storage) = frame.storage_mut() {
        eframe::set_value(storage, LANGUAGE_STORAGE_KEY, &language);
        storage.flush();
    }
}

fn persist_settings(frame: &mut eframe::Frame, settings: &AppSettings) {
    if let Some(storage) = frame.storage_mut() {
        eframe::set_value(storage, SETTINGS_STORAGE_KEY, settings);
        storage.flush();
    }
}

fn notice_text(notice: ServiceNotice, language: Language) -> String {
    let strings = language.strings();
    match notice {
        ServiceNotice::Started => strings.service_started.to_string(),
        ServiceNotice::Stopped => strings.service_stopped.to_string(),
        ServiceNotice::Disconnected => strings.device_disconnected.to_string(),
        ServiceNotice::Failed(error) => format!(
            "{}: {}",
            strings.operation_failed,
            ui::format_service_error(&error, language)
        ),
    }
}

fn settings_notice_text(
    current: &AppSettings,
    next: &AppSettings,
    service_running: bool,
    language: Language,
) -> &'static str {
    let strings = language.strings();
    if restart_required_after_settings_change(current, next, service_running) {
        strings.restart_required
    } else {
        strings.settings_saved
    }
}

fn restart_required_after_settings_change(
    current: &AppSettings,
    next: &AppSettings,
    service_running: bool,
) -> bool {
    current.service_options_changed(next) && service_running
}

fn repaint_interval(service_busy: bool) -> Duration {
    if service_busy {
        Duration::from_millis(80)
    } else {
        Duration::from_millis(300)
    }
}

fn should_minimize_to_tray(
    close_requested: bool,
    allow_close: bool,
    minimize_to_tray: bool,
    tray_available: bool,
) -> bool {
    close_requested && !allow_close && minimize_to_tray && tray_available
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::mpsc, time::Duration, time::Instant};

    use super::{
        LANGUAGE_STORAGE_KEY, LanPulseApp, SETTINGS_STORAGE_KEY, notice_text, repaint_interval,
        restart_required_after_settings_change, settings_notice_text, should_minimize_to_tray,
    };
    use crate::{
        i18n::Language,
        service::{ServiceController, ServiceError, ServiceNotice},
        settings::{AppSettings, AudioSource},
        tray::{self, TrayCommand},
        ui::UiState,
    };

    #[derive(Default)]
    struct MemoryStorage {
        values: HashMap<String, String>,
        flushed: bool,
    }

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.values.insert(key.to_string(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.values.remove(key);
        }

        fn flush(&mut self) {
            self.flushed = true;
        }
    }

    fn test_app_with_tray_rx(tray_rx: mpsc::Receiver<TrayCommand>) -> LanPulseApp {
        let settings = AppSettings::default();
        LanPulseApp {
            service: ServiceController::default(),
            tray: tray::TrayUi::unavailable_for_tests(Language::En),
            tray_rx,
            language: Language::En,
            ui_state: UiState::new(&settings),
            settings,
            notice: None,
            allow_close: false,
        }
    }

    fn test_app() -> LanPulseApp {
        let (_tray_tx, tray_rx) = mpsc::channel();
        test_app_with_tray_rx(tray_rx)
    }

    #[test]
    fn notice_text_localizes_success_and_failure_notices() {
        assert_eq!(
            notice_text(ServiceNotice::Started, Language::En),
            "Service started"
        );
        assert_eq!(
            notice_text(ServiceNotice::Stopped, Language::ZhCn),
            "服务已停止"
        );
        assert_eq!(
            notice_text(
                ServiceNotice::Failed(ServiceError::StdoutUnavailable),
                Language::ZhCn
            ),
            "操作失败: 后台服务 stdout 未打开"
        );
    }

    #[test]
    fn settings_notice_requests_restart_only_for_running_service_options() {
        let current = AppSettings::default();
        let changed_service_option = AppSettings {
            audio_source: AudioSource::Tone,
            ..current.clone()
        };
        let changed_window_option = AppSettings {
            minimize_to_tray: !current.minimize_to_tray,
            ..current.clone()
        };

        assert!(restart_required_after_settings_change(
            &current,
            &changed_service_option,
            true
        ));
        assert!(!restart_required_after_settings_change(
            &current,
            &changed_service_option,
            false
        ));
        assert!(!restart_required_after_settings_change(
            &current,
            &changed_window_option,
            true
        ));
        assert_eq!(
            settings_notice_text(&current, &changed_service_option, true, Language::En),
            "Service settings will apply on the next start"
        );
        assert_eq!(
            settings_notice_text(&current, &changed_window_option, true, Language::En),
            "Settings saved"
        );
    }

    #[test]
    fn repaint_interval_is_shorter_while_service_is_busy() {
        assert_eq!(repaint_interval(true), Duration::from_millis(80));
        assert_eq!(repaint_interval(false), Duration::from_millis(300));
    }

    #[test]
    fn minimize_to_tray_requires_close_request_and_enabled_tray() {
        assert!(should_minimize_to_tray(true, false, true, true));
        assert!(!should_minimize_to_tray(false, false, true, true));
        assert!(!should_minimize_to_tray(true, true, true, true));
        assert!(!should_minimize_to_tray(true, false, false, true));
        assert!(!should_minimize_to_tray(true, false, true, false));
    }

    #[test]
    fn active_notice_returns_current_notice_and_clears_expired_notice() {
        let mut app = test_app();

        app.show_notice("saved".to_string());
        assert_eq!(app.active_notice(), Some("saved"));

        app.notice.as_mut().unwrap().expires_at = Instant::now() - Duration::from_millis(1);

        assert_eq!(app.active_notice(), None);
        assert!(app.notice.is_none());
    }

    #[test]
    fn handle_tray_quit_allows_close_and_shuts_down_service() {
        let (tray_tx, tray_rx) = mpsc::channel();
        let mut app = test_app_with_tray_rx(tray_rx);
        let ctx = eframe::egui::Context::default();

        tray_tx.send(TrayCommand::Quit).unwrap();
        app.handle_tray(&ctx);

        assert!(app.allow_close);
        assert!(!app.service.snapshot().running);
    }

    #[test]
    fn save_persists_language_and_settings() {
        let mut app = test_app();
        let mut storage = MemoryStorage::default();
        app.language = Language::ZhCn;
        app.settings.packet_ms = 20;

        <LanPulseApp as eframe::App>::save(&mut app, &mut storage);

        let language: Language = eframe::get_value(&storage, LANGUAGE_STORAGE_KEY).unwrap();
        let settings: AppSettings = eframe::get_value(&storage, SETTINGS_STORAGE_KEY).unwrap();
        assert_eq!(language, Language::ZhCn);
        assert_eq!(settings.packet_ms, 20);
    }
}
