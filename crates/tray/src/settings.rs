//! The settings popup: driver status, keyboard layout, key remapping and the wheel mode.
//!
//! It's a borderless window that closes when it loses focus, like a tray popup. (On Wayland an
//! app can't place itself next to the tray, so the compositor decides where it appears.)
//! Changes are saved and applied as soon as they're made.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Vec2};
use x360truewireless_common::config::{Config, WheelMode};
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

pub fn run(anchor: Option<Anchor>) -> anyhow::Result<()> {
    let size = [10.0 * KEY.x + 9.0 * GAP + 2.0 * 20.0, 540.0];
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("X360TrueWireless")
        .with_app_id(APP_ID)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_inner_size(size);
    if placement::is_kde_wayland() {
        if let Some(script) = placement::kwin_place(APP_ID, anchor, size) {
            script.remove_later(Duration::from_secs(3));
        }
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
        Box::new(|cc| Ok(Box::new(Settings::new(&cc.egui_ctx)))),
    )
    .map_err(|e| anyhow::anyhow!("opening the settings window: {e}"))
}

struct Settings {
    palette: Palette,
    config: Config,
    load_error: Option<String>,
    layer: Layer,
    selected: Option<ChatpadKey>,
    /// Text being edited for the selected key.
    edit: String,
    edit_error: Option<String>,
    /// Layout text being edited (custom layout).
    layout_edit: String,
    status: Arc<Mutex<Status>>,
    picked: Arc<Mutex<Option<ChatpadKey>>>,
    message: Option<(bool, String)>,
    was_focused: bool,
}

impl Settings {
    fn new(ctx: &egui::Context) -> Self {
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

    fn header_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let status = self.status.lock().unwrap().clone();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("X360TrueWireless").size(18.0).strong());
            ui.add_space(8.0);
            let (dot, text) = match &status.running {
                None => (p.warn, "Driver not running".to_owned()),
                Some(s) => {
                    let players: Vec<String> = s
                        .connected
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| **c)
                        .map(|(i, _)| (i + 1).to_string())
                        .collect();
                    match players.len() {
                        0 => (p.weak_text, "No controller connected".to_owned()),
                        1 => (p.green, format!("Player {} connected", players[0])),
                        _ => (p.green, format!("Players {} connected", players.join(", "))),
                    }
                }
            };
            pill(ui, &p, dot, &text);
            if let Some(s) = &status.running {
                let several = s.connected.iter().filter(|c| **c).count() > 1;
                for (slot, (connected, level)) in s.connected.iter().zip(&s.battery).enumerate() {
                    if let (true, Some(level)) = (*connected, level) {
                        ui.add_space(8.0);
                        if several {
                            ui.label(egui::RichText::new(format!("P{}", slot + 1)).weak());
                        }
                        let tip = format!("Player {}: battery {}", slot + 1, level.name());
                        battery_bars(ui, &p, *level, &tip);
                    }
                }
            }
            if status.running.is_none() && ui.button("Start driver").clicked() {
                crate::tray::restart_driver();
            }
        });
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

    fn footer_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let status = self.status.lock().unwrap().clone();
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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Reset all keys").clicked() && !self.config.keys.is_empty() {
                    self.config.keys.clear();
                    if let Some(key) = self.selected {
                        self.select(key, self.layer);
                    }
                    self.save();
                }
                if let Some((ok, msg)) = &self.message {
                    ui.colored_label(if *ok { p.weak_text } else { p.warn }, msg);
                }
            });
        });
        self.wheel_ui(ui);
        if let Some(e) = &self.load_error {
            ui.colored_label(p.warn, format!("Config problem: {e}"));
        }
    }

    fn wheel_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let current = self.config.wheel_mode.unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Racing wheel");
            for (mode, label, tip) in [
                (
                    WheelMode::Wheel,
                    "Sim wheel",
                    "A wheel with force feedback, for games with wheel support. \
                     Turn Steam Input off for generic controllers so games see it.",
                ),
                (
                    WheelMode::Controller,
                    "Xbox controller",
                    "An Xbox 360 controller, for games without wheel support: steering on the \
                     left stick, with rumble and the wheel's own centring.",
                ),
                (
                    WheelMode::Joystick,
                    "Joystick",
                    "A sim wheel without gamepad buttons, for games that ignore the wheel \
                     because it looks like a gamepad.",
                ),
            ] {
                let selected = current == mode;
                let text =
                    egui::RichText::new(label).color(if selected { p.accent_text } else { p.text });
                let button = egui::Button::new(text)
                    .fill(if selected { p.accent } else { p.button })
                    .corner_radius(CornerRadius::same(14))
                    .min_size(egui::vec2(78.0, 26.0));
                if ui.add(button).on_hover_text(tip).clicked() && !selected {
                    self.config.wheel_mode = (mode != WheelMode::default()).then_some(mode);
                    self.save();
                }
            }
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Switch before starting a game")
                    .color(p.weak_text)
                    .small(),
            );
        });
    }

    /// Parse "layout" or "layout(variant)" from the layout field and save it.
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
        if let Some(key) = picked {
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

        let p = self.palette.clone();
        let frame = egui::Frame::new()
            .fill(p.window)
            .corner_radius(CornerRadius::same(14))
            .stroke(Stroke::new(1.0, mix(p.window, p.text, 0.14)))
            .inner_margin(egui::Margin::same(20));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                frame.show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    // Drag the popup by its header.
                    let header = ui.scope(|ui| self.header_ui(ui)).response;
                    if ui
                        .interact(header.rect, ui.id().with("drag"), Sense::drag())
                        .drag_started()
                    {
                        ui.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    // Added after the drag area so it gets the clicks.
                    let close = egui::Rect::from_min_size(
                        egui::pos2(ui.max_rect().right() - 28.0, header.rect.center().y - 14.0),
                        egui::vec2(28.0, 28.0),
                    );
                    if close_button(ui, close, &p).clicked() {
                        ui.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ui.add_space(6.0);
                    self.layer_tabs(ui);
                    ui.add_space(4.0);
                    self.keyboard_ui(ui);
                    ui.add_space(6.0);
                    self.editor_ui(ui);
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        self.footer_ui(ui);
                        ui.separator();
                    });
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

/// Four battery segments, filled up to the level. Green when healthy, warn colour when low.
fn battery_bars(ui: &mut egui::Ui, p: &Palette, level: Battery, tip: &str) {
    const SEGMENT: Vec2 = Vec2::new(5.0, 12.0);
    const GAP: f32 = 2.0;
    let size = Vec2::new(4.0 * SEGMENT.x + 3.0 * GAP, SEGMENT.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.on_hover_text(tip);
    let fill = if level.is_low() { p.warn } else { p.green };
    let empty = mix(p.button, p.weak_text, 0.5);
    for i in 0..4u8 {
        let x = rect.left() + i as f32 * (SEGMENT.x + GAP);
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
