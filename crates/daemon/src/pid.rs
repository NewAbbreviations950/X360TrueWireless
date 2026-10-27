//! Force feedback from USB HID PID reports (what Windows' DirectInput sends a force-feedback
//! joystick, passed on to us by vJoy) as [`ffb::Op`]s.
//!
//! A game sets an effect up in pieces, each a report naming the effect block: the effect report
//! (type, duration, gain, direction) and a type-specific block (constant, ramp, periodic,
//! condition) plus an envelope, in any order. Each change to an effect whose effect report has
//! arrived is passed on as an upload, which `wheel_ff` turns into an update of a playing effect.
//! Values follow vJoy's scaling: magnitudes are -10000..=10000, times are milliseconds.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::time::Duration;

use crate::ffb::{Condition, Effect, Envelope, Kind, Op, Wave};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectType {
    Constant,
    Ramp,
    Square,
    Sine,
    Triangle,
    SawUp,
    SawDown,
    Spring,
    Damper,
    Inertia,
    Friction,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Direction {
    /// Clockwise from north as a fraction of a turn (0.25 is east, +X).
    Polar(f32),
    /// The X component's sign is all that matters for a wheel.
    Cartesian { x: i16 },
    /// No direction (the report doesn't enable it): the effect works on the axes it enables,
    /// in the direction of its own sign.
    Axes { x: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Start,
    /// Start this effect and stop every other.
    Solo,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    EnableActuators,
    DisableActuators,
    StopAll,
    Reset,
    Pause,
    Continue,
}

/// One PID report, decoded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Report {
    Effect {
        index: u8,
        kind: EffectType,
        /// None plays until stopped.
        duration_ms: Option<u16>,
        start_delay_ms: u16,
        /// 0..=255.
        gain: u8,
        direction: Direction,
    },
    Envelope {
        index: u8,
        attack_level: u32,
        fade_level: u32,
        attack_ms: u32,
        fade_ms: u32,
    },
    Condition {
        index: u8,
        /// Wheels only use the X (steering) axis block.
        is_y: bool,
        center: i32,
        positive_coefficient: i32,
        negative_coefficient: i32,
        positive_saturation: u32,
        negative_saturation: u32,
        deadband: i32,
    },
    Periodic {
        index: u8,
        magnitude: u32,
        offset: i32,
        /// Hundredths of a degree, 0..36000.
        phase: u32,
        period_ms: u32,
    },
    Constant {
        index: u8,
        magnitude: i32,
    },
    Ramp {
        index: u8,
        start: i32,
        end: i32,
    },
    Operation {
        index: u8,
        op: Operation,
        /// 255 repeats until stopped.
        loop_count: u8,
    },
    Free {
        index: u8,
    },
    Control(Control),
    /// 0..=255.
    Gain(u8),
}

/// What's known about one effect block so far.
#[derive(Debug, Default, Clone, Copy)]
struct Partial {
    report: Option<(EffectType, Option<Duration>, Duration, f32, f32)>,
    envelope: Envelope,
    condition: Option<Report>,
    periodic: Option<Report>,
    constant: i32,
    ramp: (i32, i32),
}

/// Turns PID reports into force-feedback operations.
#[derive(Default)]
pub struct Translator {
    effects: HashMap<u8, Partial>,
    /// Whether the wheel's own autocentre has been switched off for the game's forces.
    forces: bool,
}

impl Translator {
    pub fn report(&mut self, report: Report) -> Vec<Op> {
        let mut ops = Vec::new();
        let index = match report {
            Report::Effect {
                index,
                kind,
                duration_ms,
                start_delay_ms,
                gain,
                direction,
            } => {
                let along = match direction {
                    Direction::Polar(turns) => (turns * TAU).sin(),
                    Direction::Cartesian { x } => x.signum() as f32,
                    Direction::Axes { x } => f32::from(u8::from(x)),
                };
                self.effects.entry(index).or_default().report = Some((
                    kind,
                    duration_ms.filter(|&d| d != 0).map(ms),
                    ms(start_delay_ms),
                    gain as f32 / 255.0,
                    along,
                ));
                index
            }
            Report::Envelope {
                index,
                attack_level,
                fade_level,
                attack_ms,
                fade_ms,
            } => {
                self.effects.entry(index).or_default().envelope = Envelope {
                    attack_level: unsigned(attack_level),
                    attack: ms(attack_ms),
                    fade_level: unsigned(fade_level),
                    fade: ms(fade_ms),
                };
                index
            }
            Report::Condition { index, is_y, .. } => {
                if is_y {
                    return ops;
                }
                self.effects.entry(index).or_default().condition = Some(report);
                index
            }
            Report::Periodic { index, .. } => {
                self.effects.entry(index).or_default().periodic = Some(report);
                index
            }
            Report::Constant { index, magnitude } => {
                self.effects.entry(index).or_default().constant = magnitude;
                index
            }
            Report::Ramp { index, start, end } => {
                self.effects.entry(index).or_default().ramp = (start, end);
                index
            }
            Report::Operation {
                index,
                op,
                loop_count,
            } => {
                let id = index as i16;
                match op {
                    Operation::Stop => ops.push(Op::Stop { id }),
                    Operation::Start | Operation::Solo => {
                        if op == Operation::Solo {
                            ops.extend(
                                self.effects
                                    .keys()
                                    .filter(|&&other| other != index)
                                    .map(|&other| Op::Stop { id: other as i16 }),
                            );
                        }
                        let count = match loop_count {
                            255 => i32::MAX,
                            n => n.max(1) as i32,
                        };
                        ops.push(Op::Play { id, count });
                    }
                }
                return ops;
            }
            Report::Free { index } => {
                if self.effects.remove(&index).is_some() {
                    ops.push(Op::Erase { id: index as i16 });
                }
                return ops;
            }
            Report::Control(control) => {
                match control {
                    Control::StopAll | Control::DisableActuators | Control::Pause => {
                        ops.extend(self.effects.keys().map(|&i| Op::Stop { id: i as i16 }))
                    }
                    Control::Reset => {
                        ops.extend(
                            self.effects
                                .drain()
                                .map(|(i, _)| Op::Erase { id: i as i16 }),
                        );
                        // No game forces left: the wheel centres itself again.
                        if self.forces {
                            self.forces = false;
                            ops.push(Op::Autocenter(1.0));
                        }
                    }
                    Control::EnableActuators | Control::Continue => {}
                }
                return ops;
            }
            Report::Gain(gain) => {
                ops.push(Op::Gain(gain as f32 / 255.0));
                return ops;
            }
        };
        if let Some(effect) = self.effects.get(&index).and_then(effect) {
            // DirectInput has no autocentre switch for a PID device; the wheel's own centring
            // would fight the game's forces, so it goes off once the game sets any up.
            if !self.forces {
                self.forces = true;
                ops.push(Op::Autocenter(0.0));
            }
            ops.push(Op::Upload {
                id: index as i16,
                effect,
            });
        }
        ops
    }
}

/// The effect as far as it's known, once its effect report has arrived.
fn effect(p: &Partial) -> Option<Effect> {
    let (kind, duration, delay, gain, along) = p.report?;
    let wave = match kind {
        EffectType::Square => Some(Wave::Square),
        EffectType::Sine => Some(Wave::Sine),
        EffectType::Triangle => Some(Wave::Triangle),
        EffectType::SawUp => Some(Wave::SawUp),
        EffectType::SawDown => Some(Wave::SawDown),
        _ => None,
    };
    let kind = match kind {
        EffectType::Constant => Kind::Constant {
            force: signed(p.constant) * gain * along,
            envelope: p.envelope,
        },
        EffectType::Ramp => Kind::Ramp {
            start: signed(p.ramp.0) * gain * along,
            end: signed(p.ramp.1) * gain * along,
            envelope: p.envelope,
        },
        EffectType::Spring | EffectType::Damper | EffectType::Inertia | EffectType::Friction => {
            let Some(Report::Condition {
                center,
                positive_coefficient,
                negative_coefficient,
                positive_saturation,
                negative_saturation,
                deadband,
                ..
            }) = p.condition
            else {
                return None;
            };
            Kind::Condition {
                kind: match kind {
                    EffectType::Spring => Condition::Spring,
                    EffectType::Damper => Condition::Damper,
                    EffectType::Inertia => Condition::Inertia,
                    _ => Condition::Friction,
                },
                center: signed(center),
                right_coefficient: signed(positive_coefficient) * gain,
                left_coefficient: signed(negative_coefficient) * gain,
                right_saturation: unsigned(positive_saturation),
                left_saturation: unsigned(negative_saturation),
                deadband: unsigned(deadband.max(0) as u32),
            }
        }
        _ => {
            let Some(Report::Periodic {
                magnitude,
                offset,
                phase,
                period_ms,
                ..
            }) = p.periodic
            else {
                return None;
            };
            Kind::Periodic {
                wave: wave?,
                magnitude: unsigned(magnitude) * gain * along,
                offset: signed(offset) * gain * along,
                period: ms(period_ms),
                phase: (phase % 36000) as f32 / 36000.0,
                envelope: p.envelope,
            }
        }
    };
    Some(Effect {
        kind,
        duration,
        delay,
    })
}

fn signed(v: i32) -> f32 {
    (v as f32 / 10000.0).clamp(-1.0, 1.0)
}

fn unsigned(v: u32) -> f32 {
    (v as f32 / 10000.0).min(1.0)
}

fn ms(v: impl Into<u64>) -> Duration {
    Duration::from_millis(v.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect_report(index: u8, kind: EffectType, turns: f32) -> Report {
        Report::Effect {
            index,
            kind,
            duration_ms: None,
            start_delay_ms: 0,
            gain: 255,
            direction: Direction::Polar(turns),
        }
    }

    #[test]
    fn constant_force_needs_its_effect_report() {
        let mut t = Translator::default();
        assert!(
            t.report(Report::Constant {
                index: 1,
                magnitude: 5000
            })
            .is_empty()
        );
        let ops = t.report(effect_report(1, EffectType::Constant, 0.25));
        let [Op::Autocenter(off), Op::Upload { id: 1, effect }] = ops.as_slice() else {
            panic!("{ops:?}")
        };
        assert_eq!(*off, 0.0);
        let Kind::Constant { force, .. } = effect.kind else {
            panic!("{effect:?}")
        };
        assert!((force - 0.5).abs() < 1e-3, "{force}");
        assert_eq!(effect.duration, None);

        // A later magnitude updates the uploaded effect; west pushes the other way.
        let ops = t.report(effect_report(1, EffectType::Constant, 0.75));
        let [Op::Upload { effect, .. }] = ops.as_slice() else {
            panic!("{ops:?}")
        };
        let Kind::Constant { force, .. } = effect.kind else {
            panic!("{effect:?}")
        };
        assert!((force + 0.5).abs() < 1e-3, "{force}");
    }

    #[test]
    fn constant_force_without_a_direction_keeps_its_sign() {
        let mut t = Translator::default();
        t.report(Report::Constant {
            index: 1,
            magnitude: -4000,
        });
        let ops = t.report(Report::Effect {
            index: 1,
            kind: EffectType::Constant,
            duration_ms: None,
            start_delay_ms: 0,
            gain: 255,
            direction: Direction::Axes { x: true },
        });
        let Some(Op::Upload { effect, .. }) = ops.last() else {
            panic!("{ops:?}")
        };
        let Kind::Constant { force, .. } = effect.kind else {
            panic!("{effect:?}")
        };
        assert!((force + 0.4).abs() < 1e-3, "{force}");
    }

    #[test]
    fn spring_uses_the_x_axis_block() {
        let mut t = Translator::default();
        t.report(effect_report(2, EffectType::Spring, 0.0));
        let condition = |is_y, coefficient| Report::Condition {
            index: 2,
            is_y,
            center: 0,
            positive_coefficient: coefficient,
            negative_coefficient: coefficient,
            positive_saturation: 10000,
            negative_saturation: 10000,
            deadband: 0,
        };
        assert!(t.report(condition(true, 1000)).is_empty());
        let ops = t.report(condition(false, 5000));
        let [Op::Autocenter(_), Op::Upload { effect, .. }] = ops.as_slice() else {
            panic!("{ops:?}")
        };
        let Kind::Condition {
            kind,
            right_coefficient,
            ..
        } = effect.kind
        else {
            panic!("{effect:?}")
        };
        assert_eq!(kind, Condition::Spring);
        assert!((right_coefficient - 0.5).abs() < 1e-3);
    }

    #[test]
    fn operations_free_and_reset() {
        let mut t = Translator::default();
        for index in [1, 2] {
            t.report(Report::Constant {
                index,
                magnitude: 1000,
            });
            t.report(effect_report(index, EffectType::Constant, 0.25));
        }
        assert_eq!(
            t.report(Report::Operation {
                index: 1,
                op: Operation::Start,
                loop_count: 255
            }),
            [Op::Play {
                id: 1,
                count: i32::MAX
            }]
        );
        assert_eq!(
            t.report(Report::Operation {
                index: 2,
                op: Operation::Solo,
                loop_count: 1
            }),
            [Op::Stop { id: 1 }, Op::Play { id: 2, count: 1 }]
        );
        assert_eq!(t.report(Report::Free { index: 2 }), [Op::Erase { id: 2 }]);
        assert_eq!(
            t.report(Report::Control(Control::Reset)),
            [Op::Erase { id: 1 }, Op::Autocenter(1.0)]
        );
        assert_eq!(t.report(Report::Gain(255)), [Op::Gain(1.0)]);
    }
}
