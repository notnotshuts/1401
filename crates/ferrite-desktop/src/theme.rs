use crate::*;
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub warning: Color32,
    pub error: Color32,
}
pub fn palette(ui: &egui::Ui) -> Palette {
    colors(ui.visuals().dark_mode)
}
pub fn colors(dark: bool) -> Palette {
    let rgb = Color32::from_rgb;
    if dark {
        Palette {
            bg: rgb(17, 18, 22),
            panel: rgb(28, 29, 35),
            card: rgb(37, 39, 46),
            border: rgb(58, 60, 69),
            text: rgb(245, 245, 247),
            muted: rgb(164, 166, 177),
            accent: rgb(10, 132, 255),
            warning: rgb(255, 179, 64),
            error: rgb(255, 105, 97),
        }
    } else {
        Palette {
            bg: rgb(245, 245, 247),
            panel: Color32::WHITE,
            card: rgb(239, 240, 244),
            border: rgb(221, 223, 230),
            text: rgb(29, 29, 31),
            muted: rgb(101, 105, 117),
            accent: rgb(0, 104, 222),
            warning: rgb(152, 85, 0),
            error: rgb(194, 43, 42),
        }
    }
}
pub fn apply(ctx: &egui::Context, dark: bool) {
    let c = colors(dark);
    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    let mut style = (*ctx.style()).clone();
    style.visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.panel_fill = c.bg;
    style.visuals.window_fill = c.panel;
    style.visuals.faint_bg_color = c.card;
    style.visuals.override_text_color = Some(c.text);
    style.visuals.selection.bg_fill = c.accent.gamma_multiply(0.16);
    style.visuals.selection.stroke = egui::Stroke::new(1_f32, c.accent);
    style.visuals.warn_fg_color = c.warning;
    style.visuals.error_fg_color = c.error;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(12);
        widget.bg_stroke = egui::Stroke::NONE;
        widget.bg_fill = c.card;
        widget.fg_stroke = egui::Stroke::new(1_f32, c.text);
    }
    style.visuals.widgets.hovered.bg_fill = c.border;
    style.visuals.widgets.active.bg_fill = c.accent.gamma_multiply(0.2);
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1_f32, c.border);
    style.visuals.window_corner_radius = egui::CornerRadius::same(20);
    style.visuals.window_stroke = egui::Stroke::new(1_f32, c.border);
    style.spacing.item_spacing = Vec2::new(12., 12.);
    style.spacing.button_padding = Vec2::new(16., 12.);
    style.spacing.interact_size = Vec2::new(44., 44.);
    style.spacing.combo_height = 240.;
    for (kind, size) in [
        (egui::TextStyle::Body, 16.),
        (egui::TextStyle::Button, 16.),
        (egui::TextStyle::Small, 13.),
        (egui::TextStyle::Heading, 30.),
        (egui::TextStyle::Monospace, 13.),
    ] {
        style.text_styles.insert(
            kind.clone(),
            if kind == egui::TextStyle::Monospace {
                egui::FontId::monospace(size)
            } else {
                egui::FontId::proportional(size)
            },
        );
    }
    style.animation_time = 0.18;
    ctx.set_style(style);
}
pub fn fonts(ctx: &egui::Context) {
    // Use an installed system font; Apple's fonts are never redistributed.
    #[cfg(target_os = "macos")]
    let paths = ["/System/Library/Fonts/SFNS.ttf"];
    #[cfg(target_os = "windows")]
    let paths = ["C:/Windows/Fonts/segoeui.ttf"];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let paths = [
        "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    for path in paths {
        if let Ok(data) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("system".into(), egui::FontData::from_owned(data).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "system".into());
            ctx.set_fonts(fonts);
            break;
        }
    }
}
pub fn card(ui: &egui::Ui) -> egui::Frame {
    let c = palette(ui);
    egui::Frame::new()
        .fill(c.panel)
        .corner_radius(20)
        .inner_margin(20)
        .stroke(egui::Stroke::new(1_f32, c.border.gamma_multiply(0.55)))
}
pub fn primary(text: &str, _ui: &egui::Ui) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.to_owned())
            .color(Color32::WHITE)
            .strong(),
    )
    .fill(Color32::from_rgb(0, 104, 222))
    .corner_radius(22)
    .min_size(Vec2::new(180., 46.))
}

pub fn glass(ui: &egui::Ui) -> egui::Frame {
    let c = palette(ui);
    card(ui)
        .fill(c.panel.gamma_multiply(0.92))
        .corner_radius(24)
        .shadow(egui::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(if ui.visuals().dark_mode { 40 } else { 14 }),
        })
}
