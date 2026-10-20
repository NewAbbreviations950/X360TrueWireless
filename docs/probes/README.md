# Racing wheel force-feedback probes

The Xbox 360 Wireless Racing Wheel's force-feedback packets aren't documented anywhere. These
scripts look for them by sending candidate packets through the receiver and watching what the
wheel does. Background is in Microsoft's patent US 8,117,364, which describes a PID-style
protocol: effects are uploaded to the wheel, then started and stopped. The known OUT packets
fit a header of a 4-bit class plus an 8-bit command (`08 4x` LED, `08 c0` power off,
`0c xx` chatpad, `0f c0` rumble), so the scripts sweep the unused classes first.

## Running a probe
Stop the service, start the dump tool, and keep a log:
```sh
systemctl --user stop x360truewireless.service
cargo run --bin x360truewireless-dump | tee captures/wheel-probe-$(date +%F).log
```
Then type commands (slot = the wheel's slot, 0–3; the ring light shows it):
```
p <slot> docs/probes/01-class-8.txt    run a script
c <slot>                               continue after a pause
s <slot>                               stop early
q                                      quit (xpad gets the receiver back)
```
While a probe runs, gamepad reports aren't printed. The probe reads the steering value from
them instead, and prints one line per packet:
```
PROBE slot0 [12/240] 00 00 08 0c 00 00 00 00 00 00 00 00  steering +3  0 new
```
- `steering` is the biggest change in the steering value while watching that packet.
- If it reaches the threshold, the line ends in `!! MOVED` and the probe **pauses**. Note
  what the wheel did (turned which way, pulled to centre, buzzed), then `c` or `s`.
- `NEW <bytes>` lines are packets the controller doesn't normally send, printed the first
  time each one appears, with the packet that came just before.
- If the wheel disconnects (`08 00`), the probe pauses too. Reconnect it, then `c` or `s`.
- At the end, the probe sends rumble-off plus any `stop` packets, and lists every unusual
  packet with a count.

## Safety
- Plug in the AC adapter; force feedback needs it. **Pulling the adapter is the kill switch.**
- Keep your hands off the rim during sweeps, and keep the wheel clamped so it can't fall
  over if the motor kicks.
- Scripts start with low strengths (`40` ≈ 25%). The parser refuses any `00 xx 08 c0`: that's
  power off, and the wheel obeys it with byte 1 set too. Unknown commands could still unpair
  the wheel; if it won't reconnect, re-sync it with the receiver's button.
- `00 00 0b 02` turns the wheel's motor off. Use it as a `stop` packet in scripts that might
  start a force.
- Some payload packets reset the wheel (see `docs/protocol.md`). It reconnects by itself, but
  its LED is cleared, so set the player again (`<slot> 00 00 08 42 00 00 00 00 00 00 00 00`).
- After a power cycle the wheel may join a different slot. Check the `08 80` line first.
- With the wheel's default autocentre, hang a weight from the rim so it rests off-centre.
  Then a probe also sees commands that weaken or switch off the spring.

Results so far are in `docs/protocol.md` under Racing wheel. Short version: classes 0–2 are
register reads, `0b` controls the motor (on/off and a strength), and `00 01 0f c0 kk …` is the
force-feedback channel: the wheel acknowledges and stores effect and parameter blocks
(2026-10-16), but nothing has pushed the rim sideways yet.

## Scripted sessions (`scripts/`)
The `p` probe pauses on every reset, and its blame can lag a step, so the 2026-10-16 session
drove the dump tool from shell scripts instead. Start the tool reading a FIFO:
```sh
systemctl --user stop x360truewireless.service
mkfifo /tmp/wheel.fifo; sleep infinity > /tmp/wheel.fifo &
./target/debug/x360truewireless-dump < /tmp/wheel.fifo | tee captures/wheel-probe-$(date +%F).log
```
Then, with `LOG` (the log), `FIFO` and `SLOT` set:
- `scripts/oracle.sh <file>` sends one packet per line and prints those that draw a reply.
  This is the fast way to map force-feedback fields: no rig needed, the acknowledgement
  status says whether the wheel accepted the packet.
- `scripts/sweep.sh <file>` runs steps (`packet + packet + …` per line), then reports the
  steering swing (`!!MOVED` from 1500), resets, and unusual replies. After a reset it waits
  for the wheel to rejoin and sets the LED and motor again (`SPRING`, default `48`).
- `scripts/each.sh 09:40 09:41 …` sends header-only commands one at a time, each followed by
  a motor off/on check that shows whether input reports still arrive (needs the weight).
Raw sends from the dump tool aren't guarded: only the scripts refuse `00 xx 08 c0`.
Avoid `00 00 09 41`; it stops input reports until a power cycle.

## First session's order
1. **Identify the wheel (phase 0).** With the dump tool running and no probe, connect the
   wheel, then turn the rim fully left, fully right, and press each pedal. In the log:
   - the link-info packet `00 0f 00 f0 …` (compare its bytes with a normal controller's to
     find the device type; XInput's wheel subtype is `02`);
   - which bytes of `00 01 00 f0` reports carry steering and the pedals. The probes assume
     steering at byte 10 (LX). If it's elsewhere, add `axis <byte>` to the top of each script.
2. **`00-rumble.txt`:** does the known rumble packet reach the rumble motors, the steering
   motor, or nothing?
3. **`01-class-8.txt`**, **`02-classes-0-7.txt`**, **`03-classes-9-f.txt`:** header-only
   commands, about 4, 10 and 8 minutes.
4. **`04-payloads.txt`:** payload-carrying packets, about 2 minutes.

If nothing moves and nothing new comes back in any of them, stop there: that's the gate in the
plan, and the next step is rethinking the approach rather than more guessing.

## Checking the game side meanwhile
This part doesn't need the protocol. The daemon can expose a slot as a virtual racing wheel
that accepts every force-feedback effect type and logs what games send:
```sh
systemctl --user stop x360truewireless.service
X360TRUEWIRELESS_FF_LOG=1 ~/.local/bin/x360truewireless-daemon
```
(Reinstall with `./install.sh --local` first so the binary is current. The wheel is detected on connect.)
- `evtest` should list "Microsoft Xbox 360 Wireless Racing Wheel" with steering on `ABS_X`,
  throttle on `ABS_Z` and brake on `ABS_RZ`.
- `fftest /dev/input/eventN` should log one `ff Upload …` / `Play` / `Stop` per effect.
- `scripts/ff-test.py` (python-evdev) plays constant forces both ways, a timed one and a
  spring through the virtual wheel, and prints the steering, so it shows the whole path.
- `protontricks 1465360 joy.cpl` (SnowRunner's prefix) should show the wheel, and its force
  feedback test should produce log lines.
- In SnowRunner, the log shows which effects the game really uses. Those are the ones the
  encoder must support first.
Nothing reaches the wheel's motor yet. These checks show that the PC half works.
