# Xbox 360 Wireless Receiver protocol notes

Verified against a real receiver (045e:0719) with `x360chatpad-dump`; raw logs are in `captures/` (large ones xz-compressed).
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
    from `4x` isn't known; `7f` felt the same as `5f`.
  - `0b 80`–`ff` are ignored: `0b ff` sent on top of a weak `0b 46` left it weak (the rim
    stopped at the same ±5000 short of centre).
  - The spring has no centre offset: after `5f`, `7f`, `80` and `c0`, the rim returned to
    within ±200 of centre from both locks (2026-10-16, weight off).
  - No class-`0b` write pushed the rim sideways. Writes `40` and up get no reply.
  - `0b` payload packets (`00 01 0b xx` + payload) act like the header-only command; the
    payload is ignored.
- `00 00 09 41` **stops all input reports** (also the periodic `00 00 00 f0`) until a power
  cycle; `09 40` doesn't. The motor and register reads keep working, and buttons pressed in
  this state draw `00 f8 01`. No other class-9 write undoes it, and neither does a `0f 81`
  reset.
- `00 00 0f 81`–`bf` (sampled: `81`–`84`, `88`, `90`, `a0`, `b0`) **reset the wheel**: it
  drops within about 1 s and rejoins 1–3 s later with its LED cleared. `0f 80` and
  `0f c0`–`ff` don't. Header-only classes `0d`–`0f` otherwise do nothing visible.
- Payload type `03` (`00 03 0f c0` + any payload) also resets the wheel, about 2 s after it's
  sent. That delay put the probe's blame on later steps: the 2026-10-14 payload resets
  attributed to types `06`, `07` and `09` were type `03`. Types `02`–`1f` otherwise do
  nothing visible on `0f c0`.
- After a reset the wheel rejoins without the host's settings: set the LED and the motor
  (`0b 04`, strength) again.
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

#### Force-feedback reports (found 2026-10-16)
The rumble packet's first payload byte selects a report, and the packets follow the patent's
effect-block layout. Sent as `00 01 0f c0` + 8 bytes, the payload is
`kk ii bb tt …`, where `kk` is the report type:
- `kk` = `00` is rumble (`00 LL RR …`). The wheel doesn't acknowledge it.
- Bit 7 of `kk` is ignored (`81`–`90` act like `01`–`10`); it's probably the patent's toggle
  bit. The wheel ignores a packet identical to the one before it (no second reply), so it
  treats repeats as retransmissions.
- `kk` = `02`–`05`, `07`–`09`, `0b`, `0c` are **acknowledged** with a register-style reply
  `00 00 00 9S CC` (`aS CC` for `0b`). `01`, `06`, `0a` and `0d`–`7f` get no reply.
  - `CC` is a checksum of the payload: it's linear (each payload bit flips a fixed set of
    `CC` bits, the same for every `kk`; only the base value depends on `kk`) and covers all
    7 bytes after `kk`. Same packet, same reply.
  - `S` is a status. Seen: `0` accepted (create, operation), `1` accepted parameter block,
    `2` after `0c`, `4` rejected, `5` rejected parameter block, `8` effect memory full.
  - For `0b`, `S` is `0`/`8` and `CC` mostly `c0`: it doesn't follow this pattern.
- The wheel also reports register 8 when an effect is created: bit `10` clears (`f1` →
  `e1`), and most `0b` writes set it again. Bit `08` is set while the memory is full.

| `kk` | Meaning (best guess) | Accepted when |
|---|---|---|
| `02` | create effect: `02 ii bb tt dd gg ss ll` (index, block control, type, duration, gain, start delay, loops) | `ii` ≠ 0, `bb` = `05`/`06`/`07` (bit 3 ignored), `tt` ≠ `02` |
| `03` | envelope parameters | the effect at `ii` has type `01` or `03`–`07` |
| `04` | condition parameters | type `08`–`0b` |
| `07` | periodic parameters | type `03`–`07` |
| `08` | constant-force parameters | type `01` |
| `09` | effect operation (start/stop?) | `ii` = `01`–`7f` |
| `0c` | free effect? | `ii` ≠ 0 |
| `05` | ramp parameters? never accepted | (type `02`, the likely ramp, is rejected at create) |

The parameter-block mapping matches the patent's effect types: `01` constant, `03`–`07` the
five periodic waves, `08`–`0b` spring/damper/inertia/friction. Types `0c`–`6d` are also
accepted at create but take no parameter block.

The effect memory fills after roughly 55–110 creates even with `0c` in between; a reset
(`0f 81`) clears it.

**Nothing has moved the rim yet.** Tried with the weight on and the spring at `48` or `5f`:
create type `01` (`ff ff 00 01` for duration, gain, delay, loops) with every block control,
constant parameters `7f`/`ff`/`80` in every byte, envelopes of all `00`/`7f`/`ff`, in both
orders, then `09 01 01 01` or `09 01 02 01`. With that effect loaded, every header-only
command in classes `0a`, `0d`, `0e` and `0b 00`–`3f` was also tried as a start; only the
known motor-off commands moved the rim. The fields of `09` and the units of the effect
block are the next things to map.

Unattended search on 2026-10-17 (weight on, spring `48`, 6.5 hours, nothing moved):
- Host conditions: create types `08`–`0b` (block `05`, `ff ff 00 01`), condition blocks
  (`04 01` + all `7f`, `ff`, `40`, `80`, or `7f`/`00` alternating), then `09 01 01 01`.
- Every pair of `09` bytes 2 and 3 (`09 01 XX YY 00 00 00 00`, all 65,536), with a type-`01`
  effect loaded (`08 01` + `7f`×6), one reset per 256. All were acknowledged with status `0`
  and none was rejected, so `09` doesn't validate these bytes. One reply came back as `9d`,
  but a resend of the same packet gave `90` three times, so it was a radio glitch.
- `kk` = `0b` (all bytes 1–7 swept, 2026-10-17): the status alternates `a0`/`a8` with every
  packet, whatever it holds, so bit `8` is a sequence toggle. The checksum is `c0` unless
  byte 1 has bit 0 or 1 set (then `df`); bytes 2–7 never change the reply. So only
  `0b xx & 3` registers, which fits a small command field (a device control?). Every `0b`
  value was then tried as a start, with no movement (below).
- Movement tests on 2026-10-17 evening (weight on, spring `48`, reset before each, all
  acknowledged, none moved): every `0b xx` value sent before the create, between the
  constant block and `09`, or after `09`; effect indices `02`–`7f` (sampled) with block control
  `05`/`06`/`07`; duration × gain × loops (`00`–`ff` × `7f`/`ff` × `00`/`01`/`ff`); the five
  periodic types with periodic and envelope blocks; constant force with envelopes.
- Unacknowledged report types as a start (constant effect loaded, none moved, no replies):
  `kk` = `01`, `06`, `0a`, `0d`–`7f` as `kk 01 XX 01` (`XX` = `01`/`02`/`ff`), and
  `01`/`06`/`09`/`0a` on payload types `02`, `04`–`1f` (`00 TT 0f c0 kk 01 01 01`).
- Motor mode first (2026-10-17): after each `00 00 0b 00`–`3f`, a constant force (`08 01` +
  `7f`×6 or `80`×6) started with `09 01 01 01` never moved the rim, not even in the modes
  that leave the motor limp with the weight resting on the stop.
- Only 12 bytes count: `0f c0` packets of 13–32 bytes are acknowledged with exactly the
  checksum of their first 12 bytes, so the wheel (or receiver) drops the rest.
- Only `00 01 0f c0` reaches the effect channel: `09` sent with byte 1 = `00`, `02`, `04`–`ff`
  or byte 0 = `01`–`ff` gets no reply.
- No register writes in classes `03`–`07`, `0a`, `0d`, `0e`: none of `00 00 CC 00`–`ff`, nor
  `00 01 0C XX` + payload for classes `03`–`07`, changed registers 0–5 or 8 (2026-10-17).
  Registers 6, 7 and 9–11 don't answer a read.

Reading the patent again (US 8,117,364) gives likely meanings for what's mapped so far:
- Upstream, the Device State, Block Load and Effect Operation reports each fit in 16 header
  bits (4-bit identifier, 11 information bits, toggle bit). `9S CC` would be the **Block
  Load report** (status + checksum), sent after every stored block. `09` draws one too, so
  the wheel stores `09` as a block rather than acting on it. `kk` = `0b` is the only report
  type answered with another identifier (`a`), so it's the best candidate for the **Effect
  Operation command** (an action per effect: no change/stop/start, plus index fields).
- Register 8 fits the **Device State report**: `80` toggle, `40`/`20` native-effect and motor
  enable, `10` block pool empty, `08` pool full.
- Downstream, Device Control and Device Gain are header-only (a 6-bit identifier, 6 control
  bits, a toggle). That matches `00 00 0b xx`: `0b 40`–`7f` would be the **device gain**
  (it scales the autocentre, which is the patent's "native effect"), and `0b 00`–`3f`
  **device control** (enable/disable motors, stop effects, reset, pause/continue the play
  list, start/stop the native effect).
- Tested on that reading (2026-10-18 night, weight on, none moved): a constant force at
  index 1 followed by `0b AA II 00 00 00 00 00` for every `AA` with `II` = `01`, `02`, `80`,
  and `0b AA ff ff ff ff ff ff`; `01` in each of bytes 3–7 with 18 actions; the same with the
  autocentre stopped first (`0b 10`); and constant-force or create bytes set to `40`/`c0` (a
  possible direction). The `0b` reply is `c0` when `AA & 3` = 0 and `df` otherwise, except
  `AA & 0c` ≠ 0 with all index bytes `ff`, which gives `c1`. So the reply does read the
  index bytes, but none of these combinations played the effect.
- **Device control decoded in part** (2026-10-18): after each `00 00 0b 00`–`3f`, a load and
  `0b 01`/`02`/`03 01 …`, the `0b` reply turned from `df` to `ff` (bit `20`) exactly when the
  device control's low 3 bits were 5 (`05`, `0d`, `15` … `3d`). In the patent's order of
  standard control values (0 no change, 1 enable motors, 2 disable motors, 3 stop effects,
  4 reset device, 5 pause play list, 6 continue play list) 5 is "pause". That fits what was
  already known: `0b 02` disables the motor and `0b 04` (reset, which "enables all motors")
  turns it on. So the low 3 bits are the standard control, bits 3–5 hold the native control
  (`10` stops the autocentre, `20` starts it), and bit `20` of the `0b` reply is "play list
  paused".
- **`kk` = `0b` structure** (reply map with an effect loaded at index 1, 2026-10-18):
  `0b AA xx I1 I2 I3 xx xx`. `AA` holds 2-bit actions: bits 2–3 act on the index in byte 3,
  bits 4–5 on byte 4, bits 6–7 on byte 5, and reply bits `01`/`02`/`04` report on each in
  turn. Bits 0–1 of `AA` always draw `df` (an error, or a different operation), and bytes
  2, 6 and 7 never change the reply. With index `01`, action 2 (`08`/`20`/`80`) sets that
  slot's reply bit and action 1 doesn't (`0c` also does for the first slot), so 2 is
  probably "start". The bit doesn't show that the effect plays, though: index `7f` (nothing
  loaded) answers the same, and indices with bit 7 set also set it for action 1.
- Started with `0b 08 00 01` (2026-10-18 night, weight on, none moved): constant blocks with
  one magnitude byte (`7f`/`80`/`ff`/`40`/`c0`) in each position and the rest zero, or a
  16-bit value (`ff 7f`, `01 80`, `00 7f`…) at bytes 2/4/6, for block controls `05`–`07` and
  durations `ff`/`00`; and device gain `0b 40`–`7f` (8 steps) × effect gain `ff`/`7f`/`40` ×
  three magnitudes × two durations, with the gain set in a step of its own.
- Also none moved (2026-10-18 early morning): each device control `0b 00`–`3f` set before
  a constant force (`08 01` + `ff 7f 00…` or `7f`×6) started with `0b 08 00 01`; and every
  action byte `0b AA 00 01 00 00 00 00` and `0b AA 00 01 01 01 00 00` after the load. The
  replies matched the slot map above: `c1` when the first slot's action is 2 or 3, `c2`/`c4`
  for the second and third slots, combined (`c3`…`c7`) when several index bytes are `01`.
- **The start reply shows whether the effect is complete** (2026-10-18 morning). After
  `0b 08 00 01`, the reply is `c1` for an index with nothing loaded, a create alone, or a
  constant/periodic effect with only its own parameter block. It's `80` (slot bit and bit
  `40` clear) for a condition effect with a condition block (any contents, all 72 single-byte
  variants tried), and for a constant or periodic effect with **both** its parameter block and
  an envelope (`03`). With a constant at index 1 and a spring at 2, `0b 28 00 01 02` replies
  `81`. So the slot bits look like per-slot errors ("not ready"), and the constant force
  needs an envelope block. All the constant-force movement tests with `0b 08` before this had
  no envelope. Nothing moved with the complete effects in this first check either (host
  springs with the autocentre on, constant/periodic with envelopes).
- **Force found (2026-10-18, 06:40).** A complete effect started with `0b 08 00 01` drives
  the motor:
  ```
  00 01 0f c0 02 01 05 01 ff ff 00 01   create constant force at index 1
  00 01 0f c0 08 01 MM 00 00 00 00 00   constant parameters: MM = signed force
  00 01 0f c0 03 01 00 00 00 00 00 00   envelope (required, all zero works)
  00 01 0f c0 0b 08 00 01 00 00 00 00   start index 1
  ```
  `MM` is a signed 8-bit force: positive turns the rim right (steering +), negative left.
  With the autocentre off and the weight on, `10` nudged it, `20` swept it slowly to the right
  lock, `40`–`7f` snapped it there in about a second; `c0`–`f0` drove it to the left lock,
  `01` and `ff` did nothing visible. Bytes 3–7 of the constant block don't move it by
  themselves. With the autocentre on, `80` still drives it from its rest point to the left
  lock in about 1.2 s. Periodic waves (`07` + `03`) with the autocentre off swing the rim
  lock to lock, and some springs (`04`) moved it as well. Earlier tests never moved it
  because the effects had no envelope block, or were started with `09`.
- Constant-force details (traced with the autocentre on, so the spring brings the rim back
  when the force ends):
  - **Duration** (create byte 4) is in about 10 ms units: `32` pushed for about 0.6 s, `64`
    about 1.5 s, `ff` about 2.7 s; `0a` was a short tap, `01` too short to see. **`00` plays
    until stopped.**
  - **Gain** (create byte 5) scales the force: with `7f` force, gain `20` barely moved the rim,
    `7f` gave the full push.
  - **Live update:** sending a new constant block (`08 01 MM …`) while the effect plays
    changes the force at once, with no new start. `08 01 00 …` brings it to zero.
  - **Stop:** `0b 04 00 01` (action 1 on the first slot), device control `00 00 0b 03` (stop
    all effects) and `0c 01` (free) all stop it. `0b 0c 00 01` (action 3), `0b 01 00 01` and
    `09 01` don't stop it.
  - With the autocentre off and the weight on, the rim stays at whichever lock it reaches, so
    stop tests need the autocentre on.
  - **`0b 40`–`7f` is the device gain and scales every force**, the autocentre's included.
    With the gain at `48` (what the probe scripts set), `40`/`7f` couldn't lift the weight to
    the right while `c0`/`80` (helped by the weight) drove the rim to the left lock, which
    looked at first like the autocentre blocking rightward forces. At `0b 7f`, `4f` pushed
    the rim to +11,700 against the autocentre, and to the right lock with it off.
- **Spring condition** (`04 ii CP PC NC PS NS DB`, the patent's order): with the autocentre
  off and the weight holding the rim left of centre, NC (byte 4) with NS (byte 6) pulled it
  back to centre (`00 00 7f 00 7f 00`, or all of PC…NS at `7f`), while PC/PS alone or any single
  byte did nothing. All six bytes at `7f` pulled the rim past centre to the right, so byte 2
  is the centre-point offset. The spring needs only the create and this block, then the start.
- **Damper, inertia, friction** (types `09`, `0a`, `0b`, same condition block): tested by
  starting the effect, then switching the autocentre off so the weight pulls the rim from
  about −7,000 to its stop. Time from −9,000 to −19,000: 1.07–1.21 s with no effect; the
  damper 1.31/1.46/1.60 s for coefficients `20`/`40`/`7f` (NC and PC set, saturations `7f`);
  inertia 1.39–1.62 s and friction 1.22–1.65 s with no clear trend. So all three resist, but
  weakly; the spring (`08`) at `20`, `40` or `7f` held the rim completely, and with
  coefficients `ff` (−1) it let it fall.
- **Second effect:** a constant force created at index 2 plays when started either as
  `0b 08 00 02` (first slot byte holding index 2) or `0b 20 00 00 02` (second slot byte).
- **Envelope** (`03 ii …`): setting any one byte to `7f` with a `c0` force didn't visibly
  change the push (onset 0.2–0.8 s either way, like the all-zero envelope). Not mapped yet.
- **Ramp:** create type `02` is accepted with block control `05` (other block controls are
  rejected with status 4) and then plays like a constant force with the constant block
  (`08`); the ramp block `05` is always rejected (status 5). Bytes 3–4 of the constant block
  never acted as ramp levels. Types `0c`–`10` are accepted at create but reject every block.
  So there may be no hardware ramp; the host can ramp a constant force with live updates.
- **Periodic** (`07 ii …` + envelope): not mapped yet. Byte 2 alone does nothing; with byte 6
  or 7 also set the rim gets pushed, but with the weight rig only the leftward half shows
  (autocentre on) or the timing is irregular (off). Mapping magnitude/offset/phase/period is
  better done by feel, with the weight off.
- Autocentre off (`0b 10`), then the effects started (spring/damper/inertia/friction with
  condition blocks, the five periodic waves, constant force; starts `0b 08`, `0b 0c`, `09`, and
  `0b 08` + `0b 06` + `0b 01`): none pulled the rim off its stop. The device controls after
  a start didn't either; `0b 14`/`1c` moved it, but they do that alone too. Device control 4
  (reset) turns the autocentre back on even with bit `10` set (register 8 goes back to `f1`).
  Register 8 bit `40` clears while the autocentre is off (`e1` → `a1`), so it is the native
  effect's state, as the patent reading suggested.
- `kk` = `0c` with no effect loaded answers status `6` for every value (`2` after a create),
  with the normal all-bytes checksum.

### Unknown
- `00 f8 0N 00…` (N = 1–3): seen while the chatpad was not handshaked and keys were being pressed, and from the wheel when buttons were pressed after `09 41` had stopped its reports. Probably "input happened but isn't being reported".
- `00 f8 01` / `00 f8 02` alternate every few seconds while connected; meaning unknown.
- `00 00 00 f0` (every few seconds while connected) and `00 00 00 20/30/40/60` (a burst after connect); meaning unknown.
