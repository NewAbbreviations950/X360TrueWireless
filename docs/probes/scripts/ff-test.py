#!/usr/bin/env python3
"""Play force-feedback effects on the driver's virtual racing wheel, the way a game would,
and print the steering the wheel reports. Needs python-evdev and the driver running.

    python3 docs/probes/scripts/ff-test.py

Keep hands off the rim. Expected with the current encoder: direction 0x4000 turns the rim
left (negative steering), 0xC000 right; the 1 s effect lets go after a second; the spring
pulls the rim towards centre. Whether games expect 0x4000 to mean left is checked in a game
(see docs/ROADMAP.md); flip CONSTANT_SIGN in crates/daemon/src/wheel_ff.rs if not.
"""
import sys, time, select
import evdev
from evdev import ecodes as e, ff

dev = next(evdev.InputDevice(p) for p in evdev.list_devices()
           if evdev.InputDevice(p).name == "Microsoft Xbox 360 Wireless Racing Wheel")
print("device", dev.path, flush=True)

def steering_for(seconds):
    end, samples, last = time.time() + seconds, [], None
    t0 = time.time()
    while time.time() < end:
        r, _, _ = select.select([dev.fd], [], [], 0.05)
        if r:
            for ev in dev.read():
                if ev.type == e.EV_ABS and ev.code == e.ABS_X:
                    last = ev.value
        if last is not None and (not samples or abs(samples[-1][1] - last) > 2500):
            samples.append((round(time.time() - t0, 2), last))
    return " ".join(f"{t}:{v}" for t, v in samples) + f" (end {last})"

def effect(kind, data, length_ms=0, direction=0x4000):
    return ff.Effect(kind, -1, direction, ff.Trigger(0, 0), ff.Replay(length_ms, 0), ff.EffectType(**data))

def run(name, eff, play_s=2.5, settle_s=2.0):
    eid = dev.upload_effect(eff)
    dev.write(e.EV_FF, eid, 1)
    print(f"{name}: playing  {steering_for(play_s)}", flush=True)
    dev.write(e.EV_FF, eid, 0)
    print(f"{name}: stopped  {steering_for(settle_s)}", flush=True)
    dev.erase_effect(eid)

env = ff.Envelope(0, 0, 0, 0)
print("idle:", steering_for(1.5), flush=True)
for direction, label in [(0x4000, "constant +level dir 0x4000"), (0xC000, "constant +level dir 0xC000")]:
    run(label, effect(e.FF_CONSTANT, {"ff_constant_effect": ff.Constant(0x5000, env)}, direction=direction))
run("constant 1 s duration", effect(e.FF_CONSTANT, {"ff_constant_effect": ff.Constant(0x5000, env)}, length_ms=1000, direction=0xC000), play_s=3)
cond = ff.Condition(0x7fff, 0x7fff, 0x7fff, 0x7fff, 0, 0)
run("spring", effect(e.FF_SPRING, {"ff_condition_effect": (cond, cond)}, direction=0), play_s=3)
