//! Force feedback for racing wheels: games' effects in a device-neutral form.
//!
//! Like a USB PID device, the wheel plays effects by itself once they're uploaded (see the
//! Microsoft patent cited in `docs/protocol.md`). So this keeps each effect as the game
//! described it and only normalises units: forces are -1.0..=1.0 along the steering axis,
//! times are `Duration`s. `wheel_ff` encodes them as wheel packets. Each platform's virtual
//! wheel turns games' requests into these.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effect {
    pub kind: Kind,
    /// How long one play lasts; None plays until stopped.
    pub duration: Option<Duration>,
    pub delay: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// Positive `force` is the game's +X direction (evdev direction `0x4000`). `wheel_ff`
    /// decides which way that turns the wheel.
    Constant { force: f32, envelope: Envelope },
    Ramp {
        start: f32,
        end: f32,
        envelope: Envelope,
    },
    Periodic {
        wave: Wave,
        magnitude: f32,
        offset: f32,
        period: Duration,
        /// Fraction of a period, 0.0..1.0.
        phase: f32,
        envelope: Envelope,
    },
    /// Forces that depend on the wheel's position or motion (only the steering axis is used).
    Condition {
        kind: Condition,
        center: f32,
        right_coefficient: f32,
        left_coefficient: f32,
        /// 0.0..=1.0.
        right_saturation: f32,
        left_saturation: f32,
        deadband: f32,
    },
    /// 0.0..=1.0 per motor. (PID devices on Windows have no rumble effect.)
    #[cfg_attr(windows, allow(dead_code))]
    Rumble { strong: f32, weak: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    Square,
    Triangle,
    Sine,
    SawUp,
    SawDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Spring,
    Damper,
    Friction,
    Inertia,
}

/// Fade-in and fade-out. Levels are 0.0..=1.0 of the effect's magnitude.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Envelope {
    pub attack_level: f32,
    pub attack: Duration,
    pub fade_level: f32,
    pub fade: Duration,
}

/// A force-feedback request from a game.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// A new effect, or new parameters for one already uploaded under `id`.
    Upload {
        id: i16,
        effect: Effect,
    },
    Erase {
        id: i16,
    },
    /// Play `id` `count` times.
    Play {
        id: i16,
        count: i32,
    },
    Stop {
        id: i16,
    },
    /// Overall strength, 0.0..=1.0.
    Gain(f32),
    /// Self-centring strength, 0.0 (off)..=1.0.
    Autocenter(f32),
}
