//! The virtual devices games see: one Xbox 360 pad per connected controller, or a racing
//! wheel. The platform module creates them (uinput on Linux, ViGEmBus and vJoy on Windows);
//! the chatpad shortcuts on top of a pad are handled here.

use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use x360truewireless_proto::{GamepadState, button};

use crate::platform::VirtualPad;
pub use crate::platform::Wheel;

/// Gap between the steps of a button chord.
const CHORD_STEP: Duration = Duration::from_millis(80);

pub struct Pad {
    dev: VirtualPad,
    /// Last state from the controller, and buttons we hold down on top of it (chatpad shortcuts).
    last: GamepadState,
    overlay: u16,
    /// Overlay changes scheduled for later (for button sequences like Guide-then-A).
    overlay_steps: VecDeque<(Instant, u16)>,
}

impl Pad {
    pub fn new() -> io::Result<Self> {
        Ok(Pad {
            dev: VirtualPad::new()?,
            last: GamepadState::default(),
            overlay: 0,
            overlay_steps: VecDeque::new(),
        })
    }

    pub fn emit(&mut self, state: &GamepadState) -> io::Result<()> {
        self.last = *state;
        self.dev.emit(&GamepadState {
            buttons: state.buttons | self.overlay,
            ..*state
        })
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
        self.dev.poll_rumble()
    }
}

/// Which wheel the virtual wheel presents itself as. SDL decides a joystick is a wheel only
/// from a list of USB IDs, and Proton presents only those to games as DirectInput driving
/// devices with full force feedback; games such as SnowRunner list nothing else as a wheel.
/// So both layouts take an ID from that list, and copy that wheel's buttons and axes so games
/// load their own profile for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelLayout {
    /// A Thrustmaster TMX, Thrustmaster's Xbox-layout wheel, with two pedals like the 360
    /// wheel. Its report descriptor (from a USB capture of one, in emtek995/TMX-driver's
    /// `wheel.log`) has X steering, Y brake, Z rotation throttle, a slider clutch, 13 buttons
    /// and a hat. Button numbers are from DiRT 4's `input/actionmaps/tm_tmx.xml`.
    Tmx,
    /// A Logitech G920, the Xbox-layout Logitech wheel (button numbers and axes from DiRT 4's
    /// `input/actionmaps/lg_g920.xml`).
    G920,
}
