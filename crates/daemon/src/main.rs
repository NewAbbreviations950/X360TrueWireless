//! x360truewireless-daemon: takes an Xbox 360 Wireless Receiver over from xpad and exposes each
//! controller as a virtual Xbox 360 pad, plus a shared virtual keyboard for chatpads.

mod actions;
mod dbus;
mod devices;
mod ffb;
mod indicators;
mod keyboard;
mod keymap;
mod layout;
mod wheel_ff;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nusb::transfer::{Buffer, In, Interrupt, Out, TransferError};
use nusb::{DeviceInfo, Endpoint, MaybeFuture};
use x360truewireless_common::config::WheelMode;
use x360truewireless_proto::{Battery, ChatpadEvent, Device, InPacket, out, parse};

use devices::{ChatpadState, Pad, Wheel, WheelLayout};
use keyboard::Keyboard;
use keymap::Keymap;
use layout::Layout;

const RECEIVER_IDS: &[(u16, u16)] = &[(0x045e, 0x0719), (0x045e, 0x0291), (0x045e, 0x02a9)];
const SLOTS: u8 = 4;
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);
/// How long to wait for the device type after a connect before assuming a gamepad. The link
/// info carrying it arrives within about 10 ms.
const DEVICE_TYPE_TIMEOUT: Duration = Duration::from_secs(1);
/// How often to re-read a connected controller's battery level.
const BATTERY_REQUEST_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// What a connected controller looks like to games.
enum Controller {
    Pad(Pad),
    /// A racing wheel as a sim wheel with force feedback.
    Wheel(Wheel, wheel_ff::Encoder),
    /// A racing wheel as an Xbox controller.
    WheelPad(Pad),
}

impl Controller {
    /// The mode a racing wheel was set up in; None for a controller.
    fn wheel_mode(&self) -> Option<WheelMode> {
        match self {
            Controller::Pad(_) => None,
            Controller::Wheel(wheel, _) => Some(match wheel.layout() {
                WheelLayout::Gamepad => WheelMode::Wheel,
                WheelLayout::Joystick => WheelMode::Joystick,
            }),
            Controller::WheelPad(_) => Some(WheelMode::Controller),
        }
    }
}

/// Slots whose controller should be exposed as a racing wheel whatever it reports, from the
/// comma-separated `X360TRUEWIRELESS_WHEEL_SLOTS` (e.g. `0` or `0,2`). Wheels are recognised
/// by their device type on connect, so this is only an override.
fn wheel_slots() -> [bool; SLOTS as usize] {
    let mut slots = [false; SLOTS as usize];
    for s in std::env::var("X360TRUEWIRELESS_WHEEL_SLOTS")
        .unwrap_or_default()
        .split(',')
    {
        if let Some(slot) = s.trim().parse::<usize>().ok().filter(|&s| s < slots.len()) {
            slots[slot] = true;
        }
    }
    slots
}

struct Shared {
    running: AtomicBool,
    keymap: RwLock<Keymap>,
    /// How racing wheels show up; a change applies to connected wheels at once.
    wheel_mode: RwLock<WheelMode>,
    /// Which controller slots have a controller connected.
    connected: [AtomicBool; SLOTS as usize],
    /// Last battery level reported per slot, as bars (1–4); 0 = unknown.
    battery: [AtomicU8; SLOTS as usize],
    /// Session bus connection, if available, for status and key signals.
    dbus: Option<zbus::blocking::Connection>,
    keyboard: Mutex<Keyboard>,
    /// System Caps Lock state, as mirrored onto our keyboard by the compositor.
    caps_lock: AtomicBool,
}

impl Shared {
    fn set_connected(&self, slot: u8, connected: bool) {
        self.connected[slot as usize].store(connected, Ordering::SeqCst);
        if !connected {
            self.battery[slot as usize].store(0, Ordering::SeqCst);
        }
    }

    fn set_battery(&self, slot: u8, level: Battery) {
        self.battery[slot as usize].store(level.bars(), Ordering::SeqCst);
    }

    fn key_pressed(&self, slot: u8, key: x360truewireless_proto::ChatpadKey) {
        if let Some(conn) = &self.dbus {
            dbus::emit_key(conn, slot, key);
        }
    }

    /// Re-read the config and re-detect the layout. Returns config problems, if any.
    fn reload(&self) -> Vec<String> {
        let (keymap, wheel_mode, problems) = load_config();
        *self.keymap.write().unwrap() = keymap;
        *self.wheel_mode.write().unwrap() = wheel_mode;
        problems
    }

    fn type_keys(&self, events: &[(evdev::KeyCode, bool)]) -> std::io::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        self.keyboard.lock().unwrap().send(events)
    }
}

fn main() -> Result<()> {
    let (keymap, wheel_mode, problems) = load_config();
    for p in problems {
        eprintln!("config: {p}");
    }
    let (keyboard, mut leds) =
        Keyboard::new().context("create virtual keyboard (is /dev/uinput accessible?)")?;
    let shared = Arc::new_cyclic(|weak| Shared {
        running: AtomicBool::new(true),
        keymap: RwLock::new(keymap),
        wheel_mode: RwLock::new(wheel_mode),
        connected: Default::default(),
        battery: Default::default(),
        dbus: dbus::serve(weak.clone()),
        keyboard: Mutex::new(keyboard),
        caps_lock: AtomicBool::new(false),
    });
    {
        let shared = shared.clone();
        thread::spawn(move || {
            while let Ok(on) = leds.next_caps_lock() {
                shared.caps_lock.store(on, Ordering::SeqCst);
            }
        });
    }
    {
        let shared = shared.clone();
        ctrlc::set_handler(move || shared.running.store(false, Ordering::SeqCst))?;
    }

    let mut waiting_logged = false;
    while shared.running.load(Ordering::SeqCst) {
        match find_receiver() {
            Ok(Some(info)) => {
                waiting_logged = false;
                eprintln!(
                    "receiver {:04x}:{:04x} at {}",
                    info.vendor_id(),
                    info.product_id(),
                    info.sysfs_path().display()
                );
                if let Err(e) = run_receiver(&info, &shared) {
                    eprintln!("receiver stopped: {e:#}");
                }
            }
            Ok(None) if !waiting_logged => {
                eprintln!("waiting for an Xbox 360 wireless receiver…");
                waiting_logged = true;
            }
            Ok(None) => {}
            Err(e) => eprintln!("listing USB devices failed: {e}"),
        }
        sleep_while_running(&shared, Duration::from_secs(2));
    }
    eprintln!("stopped");
    Ok(())
}

/// Build the keymap from config.toml and the detected (or configured) keyboard layout.
/// Returns it and the wheel mode, along with any config problems, which are reported but not
/// fatal.
fn load_config() -> (Keymap, WheelMode, Vec<String>) {
    let mut problems = Vec::new();
    let config = x360truewireless_common::config::Config::load().unwrap_or_else(|e| {
        problems.push(format!("{e}; using defaults"));
        Default::default()
    });
    let configured = config
        .layout_name()
        .map(|(layout, variant)| layout::LayoutName { layout, variant });
    let layout = Layout::load(configured);
    eprintln!("typing for keyboard layout {}", layout.name);
    let (table, errors) = x360truewireless_common::keymap::KeyTable::with_overrides(&config.keys);
    problems.extend(errors);
    let wheel_mode = config.wheel_mode.unwrap_or_default();
    (Keymap::with_table(layout, table), wheel_mode, problems)
}

fn find_receiver() -> Result<Option<DeviceInfo>> {
    Ok(nusb::list_devices()
        .wait()?
        .find(|d| RECEIVER_IDS.contains(&(d.vendor_id(), d.product_id()))))
}

fn run_receiver(info: &DeviceInfo, shared: &Arc<Shared>) -> Result<()> {
    let device = info
        .open()
        .wait()
        .context("open receiver (is the udev rule installed?)")?;

    // Claim every slot before starting any, so a failure never leaves half a receiver running.
    let claimed = (0..SLOTS)
        .map(|slot| {
            let iface = device.detach_and_claim_interface(slot * 2).wait();
            iface.with_context(|| format!("claim interface {}", slot * 2))
        })
        .collect::<Result<Vec<_>>>();
    let claimed = match claimed {
        Ok(claimed) => claimed,
        Err(e) => {
            restore_xpad(&device, info);
            return Err(e);
        }
    };

    let threads: Vec<_> = claimed
        .into_iter()
        .zip(0..SLOTS)
        .map(|(iface, slot)| {
            let shared = shared.clone();
            thread::spawn(move || {
                let result = run_slot(slot, &iface, &shared);
                if let Err(e) = &result {
                    eprintln!("slot {slot}: {e:#}");
                }
                // Dropping `iface` releases it and lets xpad have it back.
                result
            })
        })
        .collect();
    let mut result = Ok(());
    for t in threads {
        if let Ok(Err(e)) = t.join() {
            result = Err(e);
        }
    }
    restore_xpad(&device, info);
    result
}

fn run_slot(slot: u8, iface: &nusb::Interface, shared: &Shared) -> Result<()> {
    let mut ep_in = iface.endpoint::<Interrupt, In>(0x81 + slot * 2)?;
    let mut ep_out = iface.endpoint::<Interrupt, Out>(0x01 + slot * 2)?;
    while ep_in.pending() < 4 {
        ep_in.submit(ep_in.allocate(32));
    }
    send(&mut ep_out, &out::presence_inquiry())?;

    let force_wheel = wheel_slots()[slot as usize];
    // Log games' force-feedback requests to a wheel (X360TRUEWIRELESS_FF_LOG=1).
    let ff_log = std::env::var_os("X360TRUEWIRELESS_FF_LOG").is_some();
    let mut controller: Option<Controller> = None;
    // Connected, but the device type hasn't arrived yet: the deadline for it.
    let mut pending: Option<Instant> = None;
    let mut chatpad = ChatpadState::default();
    let mut indicators = indicators::IndicatorSync::default();
    let mut keepalive_phase = false;
    let mut next_keepalive = Instant::now();
    let mut next_battery_request = Instant::now();

    let result = (|| -> Result<()> {
        while shared.running.load(Ordering::SeqCst) {
            // Poll faster while a controller is connected so rumble requests go out promptly.
            let timeout = if controller.is_some() || pending.is_some() {
                Duration::from_millis(8)
            } else {
                Duration::from_millis(100)
            };
            if let Some(c) = ep_in.wait_next_complete(timeout) {
                match c.status {
                    Ok(()) => match parse(&c.buffer[..c.actual_len]) {
                        InPacket::Presence { connected: true }
                            if controller.is_none() && pending.is_none() =>
                        {
                            eprintln!("slot {slot}: controller connected");
                            if force_wheel {
                                controller =
                                    Some(new_controller(slot, Device::Wheel, shared, &mut ep_out)?);
                            } else {
                                // The link info usually brings the type first; ask in case.
                                pending = Some(Instant::now() + DEVICE_TYPE_TIMEOUT);
                                send(&mut ep_out, &out::device_request())?;
                            }
                            send(&mut ep_out, &out::led(2 + slot))?;
                            indicators.reset();
                            next_keepalive = Instant::now();
                            next_battery_request = Instant::now() + BATTERY_REQUEST_INTERVAL;
                            shared.set_connected(slot, true);
                        }
                        InPacket::Presence { connected: false }
                            if controller.is_some() || pending.is_some() =>
                        {
                            eprintln!("slot {slot}: controller disconnected");
                            controller = None;
                            pending = None;
                            shared.set_connected(slot, false);
                            let out = chatpad.release_all(&shared.keymap.read().unwrap());
                            shared.type_keys(&out.keys)?;
                        }
                        InPacket::Gamepad(state) => match controller.as_mut() {
                            Some(Controller::Pad(pad) | Controller::WheelPad(pad)) => {
                                pad.emit(&state)?
                            }
                            Some(Controller::Wheel(wheel, _)) => wheel.emit(&state)?,
                            None => {}
                        },
                        InPacket::LinkInfo { battery, device } => {
                            if let Some(level) = battery {
                                shared.set_battery(slot, level);
                            }
                            if let (Some(device), Some(_)) = (device, pending) {
                                controller =
                                    Some(new_controller(slot, device, shared, &mut ep_out)?);
                                pending = None;
                            }
                        }
                        InPacket::Device(device) if pending.is_some() => {
                            controller = Some(new_controller(slot, device, shared, &mut ep_out)?);
                            pending = None;
                        }
                        InPacket::Battery(level) => shared.set_battery(slot, level),
                        InPacket::Chatpad(ChatpadEvent::WakeRequest) => {
                            send(&mut ep_out, &out::chatpad_wake_ack())?
                        }
                        InPacket::Chatpad(ChatpadEvent::Active { leds }) => {
                            indicators.on_status(leds)
                        }
                        InPacket::Chatpad(ChatpadEvent::Keys(keys)) => {
                            let out = chatpad.update(keys, &shared.keymap.read().unwrap());
                            shared.type_keys(&out.keys)?;
                            for key in &out.pressed {
                                shared.key_pressed(slot, *key);
                            }
                            if out.open_chat {
                                actions::open_steam_chat();
                            }
                            if out.caps_toggled {
                                // Show it right away; the compositor's LED update confirms it.
                                shared.caps_lock.fetch_xor(true, Ordering::SeqCst);
                            }
                            if let (Some(on), Some(Controller::Pad(pad))) =
                                (out.quick_menu, controller.as_mut())
                            {
                                pad.quick_menu(on, Instant::now())?;
                            }
                        }
                        _ => {}
                    },
                    Err(TransferError::Disconnected) => anyhow::bail!("receiver unplugged"),
                    Err(e) => {
                        eprintln!("slot {slot}: read error {e:?}");
                        thread::sleep(Duration::from_millis(100));
                    }
                }
                let mut buf = c.buffer;
                buf.clear();
                ep_in.submit(buf);
            }

            if pending.is_some_and(|deadline| Instant::now() >= deadline) {
                eprintln!("slot {slot}: no device type reported; treating it as a gamepad");
                controller = Some(new_controller(slot, Device::Gamepad, shared, &mut ep_out)?);
                pending = None;
            }

            // The wheel mode was changed: replace the wheel's virtual device.
            let wanted = *shared.wheel_mode.read().unwrap();
            if let Some(mode) = controller.as_ref().and_then(Controller::wheel_mode)
                && mode != wanted
            {
                eprintln!("slot {slot}: switching the wheel to {wanted:?} mode");
                if let Some(Controller::Wheel(_, encoder)) = controller.as_mut() {
                    let mut packets = Vec::new();
                    encoder.stop_all(&mut packets);
                    for p in &packets {
                        send(&mut ep_out, p)?;
                    }
                }
                // Remove the old device before the new one appears.
                controller = None;
                controller = Some(new_controller(slot, Device::Wheel, shared, &mut ep_out)?);
            }

            match controller.as_mut() {
                Some(Controller::Pad(pad)) => {
                    pad.tick(Instant::now())?;
                    if let Some((strong, weak)) = pad.poll_ff()? {
                        send(&mut ep_out, &out::rumble(strong, weak))?;
                    }
                }
                Some(Controller::WheelPad(pad)) => {
                    pad.tick(Instant::now())?;
                    // The wheel's rumble bytes are swapped: the first drives the small motor.
                    if let Some((strong, weak)) = pad.poll_ff()? {
                        send(&mut ep_out, &out::rumble(weak, strong))?;
                    }
                }
                Some(Controller::Wheel(wheel, encoder)) => {
                    let now = Instant::now();
                    let mut packets = Vec::new();
                    for op in wheel.poll_ff()? {
                        if ff_log {
                            eprintln!("slot {slot}: ff {op:?}");
                        }
                        encoder.op(op, now, &mut packets);
                    }
                    encoder.tick(now, &mut packets);
                    for p in &packets {
                        if ff_log {
                            eprintln!("slot {slot}: ff -> {p:02x?}");
                        }
                        send(&mut ep_out, p)?;
                    }
                }
                None => {}
            }
            // The link info only gives the battery level at connect time, so ask again now and then.
            if controller.is_some() && Instant::now() >= next_battery_request {
                send(&mut ep_out, &out::battery_request())?;
                next_battery_request = Instant::now() + BATTERY_REQUEST_INTERVAL;
            }
            // Chatpad upkeep; a wheel has no chatpad port.
            if matches!(controller, Some(Controller::Pad(_))) {
                for (bit, on) in indicators.poll(
                    chatpad.indicators(shared.caps_lock.load(Ordering::SeqCst)),
                    Instant::now(),
                ) {
                    send(&mut ep_out, &out::chatpad_led(bit, on))?;
                }
                if Instant::now() >= next_keepalive {
                    send(&mut ep_out, &out::chatpad_keepalive(keepalive_phase))?;
                    keepalive_phase = !keepalive_phase;
                    next_keepalive = Instant::now() + KEEPALIVE_INTERVAL;
                }
            }
        }
        Ok(())
    })();

    let _ = shared.type_keys(&chatpad.release_all(&shared.keymap.read().unwrap()).keys);
    if controller.is_some() || pending.is_some() {
        shared.set_connected(slot, false);
        let _ = send(&mut ep_out, &out::rumble(0, 0));
    }
    if let Some(Controller::Wheel(_, encoder)) = controller.as_mut() {
        let mut packets = Vec::new();
        encoder.stop_all(&mut packets);
        for p in &packets {
            let _ = send(&mut ep_out, p);
        }
    }
    ep_in.cancel_all();
    result
}

/// The virtual device for what connected: a racing wheel for a wheel, a pad for the rest.
/// Create the virtual device for a newly connected controller, or a wheel in the current
/// wheel mode.
fn new_controller(
    slot: u8,
    device: Device,
    shared: &Shared,
    ep_out: &mut Endpoint<Interrupt, Out>,
) -> Result<Controller> {
    Ok(match device {
        Device::Wheel => {
            let mode = *shared.wheel_mode.read().unwrap();
            eprintln!("slot {slot}: racing wheel ({mode:?} mode)");
            let layout = match mode {
                WheelMode::Wheel => WheelLayout::Gamepad,
                WheelMode::Joystick => WheelLayout::Joystick,
                WheelMode::Controller => {
                    // No game forces in this mode: just the wheel's own autocentre.
                    for p in wheel_ff::Encoder::autocentre_only() {
                        send(ep_out, &p)?;
                    }
                    return Ok(Controller::WheelPad(
                        Pad::new().context("create virtual pad")?,
                    ));
                }
            };
            Controller::Wheel(
                Wheel::new(layout).context("create virtual wheel")?,
                wheel_ff::Encoder::new(),
            )
        }
        Device::Gamepad => Controller::Pad(Pad::new().context("create virtual pad")?),
        Device::Other(subtype) => {
            eprintln!(
                "slot {slot}: unknown device subtype {subtype:#04x}; treating it as a gamepad"
            );
            Controller::Pad(Pad::new().context("create virtual pad")?)
        }
    })
}

fn send(ep: &mut Endpoint<Interrupt, Out>, packet: &out::Packet) -> Result<()> {
    let mut buf = Buffer::new(packet.len());
    buf.extend_from_slice(packet);
    match ep.transfer_blocking(buf, Duration::from_millis(200)).status {
        Ok(()) => Ok(()),
        Err(TransferError::Disconnected) => anyhow::bail!("receiver unplugged"),
        // A dropped output packet (e.g. controller out of range) isn't fatal.
        Err(e) => {
            eprintln!("send {packet:02x?} failed: {e:?}");
            Ok(())
        }
    }
}

fn info_still_present(info: &DeviceInfo) -> bool {
    info.sysfs_path().exists()
}

/// Hand every controller interface back to xpad. xpad's probe occasionally fails right after
/// a release, so check each interface actually got a driver and retry a few times.
fn restore_xpad(device: &nusb::Device, info: &DeviceInfo) {
    let sysfs = info.sysfs_path();
    let name = sysfs
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    for slot in 0..SLOTS {
        let iface = slot * 2;
        let driver_link = sysfs.join(format!("{name}:1.{iface}/driver"));
        let mut last_err = None;
        for attempt in 0..5 {
            if has_driver(&driver_link) {
                break;
            }
            if attempt > 0 {
                thread::sleep(Duration::from_millis(200));
            }
            if let Err(e) = device.attach_kernel_driver(iface) {
                last_err = Some(e);
            }
        }
        if !has_driver(&driver_link) && info_still_present(info) {
            match last_err {
                Some(e) => eprintln!(
                    "warning: could not give interface {iface} back to xpad after 5 attempts: {e}"
                ),
                None => eprintln!(
                    "warning: could not give interface {iface} back to xpad after 5 attempts"
                ),
            }
        }
    }
}

/// True only when the interface is bound to xpad. Any driver would do for a plain existence
/// check, but while we still hold the interface its link points at usbfs, which would make
/// the handback think it had already succeeded.
fn has_driver(link: &Path) -> bool {
    std::fs::read_link(link).is_ok_and(|target| target.file_name() == Some("xpad".as_ref()))
}

fn sleep_while_running(shared: &Shared, total: Duration) {
    let end = Instant::now() + total;
    while shared.running.load(Ordering::SeqCst) && Instant::now() < end {
        thread::sleep(Duration::from_millis(100));
    }
}
