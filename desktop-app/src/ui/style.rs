use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Vec2,
};

use super::{ACCENT, ACCENT_DARK, BACKGROUND, BORDER, SURFACE, SURFACE_RAISED, TEXT};

pub fn configure_fonts(ctx: &egui::Context) {
    #[cfg(target_os = "linux")]
    let cjk_font = Some(FontData::from_static(include_bytes!(
        "../../assets/fonts/NotoSansSC-LanPulse.otf"
    )));
    #[cfg(target_os = "windows")]
    let cjk_font =
        load_system_cjk_font(&[r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\msyhbd.ttc"]);
    #[cfg(target_os = "macos")]
    let cjk_font = load_system_cjk_font(&[
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
    ]);
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    let cjk_font: Option<FontData> = None;

    let Some(cjk_font) = cjk_font else {
        return;
    };
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("system_cjk".to_string(), cjk_font.into());
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("system_cjk".to_string());
    }
    ctx.set_fonts(fonts);
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn load_system_cjk_font(candidates: &[&str]) -> Option<FontData> {
    candidates
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .map(FontData::from_owned)
}

pub fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 7.0);
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(13.0, FontFamily::Proportional),
    );
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = BACKGROUND;
    visuals.override_text_color = Some(TEXT);
    visuals.faint_bg_color = SURFACE;
    visuals.extreme_bg_color = Color32::from_rgb(11, 14, 16);
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_fill = SURFACE_RAISED;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(40, 48, 53);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(75, 88, 94));
    visuals.widgets.hovered.expansion = 0.0;
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, ACCENT_DARK);
    visuals.widgets.active.expansion = 0.0;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_DARK);
    ctx.set_visuals(visuals);
}

pub fn app_icon_rgba(size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let center = size as f32 / 2.0;
    let radius = size as f32 * 0.42;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= radius {
                let edge = ((radius - d) / 3.0).clamp(0.0, 1.0);
                rgba.extend_from_slice(&[
                    (42.0 * edge + 24.0 * (1.0 - edge)) as u8,
                    (184.0 * edge + 108.0 * (1.0 - edge)) as u8,
                    (132.0 * edge + 94.0 * (1.0 - edge)) as u8,
                    255,
                ]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    rgba
}
