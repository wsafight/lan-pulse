use std::sync::mpsc;

use eframe::egui;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

use crate::{i18n::Language, ui::app_icon_rgba};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    Toggle,
    Quit,
}

pub struct TrayUi {
    _icon: Option<TrayIcon>,
    show: MenuItem,
    toggle: MenuItem,
    quit: MenuItem,
}

impl TrayUi {
    pub fn set_state(&self, language: Language, running: bool, busy: bool) {
        let strings = language.strings();
        self.show.set_text(strings.tray_show);
        self.toggle.set_text(toggle_label(language, running, busy));
        self.toggle.set_enabled(!busy);
        self.quit.set_text(strings.tray_quit);
    }

    pub fn is_available(&self) -> bool {
        self._icon.is_some()
    }

    #[cfg(test)]
    pub(crate) fn unavailable_for_tests(language: Language) -> Self {
        let strings = language.strings();
        Self {
            _icon: None,
            show: MenuItem::with_id("show", strings.tray_show, true, None),
            toggle: MenuItem::with_id("toggle", strings.start_service, true, None),
            quit: MenuItem::with_id("quit", strings.tray_quit, true, None),
        }
    }
}

pub fn install(ctx: &egui::Context, language: Language) -> (TrayUi, mpsc::Receiver<TrayCommand>) {
    let (tx, rx) = mpsc::channel();
    let repaint_ctx = ctx.clone();
    let menu_tx = tx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = menu_command(event.id().as_ref());
        if let Some(command) = command {
            let _ = menu_tx.send(command);
            repaint_ctx.request_repaint();
        }
    }));

    let repaint_ctx = ctx.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = tx.send(TrayCommand::Show);
            repaint_ctx.request_repaint();
        }
    }));

    let menu = Menu::new();
    let strings = language.strings();
    let show = MenuItem::with_id("show", strings.tray_show, true, None);
    let toggle = MenuItem::with_id("toggle", strings.start_service, true, None);
    let quit = MenuItem::with_id("quit", strings.tray_quit, true, None);

    let icon = match (
        menu.append_items(&[&show, &toggle, &quit]),
        Icon::from_rgba(app_icon_rgba(32), 32, 32),
    ) {
        (Ok(()), Ok(icon)) => TrayIconBuilder::new()
            .with_tooltip("LanPulse")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok(),
        _ => None,
    };

    (
        TrayUi {
            _icon: icon,
            show,
            toggle,
            quit,
        },
        rx,
    )
}

#[cfg(target_os = "linux")]
pub fn init_platform() {
    let _ = gtk::init();
}

#[cfg(not(target_os = "linux"))]
pub fn init_platform() {}

#[cfg(target_os = "linux")]
pub fn pump_platform_events() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn pump_platform_events() {}

fn toggle_label(language: Language, running: bool, busy: bool) -> &'static str {
    let strings = language.strings();
    if busy && !running {
        strings.starting
    } else if busy {
        strings.stopping
    } else if running {
        strings.stop_service
    } else {
        strings.start_service
    }
}

fn menu_command(id: &str) -> Option<TrayCommand> {
    match id {
        "show" => Some(TrayCommand::Show),
        "toggle" => Some(TrayCommand::Toggle),
        "quit" => Some(TrayCommand::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{TrayCommand, TrayUi, menu_command, toggle_label};
    use crate::i18n::Language;

    #[test]
    fn unavailable_tray_still_updates_menu_state() {
        let tray = TrayUi::unavailable_for_tests(Language::En);

        tray.set_state(Language::En, false, false);
        tray.set_state(Language::En, true, false);
        tray.set_state(Language::En, true, true);

        assert!(!tray.is_available());
    }

    #[test]
    fn toggle_label_tracks_running_and_busy_state() {
        assert_eq!(toggle_label(Language::En, false, false), "Start Service");
        assert_eq!(toggle_label(Language::En, true, false), "Stop Service");
        assert_eq!(toggle_label(Language::En, false, true), "Starting");
        assert_eq!(toggle_label(Language::En, true, true), "Stopping");
        assert_eq!(toggle_label(Language::ZhCn, true, false), "停止服务");
    }

    #[test]
    fn menu_command_maps_known_item_ids() {
        assert_eq!(menu_command("show"), Some(TrayCommand::Show));
        assert_eq!(menu_command("toggle"), Some(TrayCommand::Toggle));
        assert_eq!(menu_command("quit"), Some(TrayCommand::Quit));
        assert_eq!(menu_command("unknown"), None);
    }
}
