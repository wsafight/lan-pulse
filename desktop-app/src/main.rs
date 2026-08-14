use eframe::egui::{self, Vec2};

mod app;
mod i18n;
mod service;
mod settings;
mod tray;
mod ui;

fn main() -> eframe::Result {
    tray::init_platform();

    eframe::run_native(
        "LanPulse",
        native_options(),
        Box::new(|cc| Ok(Box::new(app::LanPulseApp::new(cc)))),
    )
}

fn native_options() -> eframe::NativeOptions {
    let icon_rgba = ui::app_icon_rgba(64);
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LanPulse")
            .with_inner_size(Vec2::new(760.0, 620.0))
            .with_min_inner_size(Vec2::new(540.0, 520.0))
            .with_icon(egui::IconData {
                rgba: icon_rgba,
                width: 64,
                height: 64,
            }),
        centered: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::Vec2;

    use super::native_options;

    #[test]
    fn native_options_define_window_size_title_and_icon() {
        let options = native_options();
        let icon = options.viewport.icon.as_ref().unwrap();

        assert!(options.centered);
        assert_eq!(options.viewport.title.as_deref(), Some("LanPulse"));
        assert_eq!(options.viewport.inner_size, Some(Vec2::new(760.0, 620.0)));
        assert_eq!(
            options.viewport.min_inner_size,
            Some(Vec2::new(540.0, 520.0))
        );
        assert_eq!(icon.width, 64);
        assert_eq!(icon.height, 64);
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
    }
}
