//! The devices games see on Windows: Xbox 360 pads through ViGEmBus, and racing wheels as
//! vJoy joysticks with force feedback.

use std::io;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};
use x360truewireless_proto::{GamepadState, button};

use super::vjoy::{self, AXIS_MAX, HID_USAGE_RZ, HID_USAGE_X, HID_USAGE_Y};
use crate::devices::WheelLayout;
use crate::ffb;
use crate::pid;

fn vigem_error(e: vigem_client::Error) -> io::Error {
    match e {
        vigem_client::Error::BusNotFound => {
            io::Error::other("ViGEmBus is not installed; run the X360TrueWireless installer")
        }
        e => io::Error::other(format!("ViGEmBus: {e}")),
    }
}

pub struct VirtualPad {
    target: Xbox360Wired<Client>,
    /// Latest rumble from the game, set by ViGEm's notification thread.
    requested: Arc<Mutex<(u8, u8)>>,
    rumble: (u8, u8),
}

impl VirtualPad {
    pub fn new() -> io::Result<Self> {
        let client = Client::connect().map_err(vigem_error)?;
        let mut target = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
        target.plugin().map_err(vigem_error)?;
        target.wait_ready().map_err(vigem_error)?;
        let requested = Arc::new(Mutex::new((0, 0)));
        let shared = requested.clone();
        // The thread ends when the target is unplugged (dropped).
        target
            .request_notification()
            .map_err(vigem_error)?
            .spawn_thread(move |_, n| *shared.lock().unwrap() = (n.large_motor, n.small_motor));
        Ok(VirtualPad {
            target,
            requested,
            rumble: (0, 0),
        })
    }

    /// The controller's report maps one to one: XInput uses the same button bits.
    pub fn emit(&mut self, s: &GamepadState) -> io::Result<()> {
        let gamepad = XGamepad {
            buttons: XButtons { raw: s.buttons },
            left_trigger: s.lt,
            right_trigger: s.rt,
            thumb_lx: s.lx,
            thumb_ly: s.ly,
            thumb_rx: s.rx,
            thumb_ry: s.ry,
        };
        self.target.update(&gamepad).map_err(vigem_error)
    }

    /// The rumble strength `(strong, weak)` if the game changed it.
    pub fn poll_rumble(&mut self) -> io::Result<Option<(u8, u8)>> {
        let requested = *self.requested.lock().unwrap();
        Ok((requested != self.rumble).then(|| {
            self.rumble = requested;
            requested
        }))
    }
}

/// The wheel's buttons in DirectInput order: the G920's (see the Linux wheel), so profiles
/// made for one generic wheel carry over.
const BUTTONS: &[u16] = &[
    button::A,
    button::B,
    button::X,
    button::Y,
    button::RB,
    button::LB,
    button::START,
    button::BACK,
    button::RS,
    button::LS,
    button::GUIDE,
];

/// A racing wheel as the first free vJoy device: steering on X, throttle on Y and brake on Z
/// rotation (both 0 when released), the D-pad as a hat, and force feedback. vJoy has one
/// identity whatever the mode, so both wheel layouts look the same here.
pub struct Wheel {
    dev: vjoy::Device,
    layout: WheelLayout,
    ffb: Receiver<pid::Report>,
    translator: pid::Translator,
}

impl Wheel {
    pub fn new(_slot: u8, layout: WheelLayout) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        Ok(Wheel {
            dev: vjoy::Device::acquire_free(tx)?,
            layout,
            ffb: rx,
            translator: pid::Translator::default(),
        })
    }

    pub fn layout(&self) -> WheelLayout {
        self.layout
    }

    pub fn emit(&mut self, s: &GamepadState) -> io::Result<()> {
        let pedal = |v: u8| v as i32 * AXIS_MAX / 255;
        self.dev
            .set_axis(HID_USAGE_X, (s.lx as i32 + 32768) * AXIS_MAX / 65535);
        self.dev.set_axis(HID_USAGE_Y, pedal(s.rt));
        self.dev.set_axis(HID_USAGE_RZ, pedal(s.lt));
        for (n, &bit) in BUTTONS.iter().enumerate() {
            self.dev.set_button(n as u8 + 1, s.buttons & bit != 0);
        }
        let held = |bit: u16| s.buttons & bit != 0;
        let angle = match (
            held(button::DPAD_UP),
            held(button::DPAD_RIGHT),
            held(button::DPAD_DOWN),
            held(button::DPAD_LEFT),
        ) {
            (true, false, _, true) => Some(31500),
            (true, true, _, _) => Some(4500),
            (true, ..) => Some(0),
            (_, true, true, _) => Some(13500),
            (_, true, ..) => Some(9000),
            (_, _, true, true) => Some(22500),
            (_, _, true, _) => Some(18000),
            (.., true) => Some(27000),
            _ => None,
        };
        self.dev.set_pov(angle);
        Ok(())
    }

    /// Games' force-feedback requests since the last call, in order.
    pub fn poll_ff(&mut self) -> io::Result<Vec<ffb::Op>> {
        let mut ops = Vec::new();
        while let Ok(report) = self.ffb.try_recv() {
            ops.extend(self.translator.report(report));
        }
        Ok(ops)
    }
}
