//! Force feedback for racing wheels: games' evdev effects in a device-neutral form.
//!
//! Like a USB PID device, the wheel plays effects by itself once they're uploaded (see the
//! Microsoft patent cited in `docs/protocol.md`). So this keeps each effect as the game
//! described it and only normalises units: forces are -1.0..=1.0 along the steering axis,
//! times are `Duration`s. `wheel_ff` encodes them as wheel packets.
//!
//! Uploads are read with our own ioctl rather than `evdev`'s `process_ff_upload`, whose
//! `FFEffectKind::Damper`/`Inertia` drop the condition data (the coefficients).

use std::f32::consts::TAU;
use std::io;
use std::os::fd::RawFd;
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
    /// 0.0..=1.0 per motor.
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

/// The effect a game uploaded, or None for a type we don't advertise.
pub fn from_raw(raw: &libc::ff_effect) -> Option<Effect> {
    // The force's share along the steering axis: evdev direction 0x4000 is +X.
    let along = (raw.direction as f32 / 65536.0 * TAU).sin();
    let kind = match raw.type_ {
        FF_CONSTANT => {
            let c: libc::ff_constant_effect = union_field(raw);
            Kind::Constant {
                force: signed(c.level) * along,
                envelope: envelope(&c.envelope),
            }
        }
        FF_RAMP => {
            let r: libc::ff_ramp_effect = union_field(raw);
            Kind::Ramp {
                start: signed(r.start_level) * along,
                end: signed(r.end_level) * along,
                envelope: envelope(&r.envelope),
            }
        }
        FF_PERIODIC => {
            let p: libc::ff_periodic_effect = union_field(raw);
            Kind::Periodic {
                wave: match p.waveform {
                    FF_SQUARE => Wave::Square,
                    FF_TRIANGLE => Wave::Triangle,
                    FF_SINE => Wave::Sine,
                    FF_SAW_UP => Wave::SawUp,
                    FF_SAW_DOWN => Wave::SawDown,
                    _ => return None,
                },
                magnitude: signed(p.magnitude) * along,
                offset: signed(p.offset) * along,
                period: ms(p.period),
                phase: p.phase as f32 / 65536.0,
                envelope: envelope(&p.envelope),
            }
        }
        FF_SPRING | FF_FRICTION | FF_DAMPER | FF_INERTIA => {
            // Two entries, one per axis; the first is X, the steering axis.
            let [x, _]: [libc::ff_condition_effect; 2] = union_field(raw);
            Kind::Condition {
                kind: match raw.type_ {
                    FF_SPRING => Condition::Spring,
                    FF_FRICTION => Condition::Friction,
                    FF_DAMPER => Condition::Damper,
                    _ => Condition::Inertia,
                },
                center: signed(x.center),
                right_coefficient: signed(x.right_coeff),
                left_coefficient: signed(x.left_coeff),
                right_saturation: unsigned(x.right_saturation),
                left_saturation: unsigned(x.left_saturation),
                deadband: unsigned(x.deadband),
            }
        }
        FF_RUMBLE => {
            let r: libc::ff_rumble_effect = union_field(raw);
            Kind::Rumble {
                strong: unsigned(r.strong_magnitude),
                weak: unsigned(r.weak_magnitude),
            }
        }
        _ => return None,
    };
    Some(Effect {
        kind,
        duration: (raw.replay.length != 0).then(|| ms(raw.replay.length)),
        delay: ms(raw.replay.delay),
    })
}

/// Read the member of `ff_effect`'s union that `type_` says is there.
fn union_field<T: Copy>(raw: &libc::ff_effect) -> T {
    const { assert!(size_of::<T>() <= size_of::<[u64; 4]>()) };
    // SAFETY: `u` is the kernel's union, and every member is plain integers; the size is
    // checked above. Unaligned read because `u` is declared as a u64 array.
    unsafe { std::ptr::read_unaligned(raw.u.as_ptr().cast::<T>()) }
}

fn envelope(e: &libc::ff_envelope) -> Envelope {
    Envelope {
        // Envelope levels are 0..=0x7fff.
        attack_level: (e.attack_level as f32 / 32767.0).min(1.0),
        attack: ms(e.attack_length),
        fade_level: (e.fade_level as f32 / 32767.0).min(1.0),
        fade: ms(e.fade_length),
    }
}

fn signed(v: i16) -> f32 {
    (v as f32 / 32767.0).max(-1.0)
}

fn unsigned(v: u16) -> f32 {
    v as f32 / 65535.0
}

fn ms(v: u16) -> Duration {
    Duration::from_millis(v as u64)
}

// Effect type and waveform codes from linux/input.h.
const FF_RUMBLE: u16 = 0x50;
const FF_PERIODIC: u16 = 0x51;
const FF_CONSTANT: u16 = 0x52;
const FF_SPRING: u16 = 0x53;
const FF_FRICTION: u16 = 0x54;
const FF_DAMPER: u16 = 0x55;
const FF_INERTIA: u16 = 0x56;
const FF_RAMP: u16 = 0x57;
const FF_SQUARE: u16 = 0x58;
const FF_TRIANGLE: u16 = 0x59;
const FF_SINE: u16 = 0x5a;
const FF_SAW_UP: u16 = 0x5b;
const FF_SAW_DOWN: u16 = 0x5c;

/// An effect upload in progress: the uinput side of `UI_FF_UPLOAD`. Call [`Upload::finish`]
/// to answer it; the game's ioctl blocks until then.
pub struct Upload {
    fd: RawFd,
    request: libc::uinput_ff_upload,
}

const fn ioc(dir: u64, nr: u64, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((b'U' as u64) << 8) | nr
}
const UI_BEGIN_FF_UPLOAD: u64 = ioc(3, 200, size_of::<libc::uinput_ff_upload>());
const UI_END_FF_UPLOAD: u64 = ioc(1, 201, size_of::<libc::uinput_ff_upload>());

impl Upload {
    /// Fetch the effect for the `UI_FF_UPLOAD` event whose value is `request_id`.
    pub fn begin(fd: RawFd, request_id: i32) -> io::Result<Upload> {
        // SAFETY: plain C struct; all-zero is a valid value.
        let mut request: libc::uinput_ff_upload = unsafe { std::mem::zeroed() };
        request.request_id = request_id as u32;
        // SAFETY: `request` is the struct this ioctl reads and writes.
        if unsafe { libc::ioctl(fd, UI_BEGIN_FF_UPLOAD as _, &mut request) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Upload { fd, request })
    }

    pub fn raw(&self) -> &libc::ff_effect {
        &self.request.effect
    }

    /// The effect's id. The input core assigns new effects a free one before we see them.
    pub fn id(&self) -> i16 {
        self.request.effect.id
    }

    /// Answer the upload: `Ok(id)` accepts it under that id, `Err(errno)` refuses it.
    pub fn finish(mut self, result: Result<i16, i32>) -> io::Result<()> {
        match result {
            Ok(id) => {
                self.request.effect.id = id;
                self.request.retval = 0;
            }
            Err(errno) => self.request.retval = -errno,
        }
        // SAFETY: as in `begin`.
        if unsafe { libc::ioctl(self.fd, UI_END_FF_UPLOAD as _, &self.request) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw<T>(type_: u16, direction: u16, length: u16, member: T) -> libc::ff_effect {
        // SAFETY: plain C struct; all-zero is a valid value.
        let mut e: libc::ff_effect = unsafe { std::mem::zeroed() };
        e.type_ = type_;
        e.direction = direction;
        e.replay.length = length;
        // SAFETY: test-only write of a union member that fits (checked by `union_field`).
        unsafe { std::ptr::write_unaligned(e.u.as_mut_ptr().cast::<T>(), member) };
        e
    }

    fn constant(level: i16) -> libc::ff_constant_effect {
        libc::ff_constant_effect {
            level,
            envelope: libc::ff_envelope {
                attack_length: 100,
                attack_level: 0x7fff,
                fade_length: 0,
                fade_level: 0,
            },
        }
    }

    #[test]
    fn ioctl_numbers_match_the_kernel() {
        // From <linux/uinput.h> on 64-bit: _IOWR('U', 200, 104 bytes), _IOW('U', 201, ...).
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(UI_BEGIN_FF_UPLOAD, 0xc068_55c8);
            assert_eq!(UI_END_FF_UPLOAD, 0x4068_55c9);
        }
    }

    #[test]
    fn constant_force_is_projected_onto_the_steering_axis() {
        let effect = |dir| from_raw(&raw(FF_CONSTANT, dir, 500, constant(16384))).unwrap();
        let force = |dir| match effect(dir).kind {
            Kind::Constant { force, .. } => force,
            k => panic!("{k:?}"),
        };
        assert!((force(0x4000) - 0.5).abs() < 1e-3);
        assert!((force(0xc000) + 0.5).abs() < 1e-3);
        // Straight "down" has no sideways part.
        assert!(force(0).abs() < 1e-3);

        let e = effect(0x4000);
        assert_eq!(e.duration, Some(Duration::from_millis(500)));
        let Kind::Constant { envelope, .. } = e.kind else {
            unreachable!()
        };
        assert_eq!(envelope.attack, Duration::from_millis(100));
        assert_eq!(envelope.attack_level, 1.0);
    }

    #[test]
    fn zero_length_plays_until_stopped() {
        let e = from_raw(&raw(FF_CONSTANT, 0x4000, 0, constant(1000))).unwrap();
        assert_eq!(e.duration, None);
    }

    #[test]
    fn damper_keeps_its_coefficients() {
        let x = libc::ff_condition_effect {
            right_saturation: 0xffff,
            left_saturation: 0x8000,
            right_coeff: 0x4000,
            left_coeff: -0x4000,
            deadband: 0,
            center: 0,
        };
        let zero = libc::ff_condition_effect {
            right_saturation: 0,
            left_saturation: 0,
            right_coeff: 0,
            left_coeff: 0,
            deadband: 0,
            center: 0,
        };
        let e = from_raw(&raw(FF_DAMPER, 0, 0, [x, zero])).unwrap();
        let Kind::Condition {
            kind,
            right_coefficient,
            left_coefficient,
            right_saturation,
            left_saturation,
            ..
        } = e.kind
        else {
            panic!("{:?}", e.kind)
        };
        assert_eq!(kind, Condition::Damper);
        assert!((right_coefficient - 0.5).abs() < 1e-3);
        assert!((left_coefficient + 0.5).abs() < 1e-3);
        assert_eq!(right_saturation, 1.0);
        assert!((left_saturation - 0.5).abs() < 1e-3);
    }

    #[test]
    fn periodic_and_rumble() {
        let sine = libc::ff_periodic_effect {
            waveform: FF_SINE,
            period: 50,
            magnitude: -32768,
            offset: 0,
            phase: 0x8000,
            envelope: libc::ff_envelope {
                attack_length: 0,
                attack_level: 0,
                fade_length: 0,
                fade_level: 0,
            },
            custom_len: 0,
            custom_data: std::ptr::null_mut(),
        };
        let e = from_raw(&raw(FF_PERIODIC, 0x4000, 0, sine)).unwrap();
        let Kind::Periodic {
            wave,
            magnitude,
            period,
            phase,
            ..
        } = e.kind
        else {
            panic!("{:?}", e.kind)
        };
        assert_eq!(wave, Wave::Sine);
        assert!((magnitude + 1.0).abs() < 1e-3);
        assert_eq!(period, Duration::from_millis(50));
        assert_eq!(phase, 0.5);

        let rumble = libc::ff_rumble_effect {
            strong_magnitude: 0xffff,
            weak_magnitude: 0,
        };
        let e = from_raw(&raw(FF_RUMBLE, 0, 0, rumble)).unwrap();
        assert_eq!(
            e.kind,
            Kind::Rumble {
                strong: 1.0,
                weak: 0.0
            }
        );
    }

    #[test]
    fn unknown_types_are_refused() {
        assert_eq!(from_raw(&raw(0x5d, 0, 0, 0u32)), None);
    }
}
