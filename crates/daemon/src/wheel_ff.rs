//! Encodes games' force-feedback requests ([`ffb::Op`]) as racing-wheel packets.
//!
//! The wheel stores effects and plays them by itself (`docs/protocol.md`, "Force found"):
//! a create block, a parameter block, an envelope for constant forces, then a start. A new
//! parameter block changes a playing effect at once, so updates only resend that block.
//!
//! Effects are always created to play until stopped, and the daemon starts and stops them
//! itself: the wheel's own duration tops out at 2.55 s and its loop field isn't mapped. The
//! envelope's fields aren't mapped either, so it is sent all zero, which plays the force as is.
//! Periodic and ramp effects are accepted but not played until their blocks are mapped.

use std::collections::HashMap;
use std::time::Instant;

use x360truewireless_proto::out::Packet;

use crate::ffb::{Condition, Effect, Kind, Op};

/// Effect types in the create block.
const TYPE_CONSTANT: u8 = 0x01;
const TYPE_SPRING: u8 = 0x08;
const TYPE_DAMPER: u8 = 0x09;
const TYPE_INERTIA: u8 = 0x0a;
const TYPE_FRICTION: u8 = 0x0b;

/// Which way a positive [`Kind::Constant`] force turns the rim: evdev direction `0x4000`
/// is "left" in the kernel's force-feedback docs, so positive forces push left (negative
/// wheel force).
const CONSTANT_SIGN: f32 = -1.0;

#[derive(Default)]
pub struct Encoder {
    effects: HashMap<i16, Slot>,
    /// The game's overall strength, 0.0..=1.0.
    gain: f32,
    /// The game's self-centring request; None until it sends one.
    autocenter: Option<f32>,
    /// Whether the wheel's own autocentre is on, as last sent (None: as the wheel started).
    native: Option<bool>,
    /// The device gain has been set.
    gain_sent: bool,
    rumble: Option<(u8, u8)>,
}

struct Slot {
    id: i16,
    effect: Effect,
    /// The create block's type, once the effect is on the wheel.
    created: Option<u8>,
    /// The parameter block needs resending.
    dirty: bool,
    state: Play,
}

#[derive(Clone, Copy, PartialEq)]
enum Play {
    Stopped,
    /// Waiting out the start delay; `left` more plays after this one.
    Starting {
        at: Instant,
        left: i32,
    },
    Playing {
        until: Option<Instant>,
        left: i32,
    },
}

impl Encoder {
    pub fn new() -> Self {
        Encoder {
            gain: 1.0,
            ..Default::default()
        }
    }

    /// Handle one request. Parameter updates are held back until [`Encoder::tick`], so a game
    /// that updates faster than the radio link only sends the latest values.
    pub fn op(&mut self, op: Op, now: Instant, out: &mut Vec<Packet>) {
        match op {
            Op::Upload { id, effect } => match self.effects.get_mut(&id) {
                Some(slot) => {
                    slot.effect = effect;
                    slot.dirty = true;
                }
                None => {
                    self.effects.insert(
                        id,
                        Slot {
                            id,
                            effect,
                            created: None,
                            dirty: true,
                            state: Play::Stopped,
                        },
                    );
                }
            },
            Op::Erase { id } => {
                if let Some(slot) = self.effects.remove(&id) {
                    stop_playing(&slot, out);
                    if slot.created.is_some() {
                        out.push(ff(0x0c, [index(id), 0, 0, 0, 0, 0, 0]));
                    }
                }
            }
            Op::Play { id, count } => {
                if let Some(slot) = self.effects.get_mut(&id) {
                    slot.state = Play::Starting {
                        at: now + slot.effect.delay,
                        left: count - 1,
                    };
                }
            }
            Op::Stop { id } => {
                if let Some(slot) = self.effects.get_mut(&id) {
                    stop_playing(slot, out);
                    slot.state = Play::Stopped;
                }
            }
            Op::Gain(gain) => {
                self.gain = gain.clamp(0.0, 1.0);
                for slot in self.effects.values_mut() {
                    slot.dirty = true;
                }
            }
            // Only on or off: the autocentre's strength is the device gain, which also scales
            // every effect, so it stays at full.
            Op::Autocenter(strength) => self.autocenter = Some(strength.clamp(0.0, 1.0)),
        }
        self.tick(now, out);
    }

    /// Send held-back updates, start and stop effects whose time has come, and switch the
    /// wheel's own autocentre. Call regularly (every few ms) while the wheel is connected.
    pub fn tick(&mut self, now: Instant, out: &mut Vec<Packet>) {
        if !self.gain_sent {
            // Device gain `0b 40`–`7f` scales every force; the game's gain is applied to the
            // effects instead, so the autocentre keeps its strength.
            out.push(device(0x7f));
            self.gain_sent = true;
        }
        let gain = self.gain;
        let mut rumble = None;
        let mut ids: Vec<i16> = self.effects.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let slot = self.effects.get_mut(&id).unwrap();
            if let Kind::Rumble { strong, weak } = slot.effect.kind {
                advance(slot, now, out);
                if matches!(slot.state, Play::Playing { .. }) {
                    rumble = Some((byte(strong * gain), byte(weak * gain)));
                }
                continue;
            }
            let Some(blocks) = blocks(&slot.effect.kind, gain, id) else {
                advance(slot, now, out);
                continue;
            };
            if slot.created != Some(blocks.kind) {
                if slot.created.is_some() {
                    out.push(ff(0x0c, [index(id), 0, 0, 0, 0, 0, 0]));
                }
                out.push(ff(
                    0x02,
                    [index(id), 0x05, blocks.kind, 0x00, 0xff, 0x00, 0x01],
                ));
                out.push(blocks.params);
                if blocks.envelope {
                    out.push(ff(0x03, [index(id), 0, 0, 0, 0, 0, 0]));
                }
                slot.created = Some(blocks.kind);
                slot.dirty = false;
                if matches!(slot.state, Play::Playing { .. }) {
                    // A new create replaces the playing effect; start it again.
                    slot.state = Play::Starting { at: now, left: 0 };
                }
            }
            if slot.dirty {
                // Only the parameter block: it changes the force without a restart.
                out.push(blocks.params);
                slot.dirty = false;
            }
            advance(slot, now, out);
        }
        if rumble != self.rumble {
            // The wheel's rumble bytes are swapped: the first drives the small motor.
            let (strong, weak) = rumble.unwrap_or((0, 0));
            out.push([0x00, 0x01, 0x0f, 0xc0, 0x00, weak, strong, 0, 0, 0, 0, 0]);
            self.rumble = rumble;
        }

        // The wheel's autocentre stays on unless the game switches it off. SDL (so Steam,
        // which keeps every joystick open) switches it off just by opening the wheel, so it
        // only counts while the game has force effects uploaded (not rumble, which Steam
        // keeps for itself); closing the game erases them.
        let forces = (self.effects.values()).any(|s| !matches!(s.effect.kind, Kind::Rumble { .. }));
        let native = !forces || self.autocenter.is_none_or(|s| s > 0.0);
        if self.native != Some(native) {
            out.push(device(if native { 0x20 } else { 0x10 }));
            self.native = Some(native);
        }
    }

    /// Full device gain and the wheel's own autocentre, for a wheel that games drive as a
    /// controller (rumble only), so no encoder runs.
    pub fn autocentre_only() -> [Packet; 2] {
        [device(0x7f), device(0x20)]
    }

    /// Stop everything and give the wheel its autocentre back, e.g. before disconnecting.
    pub fn stop_all(&mut self, out: &mut Vec<Packet>) {
        out.push(device(0x03));
        out.push(device(0x20));
        out.push([0x00, 0x01, 0x0f, 0xc0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for slot in self.effects.values_mut() {
            slot.state = Play::Stopped;
        }
        self.native = Some(true);
        self.rumble = None;
    }
}

/// Stop the wheel's effect if it is playing there.
fn stop_playing(slot: &Slot, out: &mut Vec<Packet>) {
    if slot.created.is_some() && matches!(slot.state, Play::Playing { .. }) {
        out.push(stop(slot.id));
    }
}

/// Move an effect through its start delay, plays and duration. Effects that aren't on the
/// wheel (rumble, unmapped types) only keep their timing.
fn advance(slot: &mut Slot, now: Instant, out: &mut Vec<Packet>) {
    match slot.state {
        Play::Starting { at, left } if now >= at => {
            if slot.created.is_some() {
                out.push(start(slot.id));
            }
            slot.state = Play::Playing {
                until: slot.effect.duration.map(|d| now + d),
                left,
            };
        }
        Play::Playing {
            until: Some(until),
            left,
        } if now >= until => {
            if left > 0 {
                // The next play: the wheel's effect keeps running, so just extend it.
                slot.state = Play::Playing {
                    until: slot.effect.duration.map(|d| now + d),
                    left: left - 1,
                };
            } else {
                stop_playing(slot, out);
                slot.state = Play::Stopped;
            }
        }
        _ => {}
    }
}

struct Blocks {
    /// The create block's type.
    kind: u8,
    params: Packet,
    /// Constant forces only play with an envelope block (all zero works).
    envelope: bool,
}

/// The create type and parameter blocks for an effect, or None if it can't be played yet.
fn blocks(kind: &Kind, gain: f32, id: i16) -> Option<Blocks> {
    let i = index(id);
    Some(match *kind {
        Kind::Constant { force, .. } => Blocks {
            kind: TYPE_CONSTANT,
            params: ff(
                0x08,
                [i, signed(force * gain * CONSTANT_SIGN), 0, 0, 0, 0, 0],
            ),
            envelope: true,
        },
        Kind::Condition {
            kind,
            center,
            right_coefficient,
            left_coefficient,
            right_saturation,
            left_saturation,
            deadband,
        } => Blocks {
            kind: match kind {
                Condition::Spring => TYPE_SPRING,
                Condition::Damper => TYPE_DAMPER,
                Condition::Inertia => TYPE_INERTIA,
                Condition::Friction => TYPE_FRICTION,
            },
            // Gain scales the coefficients and the saturations, so the force scales.
            params: ff(
                0x04,
                [
                    i,
                    signed(center),
                    signed(right_coefficient * gain),
                    signed(left_coefficient * gain),
                    signed(right_saturation * gain),
                    signed(left_saturation * gain),
                    signed(deadband),
                ],
            ),
            envelope: false,
        },
        Kind::Periodic { .. } | Kind::Ramp { .. } | Kind::Rumble { .. } => return None,
    })
}

/// Wheel effect index for an evdev effect id (0..16 → 1..17; the wheel rejects index 0).
fn index(id: i16) -> u8 {
    id as u8 + 1
}

/// A packet on the force-feedback channel: `00 01 0f c0` + report type + 7 bytes.
fn ff(kind: u8, payload: [u8; 7]) -> Packet {
    let mut p = [0x00, 0x01, 0x0f, 0xc0, kind, 0, 0, 0, 0, 0, 0, 0];
    p[5..].copy_from_slice(&payload);
    p
}

/// Effect operation: action 2 (start) or 1 (stop) on the index in the first slot byte.
fn start(id: i16) -> Packet {
    ff(0x0b, [0x08, 0x00, index(id), 0, 0, 0, 0])
}

fn stop(id: i16) -> Packet {
    ff(0x0b, [0x04, 0x00, index(id), 0, 0, 0, 0])
}

/// Header-only device control / gain: `00 00 0b xx`.
fn device(value: u8) -> Packet {
    [0x00, 0x00, 0x0b, value, 0, 0, 0, 0, 0, 0, 0, 0]
}

/// -1.0..=1.0 as a signed byte.
fn signed(v: f32) -> u8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8
}

/// 0.0..=1.0 as a rumble byte.
fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffb::Envelope;
    use std::time::Duration;

    fn constant(force: f32, duration: Option<Duration>) -> Effect {
        Effect {
            kind: Kind::Constant {
                force,
                envelope: Envelope::default(),
            },
            duration,
            delay: Duration::ZERO,
        }
    }

    fn hex(packets: &[Packet]) -> Vec<String> {
        packets
            .iter()
            .map(|p| {
                let n = if p[..4] == [0, 0, 0x0b, p[3]] { 4 } else { 12 };
                p[..n]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn constant_force_plays_updates_and_stops() {
        let mut e = Encoder::new();
        let t = Instant::now();
        let mut out = Vec::new();
        e.op(
            Op::Upload {
                id: 0,
                effect: constant(0.5, None),
            },
            t,
            &mut out,
        );
        e.op(Op::Play { id: 0, count: 1 }, t, &mut out);
        assert_eq!(
            hex(&out),
            [
                // The first tick also sets full device gain and the autocentre (on).
                "00 00 0b 7f",
                "00 01 0f c0 02 01 05 01 00 ff 00 01",
                "00 01 0f c0 08 01 c0 00 00 00 00 00",
                "00 01 0f c0 03 01 00 00 00 00 00 00",
                "00 00 0b 20",
                "00 01 0f c0 0b 08 00 01 00 00 00 00",
            ]
        );

        // Two updates in one poll: only the latest goes out, as a parameter block.
        out.clear();
        e.op(
            Op::Upload {
                id: 0,
                effect: constant(-0.25, None),
            },
            t,
            &mut out,
        );
        e.op(
            Op::Upload {
                id: 0,
                effect: constant(-1.0, None),
            },
            t,
            &mut out,
        );
        let mut updates = Vec::new();
        e.tick(t, &mut updates);
        assert_eq!(
            hex(&out),
            [
                "00 01 0f c0 08 01 20 00 00 00 00 00",
                "00 01 0f c0 08 01 7f 00 00 00 00 00"
            ]
        );
        assert!(updates.is_empty());

        out.clear();
        e.op(Op::Stop { id: 0 }, t, &mut out);
        e.op(Op::Erase { id: 0 }, t, &mut out);
        assert_eq!(
            hex(&out),
            [
                "00 01 0f c0 0b 04 00 01 00 00 00 00",
                "00 01 0f c0 0c 01 00 00 00 00 00 00"
            ]
        );
    }

    #[test]
    fn the_daemon_times_durations_and_repeats() {
        let mut e = Encoder::new();
        let t = Instant::now();
        let mut out = Vec::new();
        let ms = Duration::from_millis;
        e.op(
            Op::Upload {
                id: 3,
                effect: constant(0.5, Some(ms(100))),
            },
            t,
            &mut out,
        );
        e.op(Op::Play { id: 3, count: 2 }, t, &mut out);
        assert!(hex(&out).contains(&"00 01 0f c0 0b 08 00 04 00 00 00 00".to_string()));
        for at in [50, 120] {
            out.clear();
            e.tick(t + ms(at), &mut out);
            assert!(out.is_empty(), "{at} ms: {:?}", hex(&out));
        }
        out.clear();
        e.tick(t + ms(230), &mut out);
        assert_eq!(hex(&out), ["00 01 0f c0 0b 04 00 04 00 00 00 00"]);
    }

    #[test]
    fn spring_and_gain() {
        let mut e = Encoder::new();
        let t = Instant::now();
        let mut out = Vec::new();
        let spring = Effect {
            kind: Kind::Condition {
                kind: Condition::Spring,
                center: 0.0,
                right_coefficient: 1.0,
                left_coefficient: 1.0,
                right_saturation: 1.0,
                left_saturation: 1.0,
                deadband: 0.0,
            },
            duration: None,
            delay: Duration::ZERO,
        };
        e.op(
            Op::Upload {
                id: 1,
                effect: spring,
            },
            t,
            &mut out,
        );
        assert_eq!(
            hex(&out)[1..3],
            [
                "00 01 0f c0 02 02 05 08 00 ff 00 01",
                "00 01 0f c0 04 02 00 7f 7f 7f 7f 00"
            ]
        );
        out.clear();
        e.op(Op::Gain(0.5), t, &mut out);
        assert_eq!(hex(&out), ["00 01 0f c0 04 02 00 40 40 40 40 00"]);
    }

    #[test]
    fn game_autocentre_off_keeps_the_wheel_limp_while_it_has_effects() {
        let mut e = Encoder::new();
        let t = Instant::now();
        let mut out = Vec::new();
        // Opening the wheel switches the autocentre off, but it stays on with no effects.
        e.op(Op::Autocenter(0.0), t, &mut out);
        assert_eq!(hex(&out), ["00 00 0b 7f", "00 00 0b 20"]);
        out.clear();
        e.op(
            Op::Upload {
                id: 0,
                effect: constant(0.0, None),
            },
            t,
            &mut out,
        );
        assert_eq!(hex(&out).last().unwrap(), "00 00 0b 10");
        out.clear();
        e.op(Op::Autocenter(0.5), t, &mut out);
        assert_eq!(hex(&out), ["00 00 0b 20"]);
        out.clear();
        e.op(Op::Autocenter(0.0), t, &mut out);
        assert_eq!(hex(&out), ["00 00 0b 10"]);
        out.clear();
        // The game closes: its effects are erased and the wheel centres itself again.
        e.op(Op::Erase { id: 0 }, t, &mut out);
        assert_eq!(hex(&out).last().unwrap(), "00 00 0b 20");
    }
}
