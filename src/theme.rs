#![allow(dead_code)]
use egui::{Color32, CornerRadius, Stroke, Visuals};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub chrome: Color32,
    pub surface: Color32,
    pub side: Color32,
    pub code: Color32,
    pub border: Color32,
    pub soft: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub live: Color32,
    pub add: Color32,
    pub del: Color32,
    pub add_bg: Color32,
    pub del_bg: Color32,
    pub live_bg: Color32,
    pub sel_bg: Color32,
    pub hover_bg: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Clair : crème. Sombre : bleu nuit.
pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            bg: hex(0x0F1B2D),
            chrome: hex(0x0B1525),
            surface: hex(0x13223A),
            side: hex(0x101D31),
            code: hex(0x0D182A),
            border: hex(0x243756),
            soft: hex(0x1C2D48),
            text: hex(0xE7ECF4),
            muted: hex(0x9FB0C8),
            accent: hex(0x6FA8FF),
            on_accent: hex(0x0B1525),
            live: hex(0xF2A65A),
            add: hex(0x7FD49C),
            del: hex(0xF0A7A7),
            add_bg: hex(0x123024),
            del_bg: hex(0x341C24),
            live_bg: hex(0x2A2420),
            sel_bg: hex(0x23406B),
            hover_bg: hex(0x182A44),
        }
    } else {
        Palette {
            dark,
            bg: hex(0xEFE6D3),
            chrome: hex(0xE4D8BF),
            surface: hex(0xF5EDDC),
            side: hex(0xEEE4CF),
            code: hex(0xF7F0E1),
            border: hex(0xD6C8AA),
            soft: hex(0xE8DCC3),
            text: hex(0x2A2620),
            muted: hex(0x625846),
            accent: hex(0x2E5E9E),
            on_accent: hex(0xFFFFFF),
            live: hex(0xA9561A),
            add: hex(0x2F7346),
            del: hex(0xA83434),
            add_bg: hex(0xE0EBD6),
            del_bg: hex(0xF2DCD4),
            live_bg: hex(0xF1DFC4),
            sel_bg: hex(0xD5DDE6),
            hover_bg: hex(0xEBE1CB),
        }
    }
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.surface;
    v.window_fill = p.surface;
    v.faint_bg_color = p.side;
    v.extreme_bg_color = p.code;
    v.code_bg_color = p.code;
    v.hyperlink_color = p.accent;
    v.override_text_color = Some(p.text);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    let r = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.bg_fill = p.soft;
    w.inactive.weak_bg_fill = p.soft;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.bg_fill = p.hover_bg;
    w.hovered.weak_bg_fill = p.hover_bg;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.active.bg_fill = p.sel_bg;
    w.active.weak_bg_fill = p.sel_bg;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.open.bg_fill = p.soft;
    w.open.weak_bg_fill = p.soft;
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = r;
    }
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_corner_radius = CornerRadius::same(10);
    let th = if p.dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(th);
    ctx.set_visuals_of(th, v);
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let load = |p: &str| std::fs::read(p).ok();
    if let Some(bytes) = load("C:\\Windows\\Fonts\\segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    if let Some(bytes) = load("C:\\Windows\\Fonts\\consola.ttf") {
        fonts.font_data.insert("consolas".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "consolas".into());
    }
    // Gras (titres Word, étiquettes)
    let mut bold = Vec::new();
    if let Some(bytes) = load("C:\\Windows\\Fonts\\segoeuib.ttf") {
        fonts.font_data.insert("segoe-bold".into(), egui::FontData::from_owned(bytes).into());
        bold.push("segoe-bold".to_string());
    }
    bold.extend(fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(egui::FontFamily::Name("bold".into()), bold);
    // Emoji / pictogrammes (⏸ 📷 🔊…)
    if let Some(bytes) = load("C:\\Windows\\Fonts\\seguiemj.ttf") {
        fonts.font_data.insert("emoji".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(egui::FontFamily::Proportional).or_default().push("emoji".into());
    }
    // Symboles (flèches, etc.) en secours
    if let Some(bytes) = load("C:\\Windows\\Fonts\\seguisym.ttf") {
        fonts.font_data.insert("segsym".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(egui::FontFamily::Proportional).or_default().push("segsym".into());
    }
    ctx.set_fonts(fonts);
}
