# Xbox 360 Wireless Receiver protocol notes

Verified against a real receiver (045e:0719) with `x360chatpad-dump`; raw logs are in `captures/`.
Items marked *(xpad)* come from the kernel `xpad` driver and are not independently re-verified yet.

## USB layout
- 8 interfaces, all interrupt endpoints. Controller slot N (0–3):
  - interface `2N` (protocol 0x81 / 129): gamepad + chatpad data, EP IN `0x81+2N`, EP OUT `0x01+2N`.
  - interface `2N+1` (protocol 0x82 / 130): headset audio, EP IN `0x82+2N`, EP OUT `0x02+2N` (format unknown).
- IN packets are 29 bytes except presence packets (2 bytes).

## Host → controller (OUT, 12 bytes, zero padded)
| Packet | Meaning |
|---|---|
| `08 00 0f c0 00…` | presence inquiry; receiver answers with a presence packet *(xpad)* |
| `00 00 08 4X 00…` | ring LED, `X` = xpad LED command (`6` = player 1 … `9` = player 4) *(xpad)* |
| `00 00 08 c0 00…` | power the controller off |
| `00 01 0f c0 00 SS WW 00…` | rumble, SS = strong (left) motor, WW = weak (right) motor *(xpad)* |
| `00 00 0c 1b 00…` | chatpad: answer to a wake request (`f0 03`) — **send once per wake packet** |
| `00 00 0c 1e` / `00 00 0c 1f` | chatpad keepalive, alternated about once per second |
| `00 00 0c 08`–`0b` | chatpad indicator **on**: Caps, Green, Orange, People |
| `00 00 0c 00`–`03` | chatpad indicator **off**: Caps, Green, Orange, People |

## Controller → host (IN)
### Presence (2 bytes)
`08 80` = controller connected on this slot, `08 00` = slot empty.

### Link info
`00 0f 00 f0 …` sent after connect. Bytes 7–13 look like the controller serial; byte 17 carries the battery code (see *Battery*).

### Gamepad (`00 01 00 f0 …`)
Bytes from offset 4 match the wired 360 report *(xpad)*:
| Byte | Content |
|---|---|
| 6 | dpad up `01`, down `02`, left `04`, right `08`, start `10`, back `20`, LS `40`, RS `80` |
| 7 | LB `01`, RB `02`, guide `04`, A `10`, B `20`, X `40`, Y `80` |
| 8, 9 | LT, RT (0–255) |
| 10–17 | LX, LY, RX, RY as little-endian i16 (Y up = positive) |

### Chatpad (`00 02 00 f0 …`), payload at bytes 24–28
| Bytes 24–25 | Meaning |
|---|---|
| `f0 03` | chatpad wake request → host must reply `0c 1b` (seen ~4× in a burst, answer each) |
| `f0 04` | chatpad active (sent alongside keepalives); byte 26 = indicator state, see below |
| `00 MM` | key report: `MM` = modifier bits, byte 26 = key 1, byte 27 = key 2 (`00` = none) |

Byte 27/28 of status packets is a 4-bit counter that advances with each keepalive.
Without the `1b` handshake the chatpad stays in `f0 03`, its backlight stays off, and no keys are reported.

Modifier bits (byte 25): Shift `01`, Green `02`, Orange `04`, People `08`. They are reported while held.

Key codes (byte 26/27). ✔ = seen in a capture; the rest come from the wired-chatpad table in xboxdrv, which matched every key we verified.
```
1 17✔  2 16  3 15  4 14  5 13  6 12  7 11  8 67  9 66  0 65
Q 27✔  W 26✔ E 25✔ R 24  T 23  Y 22  U 21  I 76  O 75  P 64
A 37✔  S 36  D 35  F 34  G 33  H 32  J 31  K 77  L 72  , 62
Z 46   X 45  C 44  V 43  B 42  N 41  M 52  . 53  Enter 63✔
Left 55  Space 54✔  Right 51  Backspace 71✔
```

### Chatpad indicators
Byte 26 of `f0 04` status packets: Caps `20`, Green `08`, Orange `10`, People `01`,
backlight/awake `80`.
- About 11 s after the last activity, the chatpad goes idle: backlight off, and byte 26 reads `00`.
  It **remembers** the indicators; a keypress wakes it and they reappear.
- An indicator command sent while idle only wakes the chatpad. The chatpad sends `f0 03`,
  we answer `1b`, and the command itself is dropped. Re-send it once byte 26 has `80` set.

### Battery
Two packets carry a battery code. The top three bits of the byte are the code (`code = byte >> 5`); the low five bits stay fixed for each packet type, so the codes are easy to read off.

| Packet | Byte | Fixed low bits | When it's sent |
|---|---|---|---|
| Link info `00 0f 00 f0 …` | 17 | `00011` (wheel: `00111`) | on connect (a presence inquiry gets it back, but the value doesn't change while connected) |
| Status `00 00 00 13 XX` (and `00 00 00 17 XX`) | 4 | `00010` (wheel: `00110`) | sporadically while connected |

Observed codes, from resets at each voltage (sweep 2, 3.0 V down to 2.1 V):

| Code | Byte values | Voltage | Flashing | Bars (shown in the popup) |
|---|---|---|---|---|
| 7 | `e3` (link), `e2` (status) | 2.7–3.0 V | no | full |
| 5 | `a3`, `a2` | 2.38–2.6 V | no | medium |
| 3 | `63`, `62` | 2.31–2.37 V | yes | low |
| 1 | `23`, `22` | 2.1–2.31 V | yes | empty |

- The code boundaries are about 2.37–2.38 V (5 → 3, the low-battery flash starts), about 2.6–2.7 V (7 → 5), and about 2.30–2.31 V (3 → 1). At 2.31 V the two status packets were split between codes 1 and 3, so the boundary is noisy.
- The low-battery flash rate changes between 2.1 and 2.3 V while the code stays at 1. Flash rate isn't in these packets, so it can't be read from the protocol.
- The first sweep (falling, no resets) gave values that don't fit this table: `c0` at 2.35 V, `3c` at 2.4–2.45 V, and code 1 at 2.4 V. Pad history or hysteresis probably explains them, but that isn't shown yet.
- Codes are only refreshed on connect (link) or sporadically (status), so a live bar would lag. A 4-bar display should treat the link byte as a connect-time snapshot.
- `00 00 00 40` reads the battery register on demand, and controllers answer with the status packet too (see the register reads under Racing wheel). The daemon sends it every 5 minutes. Whether the answer tracks the voltage live, or repeats the connect-time value, hasn't been tested yet.
- The racing wheel sends `e7` (link) and `e6` (status): code 7 with bit 2 set. It's set on AC with the batteries out and on batteries alone, so it isn't a power-source flag. It may just mark the wheel. The parser ignores it.
- Not yet explained: `c0` and `3c` (low bits don't match the table), and the exact full/medium boundary between 2.6 and 2.7 V.
- SDL's HIDAPI Xbox 360 wireless driver reads these same bytes (link byte 17, status byte 4) and sends no battery request. It maps the value to four buckets, but its 0–255 thresholds don't match the codes we see. [SDL source](https://github.com/libsdl-org/SDL/blob/a8780c6a287bb97549ec4411cff7ade560db441f/src/joystick/hidapi/SDL_hidapi_xbox360w.c)

### Racing wheel (Xbox 360 Wireless Racing Wheel, WRW01)
Captured in `captures/wheel-probe-2026-10-14.log` (on AC with the batteries out, then on batteries):
- Link info matches a controller's except byte 25: `82` for the wheel, `01` for a controller.
  The low bits match XInput's wheel subtype (`02`); what the `80` bit means isn't known.
- The battery byte has bit 2 set (see Battery above).
- Gamepad reports use the controller layout: steering is LX (bytes 10–11, about −32560 to
  +32648 lock to lock), the brake is LT (byte 8), the accelerator RT (byte 9), both 0–255.
  The paddles are LB/RB, and the other buttons use the usual bits.
- The wheel sent `00 f8 02` once after connecting (see Unknown).
- The rumble packet `00 01 0f c0 00 LL RR` reaches two rumble motors in the wheel, but the
  bytes are swapped compared with a controller: `LL` drives a small, high-pitched motor and
  `RR` a big, deep one. 25% (`40`) on one motor alone isn't felt; 50% is. Rumble never
  turned or stiffened the rim (steering stayed at +0 with a hand resting on it), so the
  steering motor needs other packets.
- On AC, the motor runs a default autocentre with no packets from the host: the rim springs
  back to centre, and plugging the AC in re-centres it. On batteries the rim stays wherever
  it's left.
- The wheel sends `00 05 00 …` packets that controllers don't (the `00 05` header is new):
  - Plugging the AC in: `00 05 00 80 f1 10 7f 28 7f 0c 12 02`, repeated about every 200 ms as
    `f1`, `71`, `f1`, `71`, then `f5`. Byte 4's top bit flips on each repeat, like the patent's
    toggle bit, so these may be reports waiting for an acknowledgement we never send.
  - When the dump tool took over from the daemon: `00 05 00 f0 00 12 3f ff ff ff ff ff … ff ff
    07 01` and `00 05 00 80 71 10 7f 28 7f 0c 12 02` in pairs, every 2 s, four times.
  - Unplugging the AC sent `00 00 00 80 00…`, then the status packet.
  - The LED command (`08 42`) and `08 50` each make the wheel send the pair
    `00 05 00 f0 00 12 3f ff ff ff ff ff 00… ff ff 07 01` + `00 05 00 80 [71|f1] 10 7f 28 7f 0c
    12 02` four times, 2 s apart, starting within about 3 s. Controllers never send it
    (earlier captures have LED commands and no `00 05`). It looks like a state or capability
    report after an LED change; `08 5x` may be more LED patterns. During a sweep the probe pins
    it on a later step, so time it with a single send.
  - In the first class-8 sweep, while the wheel was unassigned (all four lights flashing),
    the ring went dark around the time of that report. With the wheel showing player 1, the
    same sweep changed nothing visible.
- Classes 0–2 are **register reads**. The wheel ignores the low 6 bits of byte 3; the
  register number is (byte 2 & 3) << 2 | byte 3 >> 6, and the reply is `00 00 00 [reg]X YY`.
  These are the same pairs the link-info packet carries in bytes 14–25.

  | Command | Reply | Register |
  |---|---|---|
  | `00 00 00 00` | `00 00 00 00 03` | 0 |
  | `00 00 00 40` | `00 00 00 13 e6` | 1, battery (the "status" packet) |
  | `00 00 00 80` | `00 00 00 20 1d` | 2 |
  | `00 00 00 c0` | `00 00 00 30 03` | 3 |
  | `00 00 01 00` | `00 00 00 40 01` | 4 |
  | `00 00 01 40` | `00 00 00 50 82` | 5, device type (`01` on a controller) |
  | `00 00 02 00` | `00 00 00 80 71` / `f1` (toggles) | 8 |
  | `00 00 02 40` | `00 05 00 f0 00 11 02 00…` | 9 |
  | `00 00 02 80` | `00 05 00 f0 00 12 3f ff ff ff ff ff … ff ff 07 01` | 10 |

  Registers 6, 7 and 11–15 (`01 80`, `01 c0`, `02 c0`, `03 xx`) don't answer, and classes 4–7
  draw no reply. None of classes 0–7 moved the rim (2048 commands, weight on, wheel assigned).
- Class `0b` (`00 00 0b xx`) **controls the steering motor**. Found by hanging a weight on the
  rim so the autocentre held it off-centre; the rim swings toward the weight when the motor
  lets go. Register 8 echoes the state after writes below `40` (bit 7 toggles each time).
  - `0b 02` motor off (reg 8 `f1` → `d1`, bit `20` cleared); `0b 04` motor on (bit `20`
    set). Bit 3 of the write doesn't matter (`0a`/`0c` do the same).
  - `0b 10` motor off too, clearing bit `40` instead; `0b 20` sets it again. `0b 05` sets
    bit `02` (`f3`); its meaning isn't known. The motor runs only with bits `40` and `20`
    both set. The pattern isn't fully decoded: `0b 14` turned the motor on although the
    next reply (`b3`) showed bit `40` clear.
  - `0b 40 | n` (n = 0–31) sets a strength, probably the autocentre spring's: `40`–`43`
    limp, rising from `44`; `5f` nearly matches the default. Holding values for 5 s each
    (`40`, `5f`, `48`, `54`, `40`) gave settled positions of −28700, −15300, −22400,
    −22400, −26100, so the rim moved back toward the weight when the value dropped. Rim
    friction hides small steps (`48` → `54` didn't move it).
  - `0b 60 | n`: also strength-like (limp at `60`–`67`, rising from `68`). Its difference
    from `4x` isn't known.
  - `0b 80`–`ff`: no visible change after `82`; saturated or ignored.
  - No class-`0b` write pushed the rim sideways. Writes `40` and up get no reply.
  - In the full classes 9–f sweep, the wheel stopped sending input reports at some point in
    classes 9 or `0a` (the motor still toggled during `0b`). Register reads still worked; a
    power cycle restored it. Which write did it is unknown.
- Payload packets (`docs/probes/04-payloads.txt`, motor on, weight on): no movement. Some
  reset the wheel, which disconnects and reconnects within about 2 s with its LED cleared:
  about `00 07 0f c0` with payload A, `00 09 0f c0` with payload B, and `00 06 0f c0` with
  payload C (the probe's attribution can lag a step). The reset depends on the payload, so
  the wheel parses these packets.
- `00 01 08 c0` **powers the wheel off**, just like `00 00 08 c0`. The probe parser now
  refuses any `00 xx 08 c0`.
- Class 8 (`00 00 08 xx`, all values except `4x` and `c0`) has no force-feedback command:
  with the wheel assigned and a weight pulling the rim off-centre, none of them moved it or
  changed the autocentre.
- The wheel can rejoin on a different slot after a power cycle (it moved from slot 1 to slot
  0 when slot 0 was free). Check the `08 80` connect line before sending to a slot.

Microsoft's patent US 8,117,364 names this wheel and describes its
force-feedback protocol in general terms:
- The host uploads effect blocks and parameter blocks into the wheel's memory, then sends
  short start/stop, gain and device-control commands. The wheel plays effects by itself.
- Effect types: constant, ramp, five periodic waves, spring, damper, inertia, friction.
- An effect block is 8 one-byte fields: report type, index, block control, effect type,
  duration, gain, start delay, loop count. A toggle bit (report-type MSB) flips on every
  message.
- Short commands fit in 6–12 header bits; the rest use an 8-byte payload.

The byte values aren't given. `docs/probes/` has the scripts used to look for them.

### Unknown
- `00 f8 0N 00…` (N = 1–3): seen only while the chatpad was not handshaked and keys were being pressed. Possibly "key pressed while asleep" or an attachment status.
- `00 f8 01` / `00 f8 02` alternate every few seconds while connected; meaning unknown.
- `00 00 00 f0` (every few seconds while connected) and `00 00 00 20/30/40/60` (a burst after connect); meaning unknown.
