//! The uinput devices games see: an Xbox 360 pad, or a racing wheel with force feedback.

use std::collections::HashMap;
use std::io;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventSummary, EventType, FFEffectCode,
    FFEffectKind, InputEvent, InputId, KeyCode, UInputCode, UinputAbsSetup,
};
use x360truewireless_proto::{GamepadState, button};

use super::ffb_raw;
use crate::devices::WheelLayout;
use crate::ffb;

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

pub struct VirtualPad {
    dev: VirtualDevice,
    /// Uploaded rumble effects: id → (strong, weak).
    effects: HashMap<i16, (u16, u16)>,
    /// When the currently playing effect ends on its own.
    stop_at: Option<Instant>,
    rumble: (u8, u8),
}

impl VirtualPad {
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
        Ok(VirtualPad {
            dev,
            effects: HashMap::new(),
            stop_at: None,
            rumble: (0, 0),
        })
    }

    pub fn emit(&mut self, s: &GamepadState) -> io::Result<()> {
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

    /// Handle pending force-feedback requests from games. Returns the new rumble strength
    /// `(strong, weak)` if it changed and should be sent to the controller.
    pub fn poll_rumble(&mut self) -> io::Result<Option<(u8, u8)>> {
        let mut target = None;
        let events: Vec<InputEvent> = match self.dev.fetch_events() {
            Ok(events) => events.collect(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Vec::new(),
            Err(e) => return Err(e),
        };
        for event in events {
            match event.destructure() {
                EventSummary::UInput(event, UInputCode::UI_FF_UPLOAD, ..) => {
                    let Some(mut upload) = withdrawn(self.dev.process_ff_upload(event))? else {
                        continue;
                    };
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
                    let Some(mut erase) = withdrawn(self.dev.process_ff_erase(event))? else {
                        continue;
                    };
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

const TMX_ID: (u16, u16, u16) = (0x044f, 0xb67f, 0x0100);
const TMX_NAME: &str = "Thrustmaster TMX Racing Wheel";
const G920_ID: (u16, u16, u16) = (0x046d, 0xc262, 0x0111);
const G920_NAME: &str = "Logitech G920 Driving Force Racing Wheel";

/// The TMX's DirectInput buttons in order, from what DiRT 4's TMX profile binds to each:
/// left paddle (gear down), right paddle (gear up), X (headlights), left stick (look back),
/// A (menu select), two unused, Menu (pause), B (menu back), Y (change view), right stick,
/// View (menu select and voice), Xbox. Left stick, right stick, View and Xbox are a best
/// guess; the rest match the G920 profile's choices for the same buttons.
const TMX_BUTTONS: &[(u16, KeyCode)] = &[
    (button::LB, KeyCode::BTN_TRIGGER),
    (button::RB, KeyCode::BTN_THUMB),
    (button::X, KeyCode::BTN_THUMB2),
    (button::LS, KeyCode::BTN_TOP),
    (button::A, KeyCode::BTN_TOP2),
    (0, KeyCode::BTN_PINKIE),
    (0, KeyCode::BTN_BASE),
    (button::START, KeyCode::BTN_BASE2),
    (button::B, KeyCode::BTN_BASE3),
    (button::Y, KeyCode::BTN_BASE4),
    (button::RS, KeyCode::BTN_BASE5),
    (button::BACK, KeyCode::BTN_BASE6),
    (button::GUIDE, KeyCode(0x12c)),
];

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
        WheelLayout::Tmx => TMX_BUTTONS,
        WheelLayout::G920 => G920_BUTTONS,
    }
}

/// The axes in key-code order, which is the order SDL and Proton number them in. For the
/// G920 Proton takes exactly four: steering, throttle, brake and clutch. Other devices get
/// X, Y, Z, X rotation, Y rotation, Z rotation and a slider in turn, so the TMX has three
/// idle axes to put its throttle on Z rotation and its clutch on the slider.
fn axes(layout: WheelLayout) -> Vec<(AbsoluteAxisCode, AbsInfo)> {
    let stick = AbsInfo::new(0, -32768, 32767, 0, 0, 0);
    let released = AbsInfo::new(255, 0, 255, 0, 0, 0);
    let idle = AbsInfo::new(0, -1, 1, 0, 0, 0);
    let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
    let mut axes = match layout {
        WheelLayout::Tmx => vec![
            (AbsoluteAxisCode::ABS_X, stick),
            (AbsoluteAxisCode::ABS_Y, released),
            (AbsoluteAxisCode::ABS_Z, idle),
            (AbsoluteAxisCode::ABS_RX, idle),
            (AbsoluteAxisCode::ABS_RY, idle),
            (AbsoluteAxisCode::ABS_RZ, released),
            (AbsoluteAxisCode::ABS_THROTTLE, released),
        ],
        WheelLayout::G920 => vec![
            (AbsoluteAxisCode::ABS_X, stick),
            (AbsoluteAxisCode::ABS_Y, released),
            (AbsoluteAxisCode::ABS_Z, released),
            (AbsoluteAxisCode::ABS_RZ, released),
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
    pub fn new(_slot: u8, layout: WheelLayout) -> io::Result<Self> {
        let (id, name) = match layout {
            WheelLayout::Tmx => (TMX_ID, TMX_NAME),
            WheelLayout::G920 => (G920_ID, G920_NAME),
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
        // Released pedals read full, as on both wheels. The 360 wheel has no clutch, so it
        // stays released.
        match self.layout {
            WheelLayout::Tmx => axes.extend([
                (AbsoluteAxisCode::ABS_Y, 255 - s.lt as i32),
                (AbsoluteAxisCode::ABS_RZ, 255 - s.rt as i32),
                (AbsoluteAxisCode::ABS_THROTTLE, 255),
            ]),
            // Throttle, brake and clutch. Proton's DirectInput renames a G920's Z to Z rotation
            // ("Brake") and its Z rotation to a slider ("Clutch"), as on Windows.
            WheelLayout::G920 => axes.extend([
                (AbsoluteAxisCode::ABS_Y, 255 - s.rt as i32),
                (AbsoluteAxisCode::ABS_Z, 255 - s.lt as i32),
                (AbsoluteAxisCode::ABS_RZ, 255),
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
                    let Some(upload) =
                        withdrawn(ffb_raw::Upload::begin(self.dev.as_raw_fd(), request))?
                    else {
                        continue;
                    };
                    // The input core has already picked a free id.
                    let id = upload.id();
                    match ffb_raw::from_raw(upload.raw()) {
                        Some(effect) if (0..FF_EFFECTS_MAX as i16).contains(&id) => {
                            withdrawn(upload.finish(Ok(id)))?;
                            self.effects.insert(id, effect);
                            ops.push(ffb::Op::Upload { id, effect });
                        }
                        _ => {
                            withdrawn(upload.finish(Err(libc::EINVAL)))?;
                        }
                    }
                }
                EventSummary::UInput(event, UInputCode::UI_FF_ERASE, ..) => {
                    let Some(mut erase) = withdrawn(self.dev.process_ff_erase(event))? else {
                        continue;
                    };
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

/// A force-feedback request the game has already given up on (it waited too long, or its
/// device was swapped under it): the kernel answers `EINVAL`, and there's nothing to do.
fn withdrawn<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.raw_os_error() == Some(libc::EINVAL) => {
            eprintln!("skipped a force-feedback request the game withdrew");
            Ok(None)
        }
        Err(e) => Err(e),
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

    /// Plays the game's side against a real uinput wheel: upload, play, stop and erase a
    /// constant force through the event node, and check what `poll_ff` reports.
    #[test]
    #[ignore = "needs /dev/uinput and access to the new event node"]
    fn wheel_reports_uploaded_effects() {
        use evdev::{FFEffectData, FFEnvelope, FFReplay, FFTrigger};

        let mut wheel = Wheel::new(0, WheelLayout::Tmx).unwrap();
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
}
