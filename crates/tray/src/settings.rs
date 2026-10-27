//! The settings popup: a box per connected controller slot (with the wheel mode and a way into
//! the chatpad key remapper), and a settings page for the keyboard layout and the driver.
//!
//! It's a borderless window that closes when it loses focus, like a tray popup. (On Wayland an
//! app can't place itself next to the tray, so the compositor decides where it appears.) It
//! resizes to fit what it shows. Changes are saved and applied as soon as they're made.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Vec2};
use x360truewireless_common::config::{Config, WheelMode};
use x360truewireless_common::dbus::SlotDevice;
use x360truewireless_common::keymap::{KeyTable, Layer, Output};
use x360truewireless_common::{Battery, ChatpadKey};

use crate::client::{self, Client, Status};
use crate::placement::{self, Anchor};
use crate::theme::{Palette, mix};

/// The chatpad's physical layout, row by row.
const ROWS: &[&[ChatpadKey]] = {
    use ChatpadKey::*;
    &[
        &[D1, D2, D3, D4, D5, D6, D7, D8, D9, D0],
        &[Q, W, E, R, T, Y, U, I, O, P],
        &[A, S, D, F, G, H, J, K, L, Comma],
        &[Z, X, C, V, B, N, M, Period, Enter],
        &[Left, Space, Right, Backspace],
    ]
};

const KEY: Vec2 = Vec2::new(54.0, 50.0);
const GAP: f32 = 6.0;
/// One controller slot's box on the main page.
const BOX: Vec2 = Vec2::new(172.0, 200.0);
const BOX_GAP: f32 = 12.0;
const MARGIN: f32 = 20.0;
const HEADER: f32 = 28.0;

/// The wheel modes offered, with their labels and explanations.
#[cfg(target_os = "linux")]
const WHEEL_MODES: &[(WheelMode, &str, &str)] = &[
    (
        WheelMode::Logitech,
        "Logitech G920",
        "A force-feedback wheel that games see as a Logitech G920, so games with a G920 \
         profile set up its buttons and pedals themselves. Turn Steam Input off for generic \
         controllers so games see the wheel itself.",
    ),
    (
        WheelMode::Thrustmaster,
        "Thrustmaster TMX",
        "The same wheel shown as a Thrustmaster TMX, for games with a TMX profile but none \
         for the G920.",
    ),
    (
        WheelMode::Controller,
        "Xbox controller",
        "An Xbox 360 controller, for games without wheel support: steering on the left \
         stick, with rumble and the wheel's own centring.",
    ),
];

/// On Windows the wheel is a vJoy joystick, which has one identity whatever the mode.
#[cfg(windows)]
const WHEEL_MODES: &[(WheelMode, &str, &str)] = &[
    (
        WheelMode::Logitech,
        "Racing wheel",
        "A force-feedback wheel (a vJoy device). Bind its steering, pedals and buttons in \
         the game's controls.",
    ),
    (
        WheelMode::Controller,
        "Xbox controller",
        "An Xbox 360 controller, for games without wheel support: steering on the left \
         stick, with rumble and the wheel's own centring.",
    ),
];

/// Non-character keys offered in the picker.
const SPECIAL_KEYS: &[(&str, &str)] = &[
    ("Enter", "KEY_ENTER"),
    ("Backspace", "KEY_BACKSPACE"),
    ("Delete", "KEY_DELETE"),
    ("Tab", "KEY_TAB"),
    ("Escape", "KEY_ESC"),
    ("Space", "KEY_SPACE"),
    ("Left", "KEY_LEFT"),
    ("Right", "KEY_RIGHT"),
    ("Up", "KEY_UP"),
    ("Down", "KEY_DOWN"),
    ("Home", "KEY_HOME"),
    ("End", "KEY_END"),
    ("Page Up", "KEY_PAGEUP"),
    ("Page Down", "KEY_PAGEDOWN"),
    ("F1", "KEY_F1"),
    ("F2", "KEY_F2"),
    ("F3", "KEY_F3"),
    ("F4", "KEY_F4"),
    ("F5", "KEY_F5"),
    ("F6", "KEY_F6"),
    ("F7", "KEY_F7"),
    ("F8", "KEY_F8"),
    ("F9", "KEY_F9"),
    ("F10", "KEY_F10"),
    ("F11", "KEY_F11"),
    ("F12", "KEY_F12"),
    ("Super / Meta", "KEY_LEFTMETA"),
    ("Play / Pause", "KEY_PLAYPAUSE"),
    ("Next track", "KEY_NEXTSONG"),
    ("Previous track", "KEY_PREVIOUSSONG"),
    ("Volume up", "KEY_VOLUMEUP"),
    ("Volume down", "KEY_VOLUMEDOWN"),
    ("Mute", "KEY_MUTE"),
];

const APP_ID: &str = "x360truewireless";

/// Top-down pictures of a controller, without and with its chatpad, traced from photos taken
/// straight down on one (the outline averaged with its mirror image; buttons and keys measured
/// on the photos). Drawn in two colours, swapped for the theme's at runtime.
const CONTROLLER_SVG: &str = include_str!("icons/controller.svg");
const CHATPAD_SVG: &str = include_str!("icons/chatpad.svg");
/// The racing wheel front on, measured the same way on a photo of one.
const WHEEL_SVG: &str = include_str!("icons/wheel.svg");
const SVG_BODY: &str = "#b0b0b0";
const SVG_DETAIL: &str = "#303030";
/// How wide the pictures are in the popup; their height follows from their shape. The wheel
/// is round, so it's narrower to stay as tall as the controllers.
const ICON_WIDTH: f32 = 78.0;
const WHEEL_WIDTH: f32 = 66.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// The connected slots.
    Slots,
    /// The chatpad key remapper.
    Keys,
    /// App-wide settings, behind the gear button.
    Settings,
}

/// The window size for `page`, with `slots` slots connected.
fn page_size(page: Page, slots: usize) -> Vec2 {
    match page {
        Page::Slots => {
            let n = slots.max(1) as f32;
            let boxes = n * BOX.x + (n - 1.0) * BOX_GAP;
            egui::vec2(
                boxes.max(360.0) + 2.0 * MARGIN,
                2.0 * MARGIN + HEADER + 30.0 + BOX.y,
            )
        }
        Page::Keys => egui::vec2(10.0 * KEY.x + 9.0 * GAP + 2.0 * MARGIN, 490.0),
        Page::Settings => egui::vec2(480.0, 250.0),
    }
}

fn occupied(status: &Status) -> Vec<(usize, SlotDevice, Option<Battery>)> {
    let Some(s) = &status.running else {
        return Vec::new();
    };
    (s.devices.iter().enumerate())
        .filter(|(_, d)| **d != SlotDevice::Empty)
        .map(|(slot, &d)| (slot, d, s.battery.get(slot).copied().flatten()))
        .collect()
}

pub fn run(anchor: Option<Anchor>) -> anyhow::Result<()> {
    let status = Client::new().status();
    let size = page_size(Page::Slots, occupied(&status).len());
    let size = [size.x, size.y];
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("X360TrueWireless")
        .with_app_id(APP_ID)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_inner_size(size);
    // KWin keeps the popup next to the tray as it resizes, for as long as the script is loaded.
    let mut _placement = None;
    if placement::is_kde_wayland() {
        _placement = placement::kwin_place(APP_ID, anchor, size);
    } else if let Some(pos) = placement::x11_position(anchor, size) {
        viewport = viewport.with_position(pos);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "X360TrueWireless",
        options,
        Box::new(move |cc| Ok(Box::new(Settings::new(&cc.egui_ctx, status, anchor)))),
    )
    .map_err(|e| anyhow::anyhow!("opening the settings window: {e}"))
}

struct Settings {
    palette: Palette,
    page: Page,
    /// The window size being asked for, how many times, and when last.
    resize: (Vec2, u8, Instant),
    anchor: Option<Anchor>,
    /// Rendered device pictures, by (device, dimmed).
    icons: HashMap<(SlotDevice, bool), egui::TextureHandle>,
    config: Config,
    load_error: Option<String>,
    layer: Layer,
    selected: Option<ChatpadKey>,
    /// Text being edited for the selected key.
    edit: String,
    edit_error: Option<String>,
    /// Layout text being edited (custom layout).
    #[cfg_attr(windows, allow(dead_code))]
    layout_edit: String,
    status: Arc<Mutex<Status>>,
    picked: Arc<Mutex<Option<ChatpadKey>>>,
    message: Option<(bool, String)>,
    was_focused: bool,
}

impl Settings {
    fn new(ctx: &egui::Context, first: Status, anchor: Option<Anchor>) -> Self {
        let palette = Palette::detect();
        crate::theme::load_system_font(ctx);
        palette.apply(ctx);

        let (config, load_error) = match Config::load() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(e)),
        };
        let repaint = ctx.clone();
        let status =
            client::watch_status(Duration::from_secs(1), move |_| repaint.request_repaint());
        *status.lock().unwrap() = first;
        let picked = Arc::new(Mutex::new(None));
        {
            let picked = picked.clone();
            let repaint = ctx.clone();
            client::watch_keys(move |_slot, key| {
                *picked.lock().unwrap() = Some(key);
                repaint.request_repaint();
            });
        }
        let layout_edit = match (&config.layout, config.variant.as_deref()) {
            (Some(l), Some(v)) if !v.is_empty() => format!("{l}({v})"),
            (Some(l), _) => l.clone(),
            (None, _) => String::new(),
        };
        Settings {
            palette,
            page: Page::Slots,
            resize: (Vec2::ZERO, 0, Instant::now()),
            anchor,
            icons: HashMap::new(),
            config,
            load_error,
            layer: Layer::Normal,
            selected: None,
            edit: String::new(),
            edit_error: None,
            layout_edit,
            status,
            picked,
            message: None,
            was_focused: false,
        }
    }

    fn override_for(&self, layer: Layer, key: ChatpadKey) -> Option<&String> {
        self.config.keys.get(&layer)?.get(key.name())
    }

    /// What `key` does on `layer` itself (no fallback to the normal layer).
    fn own_output(&self, layer: Layer, key: ChatpadKey) -> Option<Output> {
        match self.override_for(layer, key) {
            Some(text) => text.parse().ok(),
            None => KeyTable::default_output(layer, key),
        }
    }

    fn select(&mut self, key: ChatpadKey, layer: Layer) {
        self.selected = Some(key);
        self.layer = layer;
        self.edit = self
            .own_output(layer, key)
            .map(|o| o.to_string())
            .unwrap_or_default();
        self.edit_error = None;
    }

    /// Set the selected key's output on the current layer (None = back to default), then save.
    fn set_output(&mut self, key: ChatpadKey, value: Option<String>) {
        let layer = self.layer;
        let default = KeyTable::default_output(layer, key).map(|o| o.to_string());
        let keys = self.config.keys.entry(layer).or_default();
        match value {
            Some(v) if Some(&v) != default.as_ref() => {
                keys.insert(key.name().to_owned(), v);
            }
            _ => {
                keys.remove(key.name());
            }
        }
        if keys.is_empty() {
            self.config.keys.remove(&layer);
        }
        self.select(key, layer);
        self.save();
    }

    fn save(&mut self) {
        self.message = Some(match self.config.save() {
            Err(e) => (false, format!("Couldn't save: {e}")),
            Ok(()) => match Client::new().reload() {
                Ok(()) => (true, "Saved".into()),
                Err(e) => (false, e),
            },
        });
    }

    /// The title row. Returns where the back button goes: it's added after the header's drag
    /// area, so that it gets the clicks.
    fn header_ui(&mut self, ui: &mut egui::Ui) -> Option<Rect> {
        let p = self.palette.clone();
        let running = self.status.lock().unwrap().running.is_some();
        let mut back = None;
        ui.horizontal(|ui| {
            ui.set_min_height(HEADER);
            if self.page != Page::Slots {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::hover());
                back = Some(rect);
            }
            let title = match self.page {
                Page::Slots => "X360TrueWireless",
                Page::Keys => "Chatpad keys",
                Page::Settings => "Settings",
            };
            ui.label(egui::RichText::new(title).size(18.0).strong());
            if !running {
                ui.add_space(8.0);
                pill(ui, &p, p.warn, "Driver not running");
            }
            if let Some((ok, msg)) = &self.message {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(msg)
                        .color(if *ok { p.weak_text } else { p.warn })
                        .small(),
                );
            }
        });
        back
    }

    /// The picture of a device (a wheel, a controller with a chatpad, or else a plain
    /// controller), dimmed for one still connecting.
    fn icon(&mut self, ctx: &egui::Context, device: SlotDevice, dim: bool) -> egui::TextureHandle {
        let p = &self.palette;
        let scale = ctx.pixels_per_point();
        self.icons
            .entry((device, dim))
            .or_insert_with(|| {
                let body = mix(p.button, p.text, if dim { 0.25 } else { 0.6 });
                let detail = mix(p.window, p.button, 0.5);
                let (svg, width) = match device {
                    SlotDevice::Wheel => (WHEEL_SVG, WHEEL_WIDTH),
                    SlotDevice::Chatpad => (CHATPAD_SVG, ICON_WIDTH),
                    _ => (CONTROLLER_SVG, ICON_WIDTH),
                };
                let image = render_svg(svg, body, detail, width * scale);
                ctx.load_texture(
                    format!("device-{device:?}-{dim}"),
                    image,
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone()
    }

    fn slots_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let status = self.status.lock().unwrap().clone();
        if status.running.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(36.0);
                ui.label("The driver isn't running, so controllers aren't set up.");
                ui.add_space(8.0);
                if ui.button("Start driver").clicked() {
                    crate::tray::restart_driver();
                }
            });
            return;
        }
        let slots = occupied(&status);
        if slots.is_empty() {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), BOX.y), Sense::hover());
            let painter = ui.painter();
            painter.rect(
                rect,
                CornerRadius::same(12),
                Color32::TRANSPARENT,
                Stroke::new(1.0, mix(p.window, p.text, 0.2)),
                egui::StrokeKind::Inside,
            );
            let icon = self.icon(ui.ctx(), SlotDevice::Controller, true);
            paint_icon(ui.painter(), &icon, rect.center() - egui::vec2(0.0, 30.0));
            let painter = ui.painter();
            painter.text(
                rect.center() + egui::vec2(0.0, 20.0),
                Align2::CENTER_CENTER,
                "No controllers connected",
                FontId::proportional(14.0),
                p.text,
            );
            painter.text(
                rect.center() + egui::vec2(0.0, 42.0),
                Align2::CENTER_CENTER,
                "Press the Guide button on a controller or wheel",
                FontId::proportional(12.0),
                p.weak_text,
            );
            return;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = BOX_GAP;
            let n = slots.len() as f32;
            let width = n * BOX.x + (n - 1.0) * BOX_GAP;
            let spare = (ui.available_width() - width) / 2.0 - BOX_GAP;
            if spare > 0.0 {
                ui.add_space(spare);
            }
            for (slot, device, battery) in slots {
                self.slot_box(ui, slot, device, battery);
            }
        });
    }

    /// One slot: what's connected, its battery, and what can be set for it.
    fn slot_box(
        &mut self,
        ui: &mut egui::Ui,
        slot: usize,
        device: SlotDevice,
        battery: Option<Battery>,
    ) {
        let p = self.palette.clone();
        let (rect, _) = ui.allocate_exact_size(BOX, Sense::hover());
        // Widgets go in a child, so placing them doesn't move the row's cursor.
        let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let inner = rect.shrink(12.0);
        let painter = ui.painter().clone();
        painter.rect(
            rect,
            CornerRadius::same(12),
            p.button.gamma_multiply(0.55),
            Stroke::new(1.0, mix(p.window, p.text, 0.12)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            inner.left_top(),
            Align2::LEFT_TOP,
            format!("Slot {}", slot + 1),
            FontId::proportional(14.0),
            p.text,
        );
        let connecting = device == SlotDevice::Connecting;
        let centre = egui::pos2(inner.center().x, inner.top() + 56.0);
        let icon = self.icon(ui.ctx(), device, connecting);
        paint_icon(&painter, &icon, centre);
        let name = match device {
            SlotDevice::Connecting => "Connecting…",
            SlotDevice::Chatpad => "Controller + chatpad",
            SlotDevice::Wheel => "Racing wheel",
            _ => "Controller",
        };
        painter.text(
            egui::pos2(inner.center().x, inner.top() + 102.0),
            Align2::CENTER_CENTER,
            name,
            FontId::proportional(13.0),
            if connecting { p.weak_text } else { p.text },
        );

        let action = Rect::from_min_size(
            egui::pos2(inner.left(), inner.top() + 118.0),
            egui::vec2(inner.width(), 28.0),
        );
        match device {
            SlotDevice::Chatpad => {
                let button = egui::Button::new("Remap keys").corner_radius(CornerRadius::same(14));
                if ui
                    .put(action, button)
                    .on_hover_text("Change what the chatpad's keys type (on every chatpad)")
                    .clicked()
                {
                    self.page = Page::Keys;
                }
            }
            SlotDevice::Wheel => {
                let current = self.config.wheel_mode.unwrap_or_default();
                // Both wheel identities are the same vJoy wheel on Windows.
                #[cfg(windows)]
                let current = match current {
                    WheelMode::Thrustmaster => WheelMode::Logitech,
                    mode => mode,
                };
                let label = (WHEEL_MODES.iter())
                    .find(|(m, ..)| *m == current)
                    .map_or("", |(_, label, _)| label);
                let mut chosen = None;
                ui.scope_builder(egui::UiBuilder::new().max_rect(action), |ui| {
                    egui::ComboBox::from_id_salt(("wheel", slot))
                        .width(action.width())
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for &(mode, label, tip) in WHEEL_MODES {
                                if ui
                                    .selectable_label(mode == current, label)
                                    .on_hover_text(tip)
                                    .clicked()
                                    && mode != current
                                {
                                    chosen = Some(mode);
                                }
                            }
                        })
                        .response
                        .on_hover_text("How games see the wheel. Switch before starting a game.");
                });
                if let Some(mode) = chosen {
                    self.config.wheel_mode = (mode != WheelMode::default()).then_some(mode);
                    self.save();
                }
            }
            _ => {}
        }

        if let Some(level) = battery {
            let size = battery_size();
            let bars = Rect::from_min_size(inner.right_bottom() - size, size);
            ui.scope_builder(egui::UiBuilder::new().max_rect(bars), |ui| {
                battery_bars(ui, &p, level, &format!("Battery {}", level.name()));
            });
        }
    }

    fn layer_tabs(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        ui.horizontal(|ui| {
            for (layer, label, color) in [
                (Layer::Normal, "Normal", p.text),
                (Layer::Green, "Green", p.green),
                (Layer::Orange, "Orange", p.orange),
            ] {
                let selected = self.layer == layer;
                let text =
                    egui::RichText::new(label).color(if selected { p.accent_text } else { color });
                let button = egui::Button::new(text)
                    .fill(if selected { p.accent } else { p.button })
                    .corner_radius(CornerRadius::same(14))
                    .min_size(egui::vec2(78.0, 26.0));
                if ui.add(button).clicked() {
                    match self.selected {
                        Some(key) => self.select(key, layer),
                        None => self.layer = layer,
                    }
                }
            }
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Click a key or press it on the chatpad")
                    .color(p.weak_text)
                    .small(),
            );
        });
    }

    fn keyboard_ui(&mut self, ui: &mut egui::Ui) {
        for row in ROWS {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                // Centre rows narrower than the full ten keys.
                let row_width: f32 =
                    row.iter().map(|&k| key_width(k)).sum::<f32>() + GAP * (row.len() - 1) as f32;
                let full_width = KEY.x * 10.0 + GAP * 9.0;
                if row_width < full_width {
                    ui.add_space((full_width - row_width) / 2.0 - GAP);
                }
                for &key in *row {
                    let width = key_width(key);
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(width, KEY.y), Sense::click());
                    self.paint_keycap(ui, rect, key, response.hovered());
                    if response.clicked() {
                        // Clicking a corner legend picks that layer.
                        let pos = response.interact_pointer_pos().unwrap_or(rect.center());
                        let top = pos.y < rect.top() + rect.height() * 0.45;
                        let layer = match (top, pos.x < rect.center().x) {
                            (true, true) if self.own_output(Layer::Green, key).is_some() => {
                                Layer::Green
                            }
                            (true, false) if self.own_output(Layer::Orange, key).is_some() => {
                                Layer::Orange
                            }
                            (true, _) => self.layer,
                            (false, _) => Layer::Normal,
                        };
                        self.select(key, layer);
                    }
                }
            });
            ui.add_space(GAP - ui.spacing().item_spacing.y);
        }
    }

    fn paint_keycap(&self, ui: &egui::Ui, rect: Rect, key: ChatpadKey, hovered: bool) {
        let p = &self.palette;
        let painter = ui.painter();
        let selected = self.selected == Some(key);
        let fill = if hovered {
            mix(p.button, p.text, 0.08)
        } else {
            p.button
        };
        let stroke = if selected {
            Stroke::new(2.0, p.accent)
        } else {
            Stroke::NONE
        };
        painter.rect(
            rect,
            CornerRadius::same(8),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );

        // Emphasise the layer being edited; dim the others.
        let tint = |c: Color32, layer: Layer| {
            if self.layer == layer {
                c
            } else {
                c.gamma_multiply(0.45)
            }
        };
        let customized = |layer: Layer| self.override_for(layer, key).is_some();

        if let Some(out) = self.own_output(Layer::Normal, key) {
            let text = short_label(out, false);
            let size = if text.chars().count() > 2 { 13.0 } else { 19.0 };
            let color = if customized(Layer::Normal) {
                p.accent
            } else {
                p.text
            };
            painter.text(
                rect.center() + egui::vec2(0.0, 7.0),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(size),
                tint(color, Layer::Normal),
            );
        }
        for (layer, color, align, pos) in [
            (
                Layer::Green,
                p.green,
                Align2::LEFT_TOP,
                rect.left_top() + egui::vec2(6.0, 4.0),
            ),
            (
                Layer::Orange,
                p.orange,
                Align2::RIGHT_TOP,
                rect.right_top() + egui::vec2(-6.0, 4.0),
            ),
        ] {
            if let Some(out) = self.own_output(layer, key) {
                let color = if customized(layer) { p.accent } else { color };
                painter.text(
                    pos,
                    align,
                    short_label(out, true),
                    FontId::proportional(12.0),
                    tint(color, layer),
                );
            }
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let Some(key) = self.selected else {
            ui.label(
                egui::RichText::new("Select a key to change what it types.").color(p.weak_text),
            );
            return;
        };
        ui.horizontal(|ui| {
            let layer_color = match self.layer {
                Layer::Normal => p.text,
                Layer::Green => p.green,
                Layer::Orange => p.orange,
            };
            ui.label(egui::RichText::new(key_name(key)).strong());
            ui.label(
                egui::RichText::new(format!("{} layer", layer_title(self.layer)))
                    .color(layer_color),
            );
            ui.label("types");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.edit)
                    .desired_width(110.0)
                    .hint_text("a character"),
            );
            let apply = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if apply || ui.button("Set").clicked() {
                match self.edit.parse::<Output>() {
                    Ok(Output::Nothing) => {
                        self.edit_error = Some("Type a character, or use Disable.".into())
                    }
                    Ok(_) => self.set_output(key, Some(self.edit.clone())),
                    Err(e) => self.edit_error = Some(e),
                }
            }
            egui::ComboBox::from_id_salt("special")
                .selected_text("Special key")
                .height(260.0)
                .show_ui(ui, |ui| {
                    for &(label, name) in SPECIAL_KEYS {
                        if ui.selectable_label(false, label).clicked() {
                            self.set_output(key, Some(name.to_owned()));
                        }
                    }
                });
            if ui.button("Disable").clicked() {
                self.set_output(key, Some(String::new()));
            }
            let custom = self.override_for(self.layer, key).is_some();
            if ui.add_enabled(custom, egui::Button::new("Reset")).clicked() {
                self.set_output(key, None);
            }
        });
        if let Some(e) = &self.edit_error {
            ui.colored_label(p.warn, e);
        }
    }

    fn keys_footer_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("The same keys apply to every chatpad.")
                    .color(self.palette.weak_text)
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Reset all keys").clicked() && !self.config.keys.is_empty() {
                    self.config.keys.clear();
                    if let Some(key) = self.selected {
                        self.select(key, self.layer);
                    }
                    self.save();
                }
            });
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let status = self.status.lock().unwrap().clone();
        #[cfg(target_os = "linux")]
        {
            ui.horizontal(|ui| {
                ui.label("Keyboard layout");
                let auto = self.config.layout.is_none();
                let detected = status
                    .running
                    .as_ref()
                    .map(|s| s.layout.clone())
                    .unwrap_or_default();
                let auto_label = if detected.is_empty() || !auto {
                    "Automatic".to_owned()
                } else {
                    format!("Automatic ({detected})")
                };
                egui::ComboBox::from_id_salt("layout")
                    .selected_text(if auto {
                        auto_label.clone()
                    } else {
                        "Custom".into()
                    })
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(auto, &auto_label).clicked() && !auto {
                            self.config.layout = None;
                            self.config.variant = None;
                            self.layout_edit.clear();
                            self.save();
                        }
                        if ui.selectable_label(!auto, "Custom").clicked() && auto {
                            self.layout_edit = if detected.is_empty() {
                                "us".into()
                            } else {
                                detected.clone()
                            };
                            self.apply_layout_edit();
                        }
                    });
                if !auto {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.layout_edit)
                            .desired_width(110.0)
                            .hint_text("us, no, de(nodeadkeys)"),
                    );
                    if field.lost_focus() {
                        self.apply_layout_edit();
                    }
                }
            });
            ui.label(
                egui::RichText::new(
                    "Chatpads type through this layout. Steam Deck game mode uses a US layout \
                     whatever the desktop's is, so set yours here if it differs.",
                )
                .color(p.weak_text)
                .small(),
            );
        }
        #[cfg(windows)]
        {
            let detected = status.running.as_ref().map(|s| s.layout.clone());
            ui.horizontal(|ui| {
                ui.label("Keyboard layout");
                ui.label(detected.as_deref().unwrap_or("unknown"));
            });
            ui.label(
                egui::RichText::new(
                    "Chatpads type through the keyboard layout of the window you type into.",
                )
                .color(p.weak_text)
                .small(),
            );
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label("Driver");
            let (dot, text) = match status.running {
                Some(_) => (p.green, "Running"),
                None => (p.warn, "Not running"),
            };
            pill(ui, &p, dot, text);
            let label = if status.running.is_some() {
                "Restart driver"
            } else {
                "Start driver"
            };
            if ui.button(label).clicked() {
                crate::tray::restart_driver();
            }
        });
        ui.add_space(14.0);
        ui.label(
            egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .color(p.weak_text)
                .small(),
        );
    }

    /// Parse "layout" or "layout(variant)" from the layout field and save it.
    #[cfg_attr(windows, allow(dead_code))]
    fn apply_layout_edit(&mut self) {
        let text = self.layout_edit.trim();
        let (layout, variant) = match text.split_once('(') {
            Some((l, v)) => (l.trim(), v.trim_end_matches(')').trim()),
            None => (text, ""),
        };
        if layout.is_empty() {
            return;
        }
        let new = (
            Some(layout.to_owned()),
            (!variant.is_empty()).then(|| variant.to_owned()),
        );
        if (self.config.layout.clone(), self.config.variant.clone()) != new {
            (self.config.layout, self.config.variant) = new;
            self.save();
        }
    }
}

impl eframe::App for Settings {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Behave like a popup: close on Escape or when focus moves elsewhere.
        let focused = ui.input(|i| i.viewport().focused);
        if focused == Some(true) {
            self.was_focused = true;
        } else if focused == Some(false) && self.was_focused {
            ui.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            ui.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // A key pressed on the chatpad selects it. Drop text focus so the character the
        // chatpad also types doesn't land in a text field.
        let picked = self.picked.lock().unwrap().take();
        if let Some(key) = picked.filter(|_| self.page == Page::Keys) {
            ui.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            let layer = if self.own_output(self.layer, key).is_some() {
                self.layer
            } else {
                Layer::Normal
            };
            self.select(key, layer);
        }

        // Fit the window to the page. A request made before the window is on screen gets
        // lost, so keep asking (a few times) until the size matches.
        let slots = occupied(&self.status.lock().unwrap()).len();
        let size = page_size(self.page, slots);
        let actual = ui.ctx().content_rect().size();
        if (actual - size).abs().max_elem() > 1.0 {
            let (wanted, tries, at) = &mut self.resize;
            if *wanted != size {
                (*wanted, *tries) = (size, 0);
            }
            if *tries < 8 && at.elapsed() >= Duration::from_millis(200) {
                *tries += 1;
                *at = Instant::now();
                // A fixed-size window: some compositors go by its size limits.
                ui.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
                ui.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
                ui.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                if !placement::is_wayland()
                    && let Some(pos) = placement::x11_position(self.anchor, [size.x, size.y])
                {
                    ui.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.into()));
                }
            }
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }

        let p = self.palette.clone();
        let frame = egui::Frame::new()
            .fill(p.window)
            .corner_radius(CornerRadius::same(14))
            .stroke(Stroke::new(1.0, mix(p.window, p.text, 0.14)))
            .inner_margin(egui::Margin::same(MARGIN as i8));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                frame.show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    // Drag the popup by its header.
                    let scope = ui.scope(|ui| self.header_ui(ui));
                    let (header, back) = (scope.response, scope.inner);
                    if ui
                        .interact(header.rect, ui.id().with("drag"), Sense::drag())
                        .drag_started()
                    {
                        ui.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    // Added after the drag area so they get the clicks.
                    if let Some(rect) = back
                        && back_button(ui, rect, &p).clicked()
                    {
                        self.page = Page::Slots;
                    }
                    let right = ui.max_rect().right();
                    let y = header.rect.center().y - 14.0;
                    let close = Rect::from_min_size(egui::pos2(right - 28.0, y), Vec2::splat(28.0));
                    if close_button(ui, close, &p).clicked() {
                        ui.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    let gear = Rect::from_min_size(egui::pos2(right - 62.0, y), Vec2::splat(28.0));
                    if gear_button(ui, gear, &p, self.page == Page::Settings).clicked() {
                        self.page = if self.page == Page::Settings {
                            Page::Slots
                        } else {
                            Page::Settings
                        };
                    }
                    ui.add_space(14.0);
                    match self.page {
                        Page::Slots => self.slots_ui(ui),
                        Page::Keys => {
                            self.layer_tabs(ui);
                            ui.add_space(4.0);
                            self.keyboard_ui(ui);
                            ui.add_space(6.0);
                            self.editor_ui(ui);
                            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                                self.keys_footer_ui(ui);
                                ui.separator();
                            });
                        }
                        Page::Settings => self.settings_ui(ui),
                    }
                    if let Some(e) = &self.load_error {
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                            ui.colored_label(p.warn, format!("Config problem: {e}"));
                        });
                    }
                });
            });
    }
}

fn pill(ui: &mut egui::Ui, p: &Palette, dot: Color32, text: &str) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(13.0), p.text);
    let size = egui::vec2(galley.size().x + 28.0, 24.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), p.button);
    painter.circle_filled(rect.left_center() + egui::vec2(12.0, 0.0), 4.0, dot);
    painter.galley(
        rect.left_top() + egui::vec2(22.0, (24.0 - galley.size().y) / 2.0),
        galley,
        p.text,
    );
}

const SEGMENT: Vec2 = Vec2::new(5.0, 12.0);
const SEGMENT_GAP: f32 = 2.0;

fn battery_size() -> Vec2 {
    Vec2::new(4.0 * SEGMENT.x + 3.0 * SEGMENT_GAP, SEGMENT.y)
}

/// Four battery segments, filled up to the level. Green when healthy, warn colour when low.
fn battery_bars(ui: &mut egui::Ui, p: &Palette, level: Battery, tip: &str) {
    let (rect, response) = ui.allocate_exact_size(battery_size(), Sense::hover());
    response.on_hover_text(tip);
    let fill = if level.is_low() { p.warn } else { p.green };
    let empty = mix(p.button, p.weak_text, 0.5);
    for i in 0..4u8 {
        let x = rect.left() + i as f32 * (SEGMENT.x + SEGMENT_GAP);
        let segment = Rect::from_min_size(egui::pos2(x, rect.top()), SEGMENT);
        let color = if i < level.bars() { fill } else { empty };
        ui.painter()
            .rect_filled(segment, CornerRadius::same(1), color);
    }
}

fn close_button(ui: &mut egui::Ui, rect: Rect, p: &Palette) -> egui::Response {
    let response = ui.interact(rect, ui.id().with("close"), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.circle_filled(rect.center(), 14.0, p.button);
    }
    let r = 5.0;
    let c = rect.center();
    let stroke = Stroke::new(1.6, p.weak_text);
    painter.line_segment([c + egui::vec2(-r, -r), c + egui::vec2(r, r)], stroke);
    painter.line_segment([c + egui::vec2(r, -r), c + egui::vec2(-r, r)], stroke);
    response.on_hover_text("Close (Esc)")
}

fn back_button(ui: &mut egui::Ui, rect: Rect, p: &Palette) -> egui::Response {
    let response = ui.interact(rect, ui.id().with("back"), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.circle_filled(rect.center(), 14.0, p.button);
    }
    // Centred on the hover circle, like the close button's cross.
    let c = rect.center();
    let stroke = Stroke::new(1.8, p.weak_text);
    painter.line_segment(
        [c + egui::vec2(3.0, -6.0), c + egui::vec2(-3.0, 0.0)],
        stroke,
    );
    painter.line_segment(
        [c + egui::vec2(-3.0, 0.0), c + egui::vec2(3.0, 6.0)],
        stroke,
    );
    response.on_hover_text("Back")
}

fn gear_button(ui: &mut egui::Ui, rect: Rect, p: &Palette, on: bool) -> egui::Response {
    let response = ui.interact(rect, ui.id().with("gear"), Sense::click());
    let painter = ui.painter();
    if on || response.hovered() {
        painter.circle_filled(rect.center(), 14.0, p.button);
    }
    let c = rect.center();
    let color = if on { p.text } else { p.weak_text };
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::TAU / 8.0;
        let dir = egui::vec2(angle.cos(), angle.sin());
        painter.line_segment([c + dir * 5.0, c + dir * 8.5], Stroke::new(2.6, color));
    }
    painter.circle_stroke(c, 5.0, Stroke::new(2.2, color));
    response.on_hover_text("Settings")
}

/// Draw a device picture centred on `c`, at the size it was rendered for.
fn paint_icon(painter: &egui::Painter, icon: &egui::TextureHandle, c: egui::Pos2) {
    let size = icon.size_vec2() / painter.ctx().pixels_per_point();
    painter.image(
        icon.id(),
        Rect::from_center_size(c, size),
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// Render one of our two-colour SVGs `width` pixels wide, in `body` and `detail`.
fn render_svg(svg: &str, body: Color32, detail: Color32, width: f32) -> egui::ColorImage {
    use resvg::{tiny_skia, usvg};
    let hex = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
    let svg = svg
        .replace(SVG_BODY, &hex(body))
        .replace(SVG_DETAIL, &hex(detail));
    let Ok(tree) = usvg::Tree::from_str(&svg, &usvg::Options::default()) else {
        return egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]);
    };
    let scale = width / tree.size().width();
    let [w, h] = [
        width.round() as u32,
        (tree.size().height() * scale).round() as u32,
    ];
    let blank = || {
        egui::ColorImage::new(
            [w as usize, h as usize],
            vec![Color32::TRANSPARENT; (w * h) as usize],
        )
    };
    let Some(mut pixmap) = tiny_skia::Pixmap::new(w, h) else {
        return blank();
    };
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia's pixels are premultiplied, like egui's.
    let pixels = (pixmap.pixels().iter())
        .map(|px| Color32::from_rgba_premultiplied(px.red(), px.green(), px.blue(), px.alpha()))
        .collect();
    egui::ColorImage::new([w as usize, h as usize], pixels)
}

fn key_width(key: ChatpadKey) -> f32 {
    match key {
        ChatpadKey::Space => KEY.x * 3.0 + GAP * 2.0,
        ChatpadKey::Enter | ChatpadKey::Backspace => KEY.x * 2.0 + GAP,
        _ => KEY.x,
    }
}

fn layer_title(layer: Layer) -> &'static str {
    match layer {
        Layer::Normal => "Normal",
        Layer::Green => "Green",
        Layer::Orange => "Orange",
    }
}

fn key_name(key: ChatpadKey) -> &'static str {
    use ChatpadKey::*;
    match key {
        D1 => "1",
        D2 => "2",
        D3 => "3",
        D4 => "4",
        D5 => "5",
        D6 => "6",
        D7 => "7",
        D8 => "8",
        D9 => "9",
        D0 => "0",
        Comma => "Comma",
        Period => "Period",
        other => other.name(),
    }
}

/// Text shown on a keycap. Letters are shown in capitals on the normal layer, as printed.
fn short_label(out: Output, alternate: bool) -> String {
    match out {
        Output::Char(c) if !alternate && c.is_alphabetic() => c.to_uppercase().collect(),
        Output::Char(' ') => "Space".into(),
        Output::Char(c) => c.to_string(),
        Output::Key(k) => {
            let name = format!("{k:?}");
            let name = name.strip_prefix("KEY_").unwrap_or(&name);
            match name {
                "ENTER" => "Enter".into(),
                "BACKSPACE" => "Bksp".into(),
                "SPACE" => "Space".into(),
                "LEFT" => "Left".into(),
                "RIGHT" => "Right".into(),
                "LEFTMETA" => "Meta".into(),
                other => {
                    let mut s = other.to_lowercase();
                    if let Some(first) = s.get_mut(0..1) {
                        first.make_ascii_uppercase();
                    }
                    s
                }
            }
        }
        Output::Nothing => "off".into(),
    }
}
