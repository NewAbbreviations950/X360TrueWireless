//! Make the settings popup look at home: colours from the desktop (KDE colour scheme, or the
//! freedesktop portal's light/dark preference and accent colour on GNOME and others) and the
//! system UI font.

use std::process::Command;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke};

#[derive(Debug, Clone)]
pub struct Palette {
    pub dark: bool,
    pub window: Color32,
    /// Background of input fields and the key area.
    pub view: Color32,
    pub button: Color32,
    pub text: Color32,
    pub weak_text: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub green: Color32,
    pub orange: Color32,
    pub warn: Color32,
}

impl Palette {
    pub fn detect() -> Self {
        let portal = portal_appearance();
        let dark = portal.as_ref().and_then(|p| p.dark).unwrap_or(true);
        let mut palette = Self::fallback(dark);
        if let Some(accent) = portal.and_then(|p| p.accent) {
            palette.accent = accent;
        }
        if let Some(kde) = kde_colors() {
            kde.apply(&mut palette);
        }
        palette.accent_text = if luminance(palette.accent) > 0.55 {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        palette
    }

    fn fallback(dark: bool) -> Self {
        if dark {
            Palette {
                dark,
                window: Color32::from_rgb(32, 35, 38),
                view: Color32::from_rgb(25, 27, 30),
                button: Color32::from_rgb(49, 54, 59),
                text: Color32::from_rgb(232, 234, 237),
                weak_text: Color32::from_rgb(150, 155, 160),
                accent: Color32::from_rgb(61, 174, 233),
                accent_text: Color32::WHITE,
                green: Color32::from_rgb(110, 200, 95),
                orange: Color32::from_rgb(245, 160, 60),
                warn: Color32::from_rgb(240, 170, 60),
            }
        } else {
            Palette {
                dark,
                window: Color32::from_rgb(239, 240, 241),
                view: Color32::from_rgb(252, 252, 252),
                button: Color32::from_rgb(226, 228, 231),
                text: Color32::from_rgb(35, 38, 41),
                weak_text: Color32::from_rgb(110, 115, 120),
                accent: Color32::from_rgb(61, 174, 233),
                accent_text: Color32::WHITE,
                green: Color32::from_rgb(45, 140, 40),
                orange: Color32::from_rgb(200, 110, 10),
                warn: Color32::from_rgb(190, 110, 0),
            }
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.panel_fill = self.window;
        v.window_fill = self.window;
        v.extreme_bg_color = self.view;
        v.faint_bg_color = self.view;
        v.override_text_color = Some(self.text);
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent;
        v.selection.stroke = Stroke::new(1.0, self.accent_text);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(8);
        v.window_stroke = Stroke::new(1.0, mix(self.window, self.text, 0.12));
        let radius = CornerRadius::same(6);
        let w = &mut v.widgets;
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.bg_stroke = Stroke::new(1.0, mix(self.window, self.text, 0.10));
        for (state, lift) in [
            (&mut w.inactive, 0.0),
            (&mut w.hovered, 0.08),
            (&mut w.active, 0.14),
            (&mut w.open, 0.08),
        ] {
            let fill = mix(self.button, self.text, lift);
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.corner_radius = radius;
            state.fg_stroke = Stroke::new(1.0, self.text);
            state.bg_stroke = Stroke::NONE;
        }
        w.hovered.bg_stroke = Stroke::new(1.0, self.accent.gamma_multiply(0.6));
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        // Pin egui's theme to ours so it doesn't switch on its own when the system flips.
        ctx.set_theme(if self.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        ctx.set_visuals(v);
        ctx.all_styles_mut(|s| {
            s.spacing.item_spacing = egui::vec2(8.0, 8.0);
            s.spacing.button_padding = egui::vec2(10.0, 5.0);
            s.spacing.interact_size.y = 28.0;
        });
    }
}

/// Use the desktop's UI font, keeping egui's bundled fonts as fallback for missing glyphs.
pub fn load_system_font(ctx: &egui::Context) {
    let path = Command::new("fc-match")
        .args(["-f", "%{file}", "sans-serif"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .filter(|p| !p.is_empty());
    let Some(bytes) = path.and_then(|p| std::fs::read(p).ok()) else {
        return;
    };
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("system".into(), FontData::from_owned(bytes).into());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "system".into());
    ctx.set_fonts(fonts);
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn luminance(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

struct Appearance {
    dark: Option<bool>,
    accent: Option<Color32>,
}

/// org.freedesktop.appearance from the settings portal (KDE, GNOME, and others).
fn portal_appearance() -> Option<Appearance> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )
    .ok()?;
    let read = |key: &str| -> Option<zbus::zvariant::OwnedValue> {
        proxy
            .call("ReadOne", &("org.freedesktop.appearance", key))
            .ok()
    };
    let dark = read("color-scheme")
        .and_then(|v| u32::try_from(v).ok())
        .and_then(|v| match v {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        });
    let accent = read("accent-color")
        .and_then(|v| <(f64, f64, f64)>::try_from(v).ok())
        .filter(|(r, g, b)| [r, g, b].iter().all(|c| (0.0..=1.0).contains(*c)))
        .map(|(r, g, b)| {
            Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
        });
    Some(Appearance { dark, accent })
}

/// Colours from KDE's active colour scheme (~/.config/kdeglobals), if any.
struct KdeColors {
    entries: Vec<(String, String, Color32)>,
}

impl KdeColors {
    fn get(&self, section: &str, key: &str) -> Option<Color32> {
        self.entries
            .iter()
            .find(|(s, k, _)| s == section && k == key)
            .map(|(_, _, c)| *c)
    }

    fn apply(&self, p: &mut Palette) {
        if let Some(c) = self.get("Colors:Window", "BackgroundNormal") {
            p.window = c;
            p.dark = luminance(c) < 0.5;
        }
        if let Some(c) = self.get("Colors:View", "BackgroundNormal") {
            p.view = c;
        }
        if let Some(c) = self.get("Colors:Button", "BackgroundNormal") {
            p.button = c;
        }
        if let Some(c) = self.get("Colors:Window", "ForegroundNormal") {
            p.text = c;
        }
        if let Some(c) = self.get("Colors:Window", "ForegroundInactive") {
            p.weak_text = c;
        }
        if let Some(c) = self
            .get("General", "AccentColor")
            .or_else(|| self.get("Colors:Selection", "BackgroundNormal"))
        {
            p.accent = c;
        }
        if let Some(c) = self.get("Colors:Window", "ForegroundPositive") {
            p.green = c;
        }
        if let Some(c) = self.get("Colors:Window", "ForegroundNeutral") {
            p.orange = c;
            p.warn = c;
        }
    }
}

fn kde_colors() -> Option<KdeColors> {
    let path = x360truewireless_common::config::config_dir()?.join("kdeglobals");
    let text = std::fs::read_to_string(path).ok()?;
    let mut section = String::new();
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.to_owned();
        } else if let Some((key, value)) = line.split_once('=') {
            let rgb: Vec<u8> = value
                .split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect();
            if let [r, g, b, ..] = rgb[..] {
                entries.push((section.clone(), key.to_owned(), Color32::from_rgb(r, g, b)));
            }
        }
    }
    (!entries.is_empty()).then_some(KdeColors { entries })
}
