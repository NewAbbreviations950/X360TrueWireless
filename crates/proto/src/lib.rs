//! Xbox 360 Wireless Receiver packet parsing and construction.
//! See `docs/protocol.md` for the format and how it was verified.

/// A packet received from one controller slot of the receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InPacket {
    /// Controller connected to / disconnected from this slot.
    Presence {
        connected: bool,
    },
    Gamepad(GamepadState),
    Chatpad(ChatpadEvent),
    /// Sent on connect (and after a presence inquiry): the battery level and what kind of
    /// device this is, when the packet carries them.
    LinkInfo {
        battery: Option<Battery>,
        device: Option<Device>,
    },
    /// The controller's battery level (from the status packet).
    Battery(Battery),
    /// Reply to [`out::device_request`].
    Device(Device),
    /// Anything we don't understand (yet).
    Other,
}

/// What kind of device is on a slot, from register 5. Its low bits match XInput's subtypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Gamepad,
    Wheel,
    /// Another subtype (register 5 & 0x7f); none has been seen yet.
    Other(u8),
}

impl Device {
    pub fn from_register(value: u8) -> Device {
        match value & 0x7f {
            0x01 => Device::Gamepad,
            0x02 => Device::Wheel,
            v => Device::Other(v),
        }
    }
}

/// Controller battery level. The codes are from `docs/protocol.md` (Battery).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Battery {
    Empty,
    Low,
    Medium,
    Full,
}

impl Battery {
    /// Map the 3-bit battery code (`byte >> 5`). Other codes are not known.
    pub fn from_code(code: u8) -> Option<Battery> {
        match code {
            1 => Some(Battery::Empty),
            3 => Some(Battery::Low),
            5 => Some(Battery::Medium),
            7 => Some(Battery::Full),
            _ => None,
        }
    }

    /// Bars to show, 1 (empty) to 4 (full).
    pub fn bars(self) -> u8 {
        match self {
            Battery::Empty => 1,
            Battery::Low => 2,
            Battery::Medium => 3,
            Battery::Full => 4,
        }
    }

    /// Inverse of [`Battery::bars`]; 0 (or anything else) means unknown.
    pub fn from_bars(bars: u8) -> Option<Battery> {
        match bars {
            1 => Some(Battery::Empty),
            2 => Some(Battery::Low),
            3 => Some(Battery::Medium),
            4 => Some(Battery::Full),
            _ => None,
        }
    }

    /// The pad flashes its light at this level and below.
    pub fn is_low(self) -> bool {
        matches!(self, Battery::Empty | Battery::Low)
    }

    /// Lower-case name for UI text ("full", "medium", "low", "empty").
    pub fn name(self) -> &'static str {
        match self {
            Battery::Full => "full",
            Battery::Medium => "medium",
            Battery::Low => "low",
            Battery::Empty => "empty",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GamepadState {
    /// Bitmask of `button::*`.
    pub buttons: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    /// Up is positive, as the controller reports it.
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

/// Gamepad button bits in [`GamepadState::buttons`] (report byte 6 is the low byte, byte 7 the high byte).
pub mod button {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LS: u16 = 0x0040;
    pub const RS: u16 = 0x0080;
    pub const LB: u16 = 0x0100;
    pub const RB: u16 = 0x0200;
    pub const GUIDE: u16 = 0x0400;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatpadEvent {
    /// The chatpad wants to wake up; the host must answer with [`out::chatpad_wake_ack`].
    WakeRequest,
    /// Chatpad handshake done. `leds` is the indicator state (`led::*` bits); it reads 0
    /// while the chatpad is idle (backlight off), even though the chatpad remembers its LEDs.
    Active {
        leds: u8,
    },
    Keys(ChatpadKeys),
}

/// Full chatpad state: held modifiers plus up to two held keys (raw codes, 0 = none).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChatpadKeys {
    /// Bitmask of `modifier::*`.
    pub modifiers: u8,
    pub keys: [u8; 2],
}

/// Chatpad indicator bits, as reported in [`ChatpadEvent::Active`].
pub mod led {
    pub const PEOPLE: u8 = 0x01;
    pub const GREEN: u8 = 0x08;
    pub const ORANGE: u8 = 0x10;
    pub const CAPS: u8 = 0x20;
    /// Set while the chatpad is awake (backlight on).
    pub const BACKLIGHT: u8 = 0x80;
    /// The indicators the host can switch.
    pub const ALL: [u8; 4] = [CAPS, GREEN, ORANGE, PEOPLE];
}

pub mod modifier {
    pub const SHIFT: u8 = 0x01;
    pub const GREEN: u8 = 0x02;
    pub const ORANGE: u8 = 0x04;
    pub const PEOPLE: u8 = 0x08;
}

pub fn parse(data: &[u8]) -> InPacket {
    match data {
        [0x08, flags, ..] => InPacket::Presence {
            connected: flags & 0x80 != 0,
        },
        [0x00, 0x01, 0x00, 0xf0, ..] if data.len() >= 18 => {
            let i16_at = |i: usize| i16::from_le_bytes([data[i], data[i + 1]]);
            InPacket::Gamepad(GamepadState {
                buttons: u16::from_le_bytes([data[6], data[7]]),
                lt: data[8],
                rt: data[9],
                lx: i16_at(10),
                ly: i16_at(12),
                rx: i16_at(14),
                ry: i16_at(16),
            })
        }
        [0x00, 0x02, 0x00, 0xf0, ..] if data.len() >= 28 => match (data[24], data[25]) {
            (0xf0, 0x03) => InPacket::Chatpad(ChatpadEvent::WakeRequest),
            (0xf0, 0x04) => InPacket::Chatpad(ChatpadEvent::Active { leds: data[26] }),
            (0x00, modifiers) => InPacket::Chatpad(ChatpadEvent::Keys(ChatpadKeys {
                modifiers,
                keys: [data[26], data[27]],
            })),
            _ => InPacket::Other,
        },
        // Link info: register/value pairs from byte 14. Byte 17 is the battery code (register
        // 1); its low bits are `00011`, or `00111` from the racing wheel (bit 2 is ignored, see
        // `docs/protocol.md`). Bytes 24–25 are register 5, the device type.
        [0x00, 0x0f, 0x00, 0xf0, ..] if data.len() >= 18 => InPacket::LinkInfo {
            battery: (data[17] & 0x1b == 0x03)
                .then(|| Battery::from_code(data[17] >> 5))
                .flatten(),
            device: match data.get(24..26) {
                Some(&[0x50, value]) => Some(Device::from_register(value)),
                _ => None,
            },
        },
        // Register 5 read: `00 00 00 50 XX`.
        [0x00, 0x00, 0x00, 0x50, value, ..] => InPacket::Device(Device::from_register(*value)),
        // Status: byte 4 is the battery code; its low bits are `00010`, or `00110` from the wheel.
        [0x00, 0x00, 0x00, 0x13 | 0x17, b, ..] if b & 0x1b == 0x02 => battery(*b),
        _ => InPacket::Other,
    }
}

/// The battery packet for a byte whose top three bits are the code. Unknown codes are `Other`.
fn battery(byte: u8) -> InPacket {
    match Battery::from_code(byte >> 5) {
        Some(level) => InPacket::Battery(level),
        None => InPacket::Other,
    }
}

/// Packets sent to a controller slot.
pub mod out {
    pub type Packet = [u8; 12];

    const fn packet(b0: u8, b1: u8, b2: u8, b3: u8) -> Packet {
        [b0, b1, b2, b3, 0, 0, 0, 0, 0, 0, 0, 0]
    }

    /// Ask whether a controller is connected; answered with a presence packet.
    pub const fn presence_inquiry() -> Packet {
        packet(0x08, 0x00, 0x0f, 0xc0)
    }

    /// Ring-of-light animation, using xpad's LED command numbers (2–5 = flash then player 1–4).
    pub const fn led(command: u8) -> Packet {
        packet(0x00, 0x00, 0x08, 0x40 | (command % 0x0e))
    }

    /// Read the battery register; answered with a status packet (`00 00 00 13 XX`).
    pub const fn battery_request() -> Packet {
        packet(0x00, 0x00, 0x00, 0x40)
    }

    /// Read register 5, the device type; answered with `00 00 00 50 XX`
    /// ([`super::InPacket::Device`]).
    pub const fn device_request() -> Packet {
        packet(0x00, 0x00, 0x01, 0x40)
    }

    pub const fn power_off() -> Packet {
        packet(0x00, 0x00, 0x08, 0xc0)
    }

    pub const fn rumble(strong: u8, weak: u8) -> Packet {
        [0x00, 0x01, 0x0f, 0xc0, 0x00, strong, weak, 0, 0, 0, 0, 0]
    }

    /// Reply to every [`super::ChatpadEvent::WakeRequest`].
    pub const fn chatpad_wake_ack() -> Packet {
        packet(0x00, 0x00, 0x0c, 0x1b)
    }

    /// Switch one chatpad indicator (a `led::*` bit other than BACKLIGHT). While the chatpad
    /// is idle this only wakes it, so it must be re-sent once it reports itself awake.
    pub const fn chatpad_led(led: u8, on: bool) -> Packet {
        let index = match led {
            super::led::CAPS => 0,
            super::led::GREEN => 1,
            super::led::ORANGE => 2,
            _ => 3,
        };
        packet(0x00, 0x00, 0x0c, index | if on { 0x08 } else { 0x00 })
    }

    /// Keepalive, sent about once a second; `phase` alternates the two variants.
    pub const fn chatpad_keepalive(phase: bool) -> Packet {
        packet(0x00, 0x00, 0x0c, if phase { 0x1f } else { 0x1e })
    }
}

macro_rules! chatpad_keys {
    ($($name:ident = $code:literal),* $(,)?) => {
        /// A physical chatpad key (excluding the modifier keys, which are reported as bits).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ChatpadKey { $($name),* }

        impl ChatpadKey {
            pub const ALL: &'static [ChatpadKey] = &[$(ChatpadKey::$name),*];

            pub fn from_code(code: u8) -> Option<Self> {
                match code {
                    $($code => Some(ChatpadKey::$name),)*
                    _ => None,
                }
            }

            pub fn code(self) -> u8 {
                match self { $(ChatpadKey::$name => $code),* }
            }

            pub fn name(self) -> &'static str {
                match self { $(ChatpadKey::$name => stringify!($name)),* }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $(stringify!($name) => Some(ChatpadKey::$name),)*
                    _ => None,
                }
            }
        }
    };
}

chatpad_keys! {
    D1 = 0x17, D2 = 0x16, D3 = 0x15, D4 = 0x14, D5 = 0x13,
    D6 = 0x12, D7 = 0x11, D8 = 0x67, D9 = 0x66, D0 = 0x65,
    Q = 0x27, W = 0x26, E = 0x25, R = 0x24, T = 0x23,
    Y = 0x22, U = 0x21, I = 0x76, O = 0x75, P = 0x64,
    A = 0x37, S = 0x36, D = 0x35, F = 0x34, G = 0x33,
    H = 0x32, J = 0x31, K = 0x77, L = 0x72, Comma = 0x62,
    Z = 0x46, X = 0x45, C = 0x44, V = 0x43, B = 0x42,
    N = 0x41, M = 0x52, Period = 0x53, Enter = 0x63,
    Left = 0x55, Space = 0x54, Right = 0x51, Backspace = 0x71,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    // Packets below are copied from docs/captures/chatpad-keys.log.

    #[test]
    fn presence() {
        assert_eq!(parse(&hex("08 80")), InPacket::Presence { connected: true });
        assert_eq!(
            parse(&hex("08 00")),
            InPacket::Presence { connected: false }
        );
    }

    #[test]
    fn gamepad_idle_with_stick_drift() {
        let p = parse(&hex(
            "00 01 00 f0 00 13 00 00 00 00 41 fa 72 ea 5a 07 3b 1a 00 00 00 00 00 00 00 00 00 00 00",
        ));
        assert_eq!(
            p,
            InPacket::Gamepad(GamepadState {
                buttons: 0,
                lt: 0,
                rt: 0,
                lx: -1471,
                ly: -5518,
                rx: 1882,
                ry: 6715
            })
        );
    }

    #[test]
    fn chatpad_status() {
        let wake = hex(
            "00 02 00 f0 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 f0 03 00 0d 0d",
        );
        let active = hex(
            "00 02 00 f0 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 f0 04 00 0e 0e",
        );
        assert_eq!(parse(&wake), InPacket::Chatpad(ChatpadEvent::WakeRequest));
        assert_eq!(
            parse(&active),
            InPacket::Chatpad(ChatpadEvent::Active { leds: 0 })
        );
        // From docs/captures/chatpad-leds.log after `0c 08`: caps + backlight.
        let caps = hex(
            "00 02 00 f0 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 f0 04 a0 09 09",
        );
        assert_eq!(
            parse(&caps),
            InPacket::Chatpad(ChatpadEvent::Active {
                leds: led::CAPS | led::BACKLIGHT
            })
        );
    }

    #[test]
    fn led_commands() {
        assert_eq!(out::chatpad_led(led::CAPS, true)[3], 0x08);
        assert_eq!(out::chatpad_led(led::GREEN, true)[3], 0x09);
        assert_eq!(out::chatpad_led(led::ORANGE, true)[3], 0x0a);
        assert_eq!(out::chatpad_led(led::PEOPLE, true)[3], 0x0b);
        assert_eq!(out::chatpad_led(led::GREEN, false)[3], 0x01);
    }

    #[test]
    fn chatpad_keys() {
        let shift_a = hex(
            "00 02 00 f0 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 37 00 00",
        );
        assert_eq!(
            parse(&shift_a),
            InPacket::Chatpad(ChatpadEvent::Keys(ChatpadKeys {
                modifiers: modifier::SHIFT,
                keys: [0x37, 0]
            }))
        );
        let q_w = hex(
            "00 02 00 f0 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 26 27 00",
        );
        assert_eq!(
            parse(&q_w),
            InPacket::Chatpad(ChatpadEvent::Keys(ChatpadKeys {
                modifiers: 0,
                keys: [0x26, 0x27]
            }))
        );
    }

    #[test]
    fn verified_key_codes() {
        for (code, key) in [
            (0x27, ChatpadKey::Q),
            (0x26, ChatpadKey::W),
            (0x25, ChatpadKey::E),
            (0x17, ChatpadKey::D1),
            (0x37, ChatpadKey::A),
            (0x63, ChatpadKey::Enter),
            (0x71, ChatpadKey::Backspace),
            (0x54, ChatpadKey::Space),
        ] {
            assert_eq!(ChatpadKey::from_code(code), Some(key));
        }
    }

    #[test]
    fn key_table_roundtrips() {
        for &k in ChatpadKey::ALL {
            assert_eq!(ChatpadKey::from_code(k.code()), Some(k));
            assert_eq!(ChatpadKey::from_name(k.name()), Some(k));
        }
        assert_eq!(ChatpadKey::from_code(0), None);
    }

    fn link(byte17: &str) -> Vec<u8> {
        hex(&format!(
            "00 0f 00 f0 00 00 00 00 00 00 00 00 00 00 00 00 13 {byte17} 20 1d"
        ))
    }

    fn linked(battery: Option<Battery>) -> InPacket {
        InPacket::LinkInfo {
            battery,
            device: None,
        }
    }

    #[test]
    fn device_type() {
        // From captures/wheel-probe-2026-10-18.log and docs/captures/chatpad-keys.log.
        let wheel = hex(
            "00 0f 00 f0 f0 cc e1 47 40 70 fb d8 76 10 00 03 13 e7 20 1d 30 03 40 01 50 82 ff ff ff",
        );
        let pad = hex(
            "00 0f 00 f0 00 cc e1 47 40 70 ef 87 54 30 00 20 13 e3 20 1d 30 03 40 01 50 01 ff ff ff",
        );
        assert_eq!(
            parse(&wheel),
            InPacket::LinkInfo {
                battery: Some(Battery::Full),
                device: Some(Device::Wheel)
            }
        );
        assert_eq!(
            parse(&pad),
            InPacket::LinkInfo {
                battery: Some(Battery::Full),
                device: Some(Device::Gamepad)
            }
        );
        assert_eq!(
            parse(&hex("00 00 00 50 82 00 00")),
            InPacket::Device(Device::Wheel)
        );
    }

    fn status(byte4: &str) -> Vec<u8> {
        hex(&format!("00 00 00 13 {byte4} 00 00 00"))
    }

    // Battery bytes are from docs/captures/battery-sweep-2026-10-10.log.
    #[test]
    fn battery_codes() {
        assert_eq!(parse(&link("e3")), linked(Some(Battery::Full)));
        assert_eq!(parse(&link("a3")), linked(Some(Battery::Medium)));
        assert_eq!(parse(&link("63")), linked(Some(Battery::Low)));
        assert_eq!(parse(&link("23")), linked(Some(Battery::Empty)));

        assert_eq!(parse(&status("e2")), InPacket::Battery(Battery::Full));
        assert_eq!(parse(&status("a2")), InPacket::Battery(Battery::Medium));
        assert_eq!(parse(&status("62")), InPacket::Battery(Battery::Low));
        assert_eq!(parse(&status("22")), InPacket::Battery(Battery::Empty));
        // The second status header, `00 00 00 17`, carries the same byte.
        assert_eq!(
            parse(&hex("00 00 00 17 62 00 00 00")),
            InPacket::Battery(Battery::Low)
        );
        // The racing wheel sets bit 2, on AC or batteries (captures/wheel-probe-2026-10-14.log).
        assert_eq!(parse(&link("e7")), linked(Some(Battery::Full)));
        assert_eq!(parse(&status("e6")), InPacket::Battery(Battery::Full));
    }

    #[test]
    fn battery_rejects_unknown_bytes() {
        // Low five bits don't match the packet type (the unexplained `c0` and `3c` values).
        assert_eq!(parse(&link("c0")), linked(None));
        assert_eq!(parse(&link("3c")), linked(None));
        assert_eq!(parse(&status("c0")), InPacket::Other);
        assert_eq!(parse(&status("3c")), InPacket::Other);
        // Right low bits, but code 0 isn't a known level.
        assert_eq!(parse(&status("02")), InPacket::Other);
        // Same first bytes as the status packet, but a different packet.
        assert_eq!(parse(&hex("00 00 00 f0 00 00 00 00")), InPacket::Other);
        // Too short to hold the byte.
        assert_eq!(parse(&hex("00 00 00 13")), InPacket::Other);
    }

    #[test]
    fn battery_bars_roundtrip() {
        for level in [Battery::Empty, Battery::Low, Battery::Medium, Battery::Full] {
            assert_eq!(Battery::from_bars(level.bars()), Some(level));
        }
        assert_eq!(Battery::from_bars(0), None);
        assert!(Battery::Low.is_low() && Battery::Empty.is_low());
        assert!(!Battery::Medium.is_low() && !Battery::Full.is_low());
    }
}
