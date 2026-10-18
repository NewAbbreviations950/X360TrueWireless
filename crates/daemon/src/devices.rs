//! The uinput devices we expose: one Xbox 360 pad per connected controller, and one keyboard
//! shared by all chatpads.

use std::collections::HashMap;
use std::io;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventSummary, EventType, FFEffectCode,
    FFEffectKind, InputEvent, InputId, KeyCode, UInputCode, UinputAbsSetup,
};
use x360truewireless_proto::{ChatpadKey, ChatpadKeys, GamepadState, button, led, modifier};

use crate::ffb;
use crate::keymap::{Keymap, Layer};

/// Identify as a wired Xbox 360 pad (what xpad reports for 045e:028e), which SDL and Steam
/// recognise out of the box.
const PAD_ID: (u16, u16, u16) = (0x045e, 0x028e, 0x0114);
const PAD_NAME: &str = "Microsoft X-Box 360 pad";
const FF_EFFECTS_MAX: u16 = 16;

const BUTTONS: &[(u16, KeyCode)] = &[
    (button::A, KeyCode::BTN_SOUTH),
    (button::B, KeyCode::BTN_EAST),
    (button::X, KeyCode::BTN_NORTH),
    (button::Y, KeyCode::BTN_WEST),
    (button::LB, KeyCode::BTN_TL),
    (button::RB, KeyCode::BTN_TR),
    (button::BACK, KeyCode::BTN_SELECT),
    (button::START, KeyCode::BTN_START),
    (button::GUIDE, KeyCode::BTN_MODE),
    (button::LS, KeyCode::BTN_THUMBL),
    (button::RS, KeyCode::BTN_THUMBR),
];

pub struct Pad {
    dev: VirtualDevice,
    /// Uploaded rumble effects: id → (strong, weak).
    effects: HashMap<i16, (u16, u16)>,
    /// When the currently playing effect ends on its own.
    stop_at: Option<Instant>,
    rumble: (u8, u8),
    /// Last state from the controller, and buttons we hold down on top of it (chatpad shortcuts).
    last: GamepadState,
    overlay: u16,
    /// Overlay changes scheduled for later (for button sequences like Guide-then-A).
    overlay_steps: std::collections::VecDeque<(Instant, u16)>,
}

impl Pad {
    pub fn new() -> io::Result<Self> {
        let keys: AttributeSet<KeyCode> = BUTTONS.iter().map(|&(_, k)| k).collect();
        let stick = AbsInfo::new(0, -32768, 32767, 16, 128, 0);
        let trigger = AbsInfo::new(0, 0, 255, 0, 0, 0);
        let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
        let mut builder = VirtualDevice::builder()?
            .name(PAD_NAME)
            .input_id(InputId::new(BusType::BUS_USB, PAD_ID.0, PAD_ID.1, PAD_ID.2))
            .with_keys(&keys)?
            .with_ff(&AttributeSet::from_iter([FFEffectCode::FF_RUMBLE]))?
            .with_ff_effects_max(FF_EFFECTS_MAX as u32);
        for (axis, info) in [
            (AbsoluteAxisCode::ABS_X, stick),
            (AbsoluteAxisCode::ABS_Y, stick),
            (AbsoluteAxisCode::ABS_RX, stick),
            (AbsoluteAxisCode::ABS_RY, stick),
            (AbsoluteAxisCode::ABS_Z, trigger),
            (AbsoluteAxisCode::ABS_RZ, trigger),
            (AbsoluteAxisCode::ABS_HAT0X, hat),
            (AbsoluteAxisCode::ABS_HAT0Y, hat),
        ] {
            builder = builder.with_absolute_axis(&UinputAbsSetup::new(axis, info))?;
        }
        let dev = builder.build()?;
        set_nonblocking(&dev)?;
        Ok(Pad {
            dev,
            effects: HashMap::new(),
            stop_at: None,
            rumble: (0, 0),
            last: GamepadState::default(),
            overlay: 0,
            overlay_steps: Default::default(),
        })
    }

    pub fn emit(&mut self, state: &GamepadState) -> io::Result<()> {
        self.last = *state;
        let s = &GamepadState {
            buttons: state.buttons | self.overlay,
            ..*state
        };
        let mut events: Vec<InputEvent> = BUTTONS
            .iter()
            .map(|&(bit, key)| {
                InputEvent::new(EventType::KEY.0, key.0, (s.buttons & bit != 0) as i32)
            })
            .collect();
        let dir =
            |neg: u16, pos: u16| (s.buttons & pos != 0) as i32 - (s.buttons & neg != 0) as i32;
        // Like xpad: evdev Y axes point down, so flip the controller's up-positive values.
        for (axis, value) in [
            (AbsoluteAxisCode::ABS_X, s.lx as i32),
            (AbsoluteAxisCode::ABS_Y, !s.ly as i32),
            (AbsoluteAxisCode::ABS_RX, s.rx as i32),
            (AbsoluteAxisCode::ABS_RY, !s.ry as i32),
            (AbsoluteAxisCode::ABS_Z, s.lt as i32),
            (AbsoluteAxisCode::ABS_RZ, s.rt as i32),
            (
                AbsoluteAxisCode::ABS_HAT0X,
                dir(button::DPAD_LEFT, button::DPAD_RIGHT),
            ),
            (
                AbsoluteAxisCode::ABS_HAT0Y,
                dir(button::DPAD_UP, button::DPAD_DOWN),
            ),
        ] {
            events.push(InputEvent::new(EventType::ABSOLUTE.0, axis.0, value));
        }
        self.dev.emit(&events)
    }

    /// Press or release the Steam Quick Access shortcut. Like a person would, Guide goes
    /// down first and A follows, so Steam sees a Guide chord rather than a stray A press.
    pub fn quick_menu(&mut self, on: bool, now: Instant) -> io::Result<()> {
        let (first, second) = if on {
            (button::GUIDE, button::GUIDE | button::A)
        } else {
            (button::GUIDE, 0)
        };
        self.overlay_steps.clear();
        self.overlay_steps.push_back((now, first));
        self.overlay_steps.push_back((now + CHORD_STEP, second));
        self.tick(now)
    }

    /// Apply scheduled overlay changes that are due.
    pub fn tick(&mut self, now: Instant) -> io::Result<()> {
        let mut changed = false;
        while let Some(&(at, buttons)) = self.overlay_steps.front() {
            if at > now {
                break;
            }
            self.overlay_steps.pop_front();
            changed |= self.overlay != buttons;
            self.overlay = buttons;
        }
        if changed {
            let last = self.last;
            self.emit(&last)?;
        }
        Ok(())
    }

    /// Handle pending force-feedback requests from games. Returns the new rumble strength
    /// `(strong, weak)` if it changed and should be sent to the controller.
    pub fn poll_ff(&mut self) -> io::Result<Option<(u8, u8)>> {
        let mut target = None;
        let events: Vec<InputEvent> = match self.dev.fetch_events() {
            Ok(events) => events.collect(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Vec::new(),
            Err(e) => return Err(e),
        };
        for event in events {
            match event.destructure() {
                EventSummary::UInput(event, UInputCode::UI_FF_UPLOAD, ..) => {
                    let mut upload = self.dev.process_ff_upload(event)?;
                    let id = upload.effect_id();
                    let id = if (0..FF_EFFECTS_MAX as i16).contains(&id) {
                        id
                    } else {
                        // New effect: pick the lowest free id.
                        match (0..FF_EFFECTS_MAX as i16).find(|i| !self.effects.contains_key(i)) {
                            Some(free) => free,
                            None => {
                                upload.set_retval(-libc::ENOSPC);
                                continue;
                            }
                        }
                    };
                    let magnitudes = match upload.effect().kind {
                        FFEffectKind::Rumble {
                            strong_magnitude,
                            weak_magnitude,
                        } => (strong_magnitude, weak_magnitude),
                        _ => (0, 0),
                    };
                    self.effects.insert(id, magnitudes);
                    upload.set_effect_id(id);
                    upload.set_retval(0);
                }
                EventSummary::UInput(event, UInputCode::UI_FF_ERASE, ..) => {
                    let mut erase = self.dev.process_ff_erase(event)?;
                    self.effects.remove(&(erase.effect_id() as i16));
                    erase.set_retval(0);
                }
                EventSummary::ForceFeedback(_, code, value) => {
                    let id = code.0 as i16;
                    if value > 0 {
                        let (strong, weak) = self.effects.get(&id).copied().unwrap_or((0, 0));
                        target = Some(((strong >> 8) as u8, (weak >> 8) as u8));
                        // Games mostly stop effects explicitly; cap runaway rumble just in case.
                        self.stop_at = Some(Instant::now() + Duration::from_secs(10));
                    } else {
                        target = Some((0, 0));
                        self.stop_at = None;
                    }
                }
                _ => {}
            }
        }
        if target.is_none() && self.stop_at.is_some_and(|t| Instant::now() >= t) {
            target = Some((0, 0));
            self.stop_at = None;
        }
        Ok(match target {
            Some(r) if r != self.rumble => {
                self.rumble = r;
                Some(r)
            }
            _ => None,
        })
    }
}

/// Which wheel the virtual wheel presents itself as. SDL decides a joystick is a wheel only
/// from a list of USB IDs, and Proton presents only those to games as DirectInput driving
/// devices with full force feedback; games such as SnowRunner list nothing else as a wheel.
/// So both layouts take an ID from that list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelLayout {
    /// A Logitech G920, the Xbox-layout Logitech wheel, so games load their own G920 profile
    /// (button numbers and axes from DiRT 4's `input/actionmaps/lg_g920.xml`).
    G920,
    /// The OpenFFBoard's ID (a generic open-source force-feedback wheel, in pid.codes' open
    /// VID), for games without a G920 profile; everything has to be bound in the game.
    /// Gamepad buttons let SDL map it as a gamepad too, which Proton ignores for wheels.
    Generic,
}

const GENERIC_ID: (u16, u16, u16) = (0x1209, 0xffb0, 0x0001);
const GENERIC_NAME: &str = "Microsoft Xbox 360 Wireless Racing Wheel";
const G920_ID: (u16, u16, u16) = (0x046d, 0xc262, 0x0111);
const G920_NAME: &str = "Logitech G920 Driving Force Racing Wheel";

/// The G920's DirectInput buttons in order (SDL, and so Proton, numbers buttons by key code):
/// A, B, X, Y, right paddle, left paddle, Menu, View, right stick, left stick, Xbox. The 360
/// wheel's paddles report as RB and LB.
const G920_BUTTONS: &[(u16, KeyCode)] = &[
    (button::A, KeyCode::BTN_TRIGGER),
    (button::B, KeyCode::BTN_THUMB),
    (button::X, KeyCode::BTN_THUMB2),
    (button::Y, KeyCode::BTN_TOP),
    (button::RB, KeyCode::BTN_TOP2),
    (button::LB, KeyCode::BTN_PINKIE),
    (button::START, KeyCode::BTN_BASE),
    (button::BACK, KeyCode::BTN_BASE2),
    (button::RS, KeyCode::BTN_BASE3),
    (button::LS, KeyCode::BTN_BASE4),
    (button::GUIDE, KeyCode::BTN_BASE5),
];

/// Every effect type a wheel could play. Periodic and ramp effects aren't played yet
/// (`wheel_ff`), but they're still advertised so games that use them keep their other effects.
const WHEEL_FF: &[FFEffectCode] = &[
    FFEffectCode::FF_CONSTANT,
    FFEffectCode::FF_RAMP,
    FFEffectCode::FF_PERIODIC,
    FFEffectCode::FF_SQUARE,
    FFEffectCode::FF_TRIANGLE,
    FFEffectCode::FF_SINE,
    FFEffectCode::FF_SAW_UP,
    FFEffectCode::FF_SAW_DOWN,
    FFEffectCode::FF_SPRING,
    FFEffectCode::FF_FRICTION,
    FFEffectCode::FF_DAMPER,
    FFEffectCode::FF_INERTIA,
    FFEffectCode::FF_RUMBLE,
    FFEffectCode::FF_GAIN,
    FFEffectCode::FF_AUTOCENTER,
];

fn buttons(layout: WheelLayout) -> &'static [(u16, KeyCode)] {
    match layout {
        WheelLayout::G920 => G920_BUTTONS,
        WheelLayout::Generic => BUTTONS,
    }
}

/// The axes in key-code order, which is the order SDL and Proton number them in. For the
/// G920 Proton takes exactly four: steering, throttle, brake and clutch.
fn axes(layout: WheelLayout) -> Vec<(AbsoluteAxisCode, AbsInfo)> {
    let stick = AbsInfo::new(0, -32768, 32767, 0, 0, 0);
    let pedal = AbsInfo::new(0, 0, 255, 0, 0, 0);
    let released = AbsInfo::new(255, 0, 255, 0, 0, 0);
    let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
    let mut axes = match layout {
        WheelLayout::G920 => vec![
            (AbsoluteAxisCode::ABS_X, stick),
            (AbsoluteAxisCode::ABS_Y, released),
            (AbsoluteAxisCode::ABS_Z, released),
            (AbsoluteAxisCode::ABS_RZ, released),
        ],
        WheelLayout::Generic => vec![
            (AbsoluteAxisCode::ABS_X, stick),
            (AbsoluteAxisCode::ABS_Y, stick),
            (AbsoluteAxisCode::ABS_RX, stick),
            (AbsoluteAxisCode::ABS_RY, stick),
            (AbsoluteAxisCode::ABS_Z, pedal),
            (AbsoluteAxisCode::ABS_RZ, pedal),
        ],
    };
    axes.extend([
        (AbsoluteAxisCode::ABS_HAT0X, hat),
        (AbsoluteAxisCode::ABS_HAT0Y, hat),
    ]);
    axes
}

pub struct Wheel {
    dev: VirtualDevice,
    layout: WheelLayout,
    effects: HashMap<i16, ffb::Effect>,
}

impl Wheel {
    pub fn new(layout: WheelLayout) -> io::Result<Self> {
        let (id, name) = match layout {
            WheelLayout::G920 => (G920_ID, G920_NAME),
            WheelLayout::Generic => (GENERIC_ID, GENERIC_NAME),
        };
        let keys: AttributeSet<KeyCode> = buttons(layout).iter().map(|&(_, k)| k).collect();
        let mut builder = VirtualDevice::builder()?
            .name(name)
            .input_id(InputId::new(BusType::BUS_USB, id.0, id.1, id.2))
            .with_keys(&keys)?
            .with_ff(&WHEEL_FF.iter().copied().collect::<AttributeSet<_>>())?
            .with_ff_effects_max(FF_EFFECTS_MAX as u32);
        for (axis, info) in axes(layout) {
            builder = builder.with_absolute_axis(&UinputAbsSetup::new(axis, info))?;
        }
        let dev = builder.build()?;
        set_nonblocking(&dev)?;
        Ok(Wheel {
            dev,
            layout,
            effects: HashMap::new(),
        })
    }

    pub fn layout(&self) -> WheelLayout {
        self.layout
    }

    /// XInput reports a wheel like a pad: steering on LX, throttle on RT, brake on LT.
    pub fn emit(&mut self, s: &GamepadState) -> io::Result<()> {
        let mut events: Vec<InputEvent> = buttons(self.layout)
            .iter()
            .map(|&(bit, key)| {
                InputEvent::new(EventType::KEY.0, key.0, (s.buttons & bit != 0) as i32)
            })
            .collect();
        let dir =
            |neg: u16, pos: u16| (s.buttons & pos != 0) as i32 - (s.buttons & neg != 0) as i32;
        let mut axes = vec![(AbsoluteAxisCode::ABS_X, s.lx as i32)];
        match self.layout {
            // Released pedals read full, as on a G920: throttle, brake and clutch (none, so
            // always released). Proton's DirectInput renames a G920's Z to Z rotation
            // ("Brake") and its Z rotation to a slider ("Clutch"), as on Windows.
            WheelLayout::G920 => axes.extend([
                (AbsoluteAxisCode::ABS_Y, 255 - s.rt as i32),
                (AbsoluteAxisCode::ABS_Z, 255 - s.lt as i32),
                (AbsoluteAxisCode::ABS_RZ, 255),
            ]),
            // As on the pad. The other stick values go to Y, RX and RY until a capture shows
            // whether the wheel uses them.
            WheelLayout::Generic => axes.extend([
                (AbsoluteAxisCode::ABS_Y, !s.ly as i32),
                (AbsoluteAxisCode::ABS_RX, s.rx as i32),
                (AbsoluteAxisCode::ABS_RY, !s.ry as i32),
                (AbsoluteAxisCode::ABS_Z, s.lt as i32),
                (AbsoluteAxisCode::ABS_RZ, s.rt as i32),
            ]),
        }
        axes.extend([
            (
                AbsoluteAxisCode::ABS_HAT0X,
                dir(button::DPAD_LEFT, button::DPAD_RIGHT),
            ),
            (
                AbsoluteAxisCode::ABS_HAT0Y,
                dir(button::DPAD_UP, button::DPAD_DOWN),
            ),
        ]);
        for (axis, value) in axes {
            events.push(InputEvent::new(EventType::ABSOLUTE.0, axis.0, value));
        }
        self.dev.emit(&events)
    }

    /// Answer pending force-feedback requests from games and return them, in order.
    pub fn poll_ff(&mut self) -> io::Result<Vec<ffb::Op>> {
        let events: Vec<InputEvent> = match self.dev.fetch_events() {
            Ok(events) => events.collect(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Vec::new(),
            Err(e) => return Err(e),
        };
        let mut ops = Vec::new();
        for event in events {
            match event.destructure() {
                EventSummary::UInput(_, UInputCode::UI_FF_UPLOAD, request) => {
                    let upload = ffb::Upload::begin(self.dev.as_raw_fd(), request)?;
                    // The input core has already picked a free id.
                    let id = upload.id();
                    match ffb::from_raw(upload.raw()) {
                        Some(effect) if (0..FF_EFFECTS_MAX as i16).contains(&id) => {
                            upload.finish(Ok(id))?;
                            self.effects.insert(id, effect);
                            ops.push(ffb::Op::Upload { id, effect });
                        }
                        _ => upload.finish(Err(libc::EINVAL))?,
                    }
                }
                EventSummary::UInput(event, UInputCode::UI_FF_ERASE, ..) => {
                    let mut erase = self.dev.process_ff_erase(event)?;
                    let id = erase.effect_id() as i16;
                    self.effects.remove(&id);
                    erase.set_retval(0);
                    ops.push(ffb::Op::Erase { id });
                }
                EventSummary::ForceFeedback(_, FFEffectCode::FF_GAIN, value) => {
                    ops.push(ffb::Op::Gain(value as f32 / 65535.0))
                }
                EventSummary::ForceFeedback(_, FFEffectCode::FF_AUTOCENTER, value) => {
                    ops.push(ffb::Op::Autocenter(value as f32 / 65535.0))
                }
                EventSummary::ForceFeedback(_, code, value) => {
                    let id = code.0 as i16;
                    ops.push(if value > 0 {
                        ffb::Op::Play { id, count: value }
                    } else {
                        ffb::Op::Stop { id }
                    });
                }
                _ => {}
            }
        }
        Ok(ops)
    }
}

/// Shift, Green and Orange are one-shot like on the console: tap one, then the key. Holding
/// the modifier while pressing the key works too.
const STICKY: [u8; 3] = [modifier::SHIFT, modifier::GREEN, modifier::ORANGE];

/// What a chatpad report should do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChatpadOutput {
    /// Key events for the shared keyboard.
    pub keys: Vec<(KeyCode, bool)>,
    /// Press (`true`) or release the Steam Quick Access shortcut (Guide+A) on this controller's pad.
    pub quick_menu: Option<bool>,
    /// Caps Lock was toggled (Shift + Orange).
    pub caps_toggled: bool,
    /// Open Steam's friends & chat window (People).
    pub open_chat: bool,
    /// Chatpad keys newly pressed (for the settings app's "press a key" picker).
    pub pressed: Vec<ChatpadKey>,
}

/// Gap between the steps of a button chord.
const CHORD_STEP: Duration = Duration::from_millis(80);

/// Turns one chatpad's key reports into key events for the shared keyboard.
///
/// Special combinations (either key may be held or tapped first):
/// - Shift + Orange toggles Caps Lock.
/// - Green + People opens the Steam Quick Access menu; People alone opens Steam chat.
#[derive(Default)]
pub struct ChatpadState {
    modifiers: u8,
    /// One-shot modifiers waiting for the next key.
    latched: u8,
    /// Held modifiers that were used for something (so releasing them doesn't latch).
    used: u8,
    /// Whether the held People key is acting as the Quick Access shortcut rather than Meta.
    people_quick_menu: bool,
    /// Raw codes currently held, with the chord held for each (released in reverse).
    held: Vec<(u8, Vec<KeyCode>)>,
}

impl ChatpadState {
    pub fn update(&mut self, report: ChatpadKeys, keymap: &Keymap) -> ChatpadOutput {
        let mut out = ChatpadOutput::default();
        let pressed = report.modifiers & !self.modifiers;
        let released = self.modifiers & !report.modifiers;
        self.used &= !pressed;

        if pressed & modifier::PEOPLE != 0 {
            let green = (report.modifiers | self.latched) & modifier::GREEN;
            self.people_quick_menu = green != 0;
            if self.people_quick_menu {
                self.latched &= !modifier::GREEN;
                self.used |= green & report.modifiers;
                out.quick_menu = Some(true);
            } else {
                out.open_chat = true;
            }
        }
        if released & modifier::PEOPLE != 0 && self.people_quick_menu {
            out.quick_menu = Some(false);
        }

        for bit in STICKY {
            if released & bit == 0 || self.used & bit != 0 {
                continue;
            }
            let caps_partner = match bit {
                modifier::SHIFT => modifier::ORANGE,
                modifier::ORANGE => modifier::SHIFT,
                _ => 0,
            };
            if (report.modifiers | self.latched) & caps_partner != 0 {
                out.keys.extend([
                    (KeyCode::KEY_CAPSLOCK, true),
                    (KeyCode::KEY_CAPSLOCK, false),
                ]);
                out.caps_toggled = true;
                self.latched &= !(bit | caps_partner);
                self.used |= caps_partner & report.modifiers;
                continue;
            }
            self.latched ^= bit;
            // Green and Orange are alternatives; latching one cancels the other.
            if bit != modifier::SHIFT && self.latched & bit != 0 {
                self.latched &= !(modifier::GREEN | modifier::ORANGE) | bit;
            }
        }
        self.modifiers = report.modifiers;

        // Release keys no longer reported.
        self.held.retain(|(code, chord)| {
            if report.keys.contains(code) {
                return true;
            }
            out.keys.extend(chord.iter().rev().map(|&k| (k, false)));
            false
        });

        for &code in report.keys.iter().filter(|&&c| c != 0) {
            if self.held.iter().any(|(c, _)| *c == code) {
                continue;
            }
            let active = report.modifiers | self.latched;
            let layer = if active & modifier::GREEN != 0 {
                Layer::Green
            } else if active & modifier::ORANGE != 0 {
                Layer::Orange
            } else {
                Layer::Normal
            };
            let combo = match ChatpadKey::from_code(code) {
                Some(key) => {
                    out.pressed.push(key);
                    keymap.combo(layer, key, active & modifier::SHIFT != 0)
                }
                None => {
                    eprintln!("unknown chatpad key code {code:#04x}");
                    None
                }
            };
            let chord = match combo {
                Some(combo) => {
                    for tap in &combo.taps {
                        out.keys.extend(tap.iter().map(|&k| (k, true)));
                        out.keys.extend(tap.iter().rev().map(|&k| (k, false)));
                    }
                    out.keys.extend(combo.hold.iter().map(|&k| (k, true)));
                    combo.hold
                }
                None => Vec::new(),
            };
            self.held.push((code, chord));
            self.latched = 0;
            self.used |= report.modifiers;
        }
        out
    }

    /// Indicator lights (`led::*` bits) this state should show: Caps for the system's Caps
    /// Lock or a latched Shift, Green/Orange while latched or held.
    pub fn indicators(&self, caps_lock: bool) -> u8 {
        let active = self.modifiers | self.latched;
        let mut leds = 0;
        if caps_lock || self.latched & modifier::SHIFT != 0 {
            leds |= led::CAPS;
        }
        if active & modifier::GREEN != 0 {
            leds |= led::GREEN;
        }
        if active & modifier::ORANGE != 0 {
            leds |= led::ORANGE;
        }
        leds
    }

    /// Release everything (controller disconnected or daemon stopping).
    pub fn release_all(&mut self, keymap: &Keymap) -> ChatpadOutput {
        let out = self.update(ChatpadKeys::default(), keymap);
        self.latched = 0;
        out
    }
}

fn set_nonblocking(dev: &VirtualDevice) -> io::Result<()> {
    let fd = dev.as_raw_fd();
    // SAFETY: plain fcntl on a file descriptor we own for the duration of the call.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;
    use KeyCode as K;

    /// Plays the game's side against a real uinput wheel: upload, play, stop and erase a
    /// constant force through the event node, and check what `poll_ff` reports.
    #[test]
    #[ignore = "needs /dev/uinput and access to the new event node"]
    fn wheel_reports_uploaded_effects() {
        use evdev::{FFEffectData, FFEnvelope, FFReplay, FFTrigger};

        let mut wheel = Wheel::new(WheelLayout::Generic).unwrap();
        let node = wheel
            .dev
            .enumerate_dev_nodes_blocking()
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let game = std::thread::spawn(move || {
            // udev needs a moment to apply the joystick ACL.
            std::thread::sleep(Duration::from_millis(300));
            let mut device = evdev::Device::open(&node).unwrap();
            let mut effect = device
                .upload_ff_effect(FFEffectData {
                    direction: 0xc000,
                    trigger: FFTrigger {
                        button: 0,
                        interval: 0,
                    },
                    replay: FFReplay {
                        length: 250,
                        delay: 0,
                    },
                    kind: FFEffectKind::Constant {
                        level: 0x4000,
                        envelope: FFEnvelope {
                            attack_length: 0,
                            attack_level: 0,
                            fade_length: 0,
                            fade_level: 0,
                        },
                    },
                })
                .unwrap();
            effect.play(1).unwrap();
            effect.stop().unwrap();
            // Dropping the effect erases it.
        });
        let mut ops = Vec::new();
        let until = Instant::now() + Duration::from_secs(3);
        while !game.is_finished() && Instant::now() < until {
            ops.extend(wheel.poll_ff().unwrap());
            std::thread::sleep(Duration::from_millis(5));
        }
        game.join().unwrap();
        ops.extend(wheel.poll_ff().unwrap());

        let [ffb::Op::Upload { id, effect }, rest @ ..] = ops.as_slice() else {
            panic!("{ops:?}")
        };
        let ffb::Kind::Constant { force, .. } = effect.kind else {
            panic!("{effect:?}")
        };
        assert!((force + 0.5).abs() < 1e-3, "{force}");
        assert_eq!(effect.duration, Some(Duration::from_millis(250)));
        assert_eq!(
            rest,
            [
                ffb::Op::Play { id: *id, count: 1 },
                ffb::Op::Stop { id: *id },
                // The input core stops an effect again before erasing it.
                ffb::Op::Stop { id: *id },
                ffb::Op::Erase { id: *id }
            ]
        );
    }

    fn report(modifiers: u8, keys: &[u8]) -> ChatpadKeys {
        let mut r = ChatpadKeys {
            modifiers,
            keys: [0; 2],
        };
        r.keys[..keys.len()].copy_from_slice(keys);
        r
    }

    const A: u8 = 0x37;
    const Q: u8 = 0x27;

    #[test]
    fn plain_key() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        assert_eq!(s.update(report(0, &[A]), &map).keys, vec![(K::KEY_A, true)]);
        assert_eq!(s.update(report(0, &[]), &map).keys, vec![(K::KEY_A, false)]);
    }

    #[test]
    fn held_shift() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        assert!(s.update(report(modifier::SHIFT, &[]), &map).keys.is_empty());
        assert_eq!(
            s.update(report(modifier::SHIFT, &[A]), &map).keys,
            vec![(K::KEY_LEFTSHIFT, true), (K::KEY_A, true)]
        );
        s.update(report(modifier::SHIFT, &[]), &map);
        s.update(report(0, &[]), &map);
        // Shift was used while held, so it must not latch for the next key.
        assert_eq!(s.update(report(0, &[A]), &map).keys, vec![(K::KEY_A, true)]);
    }

    #[test]
    fn tapped_green_applies_once() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        s.update(report(modifier::GREEN, &[]), &map);
        s.update(report(0, &[]), &map);
        // Green+Q = '!'
        assert_eq!(
            s.update(report(0, &[Q]), &map).keys,
            vec![(K::KEY_LEFTSHIFT, true), (K::KEY_1, true)]
        );
        assert_eq!(
            s.update(report(0, &[]), &map).keys,
            vec![(K::KEY_1, false), (K::KEY_LEFTSHIFT, false)]
        );
        assert_eq!(s.update(report(0, &[Q]), &map).keys, vec![(K::KEY_Q, true)]);
    }

    #[test]
    fn tapping_green_twice_cancels() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        for _ in 0..2 {
            s.update(report(modifier::GREEN, &[]), &map);
            s.update(report(0, &[]), &map);
        }
        assert_eq!(s.update(report(0, &[Q]), &map).keys, vec![(K::KEY_Q, true)]);
    }

    #[test]
    fn orange_replaces_latched_green() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        for m in [modifier::GREEN, modifier::ORANGE] {
            s.update(report(m, &[]), &map);
            s.update(report(0, &[]), &map);
        }
        // Orange+R = '$'
        assert_eq!(
            s.update(report(0, &[0x24]), &map).keys,
            vec![(K::KEY_LEFTSHIFT, true), (K::KEY_4, true)]
        );
    }

    const CAPS: [(KeyCode, bool); 2] = [(K::KEY_CAPSLOCK, true), (K::KEY_CAPSLOCK, false)];

    #[test]
    fn shift_plus_orange_toggles_caps_lock() {
        let map = Keymap::new(Layout::us());
        // Hold Shift, tap Orange.
        let mut s = ChatpadState::default();
        s.update(report(modifier::SHIFT, &[]), &map);
        s.update(report(modifier::SHIFT | modifier::ORANGE, &[]), &map);
        assert_eq!(s.update(report(modifier::SHIFT, &[]), &map).keys, CAPS);
        s.update(report(0, &[]), &map);
        // Nothing left latched: the next key is plain.
        assert_eq!(s.update(report(0, &[A]), &map).keys, vec![(K::KEY_A, true)]);

        // Tap Orange, then tap Shift.
        let mut s = ChatpadState::default();
        s.update(report(modifier::ORANGE, &[]), &map);
        s.update(report(0, &[]), &map);
        s.update(report(modifier::SHIFT, &[]), &map);
        assert_eq!(s.update(report(0, &[]), &map).keys, CAPS);
        assert_eq!(s.update(report(0, &[A]), &map).keys, vec![(K::KEY_A, true)]);
    }

    #[test]
    fn green_plus_people_is_quick_menu() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        s.update(report(modifier::GREEN, &[]), &map);
        let down = s.update(report(modifier::GREEN | modifier::PEOPLE, &[]), &map);
        assert_eq!(
            down,
            ChatpadOutput {
                quick_menu: Some(true),
                ..Default::default()
            }
        );
        let up = s.update(report(modifier::GREEN, &[]), &map);
        assert_eq!(
            up,
            ChatpadOutput {
                quick_menu: Some(false),
                ..Default::default()
            }
        );
        // Green was used, so releasing it doesn't latch.
        s.update(report(0, &[]), &map);
        assert_eq!(s.update(report(0, &[Q]), &map).keys, vec![(K::KEY_Q, true)]);
    }

    #[test]
    fn tapped_green_then_people_is_quick_menu() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        s.update(report(modifier::GREEN, &[]), &map);
        s.update(report(0, &[]), &map);
        assert_eq!(
            s.update(report(modifier::PEOPLE, &[]), &map).quick_menu,
            Some(true)
        );
        assert_eq!(s.update(report(0, &[]), &map).quick_menu, Some(false));
    }

    #[test]
    fn people_alone_opens_chat() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        let down = s.update(report(modifier::PEOPLE, &[]), &map);
        assert!(down.open_chat && down.keys.is_empty() && down.quick_menu.is_none());
        let up = s.update(report(0, &[]), &map);
        assert_eq!(up, ChatpadOutput::default());
    }

    #[test]
    fn indicators_follow_latches_and_caps() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        s.update(report(modifier::GREEN, &[]), &map);
        assert_eq!(s.indicators(false), led::GREEN); // held
        s.update(report(0, &[]), &map);
        assert_eq!(s.indicators(false), led::GREEN); // latched
        s.update(report(0, &[Q]), &map);
        assert_eq!(s.indicators(false), 0); // consumed
        assert_eq!(s.indicators(true), led::CAPS); // system Caps Lock
        s.update(report(0, &[]), &map);
        s.update(report(modifier::SHIFT, &[]), &map);
        s.update(report(modifier::SHIFT | modifier::ORANGE, &[]), &map);
        assert!(s.update(report(modifier::SHIFT, &[]), &map).caps_toggled);
        s.update(report(0, &[]), &map);
        // Latched Shift lights Caps until used.
        s.update(report(modifier::SHIFT, &[]), &map);
        s.update(report(0, &[]), &map);
        assert_eq!(s.indicators(false), led::CAPS);
    }
}
