//! Protocol exploration tool: takes the controller interfaces of an Xbox 360 wireless
//! receiver away from xpad, hex-dumps everything they send, and lets you type packets
//! to send back. xpad is reattached on exit (Ctrl-C or `q`).
//!
//! stdin commands:
//!   <slot> <hex bytes>   send a packet to slot 0-3, e.g. `0 00 00 0c 1b`
//!   k <slot> on|off      toggle chatpad keepalive + wake handshake for a slot
//!   p <slot> <file>      run a probe script on a slot (see `docs/probes/README.md`)
//!   c <slot>             continue a probe that paused because the wheel moved
//!   s <slot>             stop a probe (sends its stop packets)
//!   q                    quit

use std::collections::{HashMap, VecDeque};
use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use nusb::MaybeFuture;
use nusb::transfer::{Buffer, In, Interrupt, Out};

const RECEIVER_IDS: &[(u16, u16)] = &[(0x045e, 0x0719), (0x045e, 0x0291), (0x045e, 0x02a9)];
const SLOTS: u8 = 4;

enum Cmd {
    Send(Vec<u8>),
    Keepalive(bool),
    Probe(Box<Probe>),
    Continue,
    StopProbe,
}

fn main() -> Result<()> {
    let info = nusb::list_devices()
        .wait()?
        .find(|d| RECEIVER_IDS.contains(&(d.vendor_id(), d.product_id())))
        .context("no Xbox 360 wireless receiver found")?;
    println!(
        "found receiver {:04x}:{:04x} on bus {} addr {}",
        info.vendor_id(),
        info.product_id(),
        info.bus_id(),
        info.device_address()
    );
    let device = info
        .open()
        .wait()
        .context("open receiver (need the udev rule or root)")?;

    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        ctrlc::set_handler(move || running.store(false, Ordering::SeqCst))?;
    }

    let result = run(&device, &running);
    // Give every controller interface back to xpad, even ones a crashed earlier run left orphaned.
    #[cfg(target_os = "linux")]
    {
        for slot in 0..SLOTS {
            let _ = device.attach_kernel_driver(slot * 2);
        }
        println!("released interfaces; xpad reattached");
    }
    result
}

fn run(device: &nusb::Device, running: &Arc<AtomicBool>) -> Result<()> {
    // Claim everything before starting threads so a failure leaves nothing half-running.
    let mut claimed = Vec::new();
    for slot in 0..SLOTS {
        let iface = device
            .detach_and_claim_interface(slot * 2)
            .wait()
            .with_context(|| format!("claim interface {}", slot * 2))?;
        // Slot N uses interface 2N with endpoints 0x81+2N (in) / 0x01+2N (out).
        let ep_in = iface.endpoint::<Interrupt, In>(0x81 + slot * 2)?;
        let ep_out = iface.endpoint::<Interrupt, Out>(0x01 + slot * 2)?;
        claimed.push((slot, iface, ep_in, ep_out));
    }

    let start = Instant::now();
    let mut senders = Vec::new();
    let mut threads = Vec::new();
    for (slot, iface, mut ep_in, mut ep_out) in claimed {
        let (tx, rx) = mpsc::channel::<Cmd>();
        senders.push(tx);
        let running = running.clone();
        threads.push(thread::spawn(move || {
            let _iface = iface; // keep claimed (and reattach xpad on drop)
            let mut keepalive = false;
            let mut last_keepalive = Instant::now();
            let mut toggle = false;
            let mut probe: Option<Probe> = None;
            while ep_in.pending() < 4 {
                ep_in.submit(ep_in.allocate(64));
            }
            while running.load(Ordering::SeqCst) {
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        Cmd::Send(bytes) => send(&mut ep_out, slot, &bytes),
                        Cmd::Keepalive(on) => keepalive = on,
                        Cmd::Probe(p) => {
                            let p = *p;
                            if let Some(old) = probe.take() {
                                old.finish(&mut |p| send_quiet(&mut ep_out, p), slot, "replaced");
                            }
                            println!("PROBE slot{slot}: {} steps from {}", p.total, p.name);
                            probe = Some(p);
                        }
                        Cmd::Continue => match probe.as_mut() {
                            Some(p) => p.paused = false,
                            None => println!("PROBE slot{slot}: nothing to continue"),
                        },
                        Cmd::StopProbe => match probe.take() {
                            Some(p) => {
                                p.finish(&mut |p| send_quiet(&mut ep_out, p), slot, "stopped")
                            }
                            None => println!("PROBE slot{slot}: no probe running"),
                        },
                    }
                }
                if let Some(p) = probe.as_mut()
                    && p.step(&mut |p| send_quiet(&mut ep_out, p), slot)
                {
                    probe
                        .take()
                        .unwrap()
                        .finish(&mut |p| send_quiet(&mut ep_out, p), slot, "done");
                }
                if keepalive && last_keepalive.elapsed() >= Duration::from_secs(1) {
                    send(
                        &mut ep_out,
                        slot,
                        &chatpad_cmd(if toggle { 0x1f } else { 0x1e }),
                    );
                    toggle = !toggle;
                    last_keepalive = Instant::now();
                }
                // Probes need tighter timing than the 50 ms idle poll.
                let timeout = Duration::from_millis(if probe.is_some() { 5 } else { 50 });
                if let Some(c) = ep_in.wait_next_complete(timeout) {
                    let t = start.elapsed().as_secs_f64();
                    match c.status {
                        Ok(()) => {
                            let d = &c.buffer[..c.actual_len];
                            // While probing, gamepad reports would drown the log; the
                            // probe tracks the steering value from them instead.
                            match probe.as_mut() {
                                Some(p) => p.on_packet(d, t),
                                None => println!("{t:9.3} slot{slot} IN  {}", hex(d)),
                            }
                            // Chatpad wake request: the host must answer with 0x1b each time.
                            if keepalive
                                && d.len() >= 26
                                && d[..4] == [0x00, 0x02, 0x00, 0xf0]
                                && d[24] == 0xf0
                                && d[25] == 0x03
                            {
                                send(&mut ep_out, slot, &chatpad_cmd(0x1b));
                            }
                        }
                        Err(e) => {
                            println!("{t:9.3} slot{slot} IN  error: {e:?}");
                            thread::sleep(Duration::from_millis(200));
                        }
                    }
                    let mut buf = c.buffer;
                    buf.clear();
                    ep_in.submit(buf);
                }
            }
            if let Some(p) = probe.take() {
                p.finish(&mut |p| send_quiet(&mut ep_out, p), slot, "quit");
            }
            ep_in.cancel_all();
        }));
    }
    println!("claimed all {SLOTS} controller interfaces; xpad detached. Ctrl-C or `q` to quit.");

    // stdin runs on its own thread so Ctrl-C can still end the program while it blocks.
    {
        let running = running.clone();
        thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                match parse_cmd(&line) {
                    Ok(None) => {
                        running.store(false, Ordering::SeqCst);
                        break;
                    }
                    Ok(Some((slot, cmd))) => {
                        let _ = senders[slot].send(cmd);
                    }
                    Err(e) => eprintln!("? {e}"),
                }
            }
        });
    }

    for t in threads {
        let _ = t.join();
    }
    Ok(())
}

fn send(ep: &mut nusb::Endpoint<Interrupt, Out>, slot: u8, bytes: &[u8]) {
    let status = send_quiet(ep, bytes);
    println!("          slot{slot} OUT {} -> {status:?}", hex(bytes));
}

type SendResult = Result<(), nusb::transfer::TransferError>;

fn send_quiet(ep: &mut nusb::Endpoint<Interrupt, Out>, bytes: &[u8]) -> SendResult {
    let mut buf = Buffer::new(bytes.len());
    buf.extend_from_slice(bytes);
    ep.transfer_blocking(buf, Duration::from_millis(500)).status
}

/// Probe packets are padded to the receiver's usual 12-byte OUT size.
const PACKET_LEN: usize = 12;
/// Refuse scripts that would take hours.
const MAX_STEPS: usize = 8192;

/// A scripted probe: sends packets one at a time and watches what each one does. It logs
/// packets the controller doesn't normally send, and pauses when the steering value moves
/// (a racing wheel's motor turned it). The format is described in `docs/probes/README.md`.
struct Probe {
    name: String,
    steps: VecDeque<Step>,
    total: usize,
    sent: usize,
    /// Byte offset of the little-endian i16 steering value in `00 01 00 f0` reports.
    axis: usize,
    /// Steering change that counts as the wheel moving.
    threshold: i32,
    /// How long to watch after each packet.
    settle: Duration,
    /// Sent when the probe ends for any reason, after `rumble(0, 0)`.
    stop: Vec<Vec<u8>>,
    paused: bool,
    watch: Option<Watch>,
    steering: Option<i16>,
    /// Unusual packets seen so far, and how often.
    seen: HashMap<Vec<u8>, usize>,
}

#[derive(Debug, PartialEq)]
enum Step {
    Send(Vec<u8>),
    Wait(Duration),
    Say(String),
}

/// What happened since the last step started.
struct Watch {
    /// None for a `wait` step.
    packet: Option<Vec<u8>>,
    until: Instant,
    baseline: Option<i16>,
    max_delta: i32,
    new_packets: usize,
}

impl Probe {
    fn parse(name: &str, script: &str) -> Result<Probe> {
        let mut probe = Probe {
            name: name.to_owned(),
            steps: VecDeque::new(),
            total: 0,
            sent: 0,
            axis: 10,
            threshold: 1500,
            settle: Duration::from_millis(300),
            stop: Vec::new(),
            paused: false,
            watch: None,
            steering: None,
            seen: HashMap::new(),
        };
        for (n, line) in script.lines().enumerate() {
            let line = line.split('#').next().unwrap().trim();
            let context = || format!("{name}:{}: {line}", n + 1);
            let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let number = || -> Result<u64> { Ok(rest.trim().parse()?) };
            match word {
                "" => {}
                "axis" => probe.axis = number().with_context(context)? as usize,
                "threshold" => probe.threshold = number().with_context(context)? as i32,
                "settle" => probe.settle = Duration::from_millis(number().with_context(context)?),
                "wait" => probe.steps.push_back(Step::Wait(Duration::from_millis(
                    number().with_context(context)?,
                ))),
                "say" => probe.steps.push_back(Step::Say(rest.trim().to_owned())),
                "stop" => {
                    let packet = pad(parse_hex(rest.split_whitespace()).with_context(context)?);
                    if is_power_off(&packet) {
                        bail!("{}: would power the controller off", context());
                    }
                    probe.stop.push(packet)
                }
                _ => {
                    for packet in expand(line).with_context(context)? {
                        if is_power_off(&packet) {
                            bail!("{}: would power the controller off", context());
                        }
                        probe.steps.push_back(Step::Send(packet));
                        probe.total += 1;
                    }
                }
            }
            if probe.total > MAX_STEPS {
                bail!("{name}: more than {MAX_STEPS} packets; split the script");
            }
        }
        Ok(probe)
    }

    /// Advance the script. Returns true once it has finished.
    fn step(&mut self, send: &mut impl FnMut(&[u8]) -> SendResult, slot: u8) -> bool {
        if let Some(watch) = &self.watch {
            if Instant::now() < watch.until {
                return false;
            }
            let watch = self.watch.take().unwrap();
            if let Some(packet) = &watch.packet {
                let moved = watch.max_delta.abs() >= self.threshold;
                println!(
                    "PROBE slot{slot} [{}/{}] {}  steering {:+}  {} new{}",
                    self.sent,
                    self.total,
                    hex(packet),
                    watch.max_delta,
                    watch.new_packets,
                    if moved { "  !! MOVED" } else { "" }
                );
                if moved {
                    self.paused = true;
                    println!(
                        "PROBE slot{slot}: paused because the wheel moved. Note what it did, \
                         then `c {slot}` to continue or `s {slot}` to stop."
                    );
                }
            }
        }
        if self.paused {
            return false;
        }
        let packet = match self.steps.pop_front() {
            None => return true,
            Some(Step::Say(text)) => {
                println!("PROBE slot{slot}: {text}");
                return false;
            }
            Some(Step::Wait(time)) => {
                self.watch(None, time);
                return false;
            }
            Some(Step::Send(packet)) => packet,
        };
        self.sent += 1;
        if let Err(e) = send(&packet) {
            println!("PROBE slot{slot}: sending {} failed: {e:?}", hex(&packet));
        }
        self.watch(Some(packet), self.settle);
        false
    }

    fn watch(&mut self, packet: Option<Vec<u8>>, time: Duration) {
        self.watch = Some(Watch {
            packet,
            until: Instant::now() + time,
            baseline: self.steering,
            max_delta: 0,
            new_packets: 0,
        });
    }

    /// Look at a packet from the controller.
    fn on_packet(&mut self, data: &[u8], t: f64) {
        if data.starts_with(&[0x00, 0x01, 0x00, 0xf0]) {
            if let Some(b) = data.get(self.axis..self.axis + 2) {
                let value = i16::from_le_bytes([b[0], b[1]]);
                self.steering = Some(value);
                if let Some(w) = self.watch.as_mut() {
                    let baseline = *w.baseline.get_or_insert(value);
                    let delta = value as i32 - baseline as i32;
                    if delta.abs() > w.max_delta.abs() {
                        w.max_delta = delta;
                    }
                }
            }
            return;
        }
        if is_routine(data) {
            return;
        }
        if data == [0x08, 0x00] && !self.paused {
            self.paused = true;
            println!(
                "{t:9.3} PROBE: the controller disconnected; paused. Reconnect it, then `c` to \
                 continue or `s` to stop."
            );
        }
        let count = self.seen.entry(data.to_vec()).or_default();
        *count += 1;
        if *count == 1 {
            let after = match self.watch.as_ref().and_then(|w| w.packet.as_ref()) {
                Some(p) => format!(" (after {})", hex(p)),
                None => String::new(),
            };
            println!("{t:9.3} NEW {}{after}", hex(data));
            if let Some(w) = self.watch.as_mut() {
                w.new_packets += 1;
            }
        }
    }

    /// Send the stop packets and print a summary.
    fn finish(self, send: &mut impl FnMut(&[u8]) -> SendResult, slot: u8, why: &str) {
        let rumble_off = pad(vec![0x00, 0x01, 0x0f, 0xc0]);
        for packet in std::iter::once(&rumble_off).chain(&self.stop) {
            let _ = send(packet);
        }
        println!(
            "PROBE slot{slot}: {why} after {}/{} packets; {} distinct unusual packets",
            self.sent,
            self.total,
            self.seen.len()
        );
        let mut seen: Vec<_> = self.seen.into_iter().collect();
        seen.sort();
        for (packet, count) in seen {
            println!("PROBE slot{slot}:   {count:5}× {}", hex(&packet));
        }
    }
}

/// Packets every connected controller sends anyway (see `docs/protocol.md`). Presence
/// packets are not routine: a probe that disconnects the controller should show up.
fn is_routine(d: &[u8]) -> bool {
    matches!(
        d,
        [0x00, 0x01 | 0x02, 0x00, 0xf0, ..]
            | [
                0x00,
                0x00,
                0x00,
                0x13 | 0x17 | 0xf0 | 0x20 | 0x30 | 0x40 | 0x60,
                ..
            ]
            | [0x00, 0xf8, 0x01 | 0x02, ..]
    )
}

/// `08 c0` is power off. The wheel also obeys it with byte 1 set (`00 01 08 c0`).
fn is_power_off(packet: &[u8]) -> bool {
    matches!(packet, [0x00, _, 0x08, 0xc0, ..])
}

fn pad(mut packet: Vec<u8>) -> Vec<u8> {
    if packet.len() < PACKET_LEN {
        packet.resize(PACKET_LEN, 0);
    }
    packet
}

/// Expand a packet line: hex bytes, where `{a0..af}` stands for every byte in that range.
/// Several ranges give every combination, the leftmost varying slowest.
fn expand(line: &str) -> Result<Vec<Vec<u8>>> {
    let mut packets = vec![Vec::new()];
    for word in line.split_whitespace() {
        let choices: Vec<Vec<u8>> = match word.strip_prefix('{').and_then(|w| w.strip_suffix('}')) {
            Some(range) => {
                let (from, to) = range
                    .split_once("..")
                    .context("ranges look like {00..ff}")?;
                let from = u8::from_str_radix(from, 16).context("bad range start")?;
                let to = u8::from_str_radix(to, 16).context("bad range end")?;
                if from > to {
                    bail!("range {word} is backwards");
                }
                (from..=to).map(|b| vec![b]).collect()
            }
            None => vec![parse_hex(std::iter::once(word))?],
        };
        packets = packets
            .iter()
            .flat_map(|p| choices.iter().map(move |c| [p.as_slice(), c].concat()))
            .collect();
        if packets.len() > MAX_STEPS {
            bail!("line expands to more than {MAX_STEPS} packets");
        }
    }
    Ok(packets.into_iter().map(pad).collect())
}

fn chatpad_cmd(cmd: u8) -> [u8; 12] {
    [0x00, 0x00, 0x0c, cmd, 0, 0, 0, 0, 0, 0, 0, 0]
}

/// `Ok(None)` means quit.
fn parse_cmd(line: &str) -> Result<Option<(usize, Cmd)>> {
    let mut words = line.split_whitespace();
    let first = match words.next() {
        Some(w) => w,
        None => bail!("empty line"),
    };
    if first == "q" {
        return Ok(None);
    }
    if first == "p" {
        let slot = parse_slot(words.next())?;
        let path = words.next().context("missing probe script path")?;
        let script = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
        let probe = Probe::parse(path, &script)?;
        return Ok(Some((slot, Cmd::Probe(Box::new(probe)))));
    }
    if first == "c" || first == "s" {
        let slot = parse_slot(words.next())?;
        let cmd = if first == "c" {
            Cmd::Continue
        } else {
            Cmd::StopProbe
        };
        return Ok(Some((slot, cmd)));
    }
    if first == "k" {
        let slot = parse_slot(words.next())?;
        let on = matches!(words.next(), Some("on"));
        return Ok(Some((slot, Cmd::Keepalive(on))));
    }
    let slot = parse_slot(Some(first))?;
    let bytes = parse_hex(words)?;
    if bytes.is_empty() {
        bail!("no bytes to send");
    }
    Ok(Some((slot, Cmd::Send(bytes))))
}

/// Hex bytes, either spaced (`00 01`) or run together (`0001`).
fn parse_hex<'a>(words: impl Iterator<Item = &'a str>) -> Result<Vec<u8>> {
    words
        .flat_map(|w| {
            w.as_bytes()
                .chunks(2)
                .map(|c| std::str::from_utf8(c).unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .map(|b| u8::from_str_radix(&b, 16).with_context(|| format!("bad hex byte {b:?}")))
        .collect()
}

fn parse_slot(w: Option<&str>) -> Result<usize> {
    let slot: usize = w
        .context("missing slot")?
        .parse()
        .context("slot must be 0-3")?;
    if slot >= SLOTS as usize {
        bail!("slot must be 0-3");
    }
    Ok(slot)
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_ranges_leftmost_slowest() {
        let packets = expand("00 {01..02} 0f {c0..c1}").unwrap();
        let heads: Vec<_> = packets.iter().map(|p| hex(&p[..4])).collect();
        assert_eq!(
            heads,
            ["00 01 0f c0", "00 01 0f c1", "00 02 0f c0", "00 02 0f c1"]
        );
        assert!(packets.iter().all(|p| p.len() == PACKET_LEN));
        assert!(expand("00 {05..04}").is_err());
    }

    #[test]
    fn parses_settings_and_steps() {
        let probe = Probe::parse(
            "t",
            "# header\nsettle 100\naxis 12\nstop 00 00 0b 02\nsay hi\n00 00 {08..09} 00 # two\nwait 50\n",
        )
        .unwrap();
        assert_eq!(probe.settle, Duration::from_millis(100));
        assert_eq!(probe.axis, 12);
        assert_eq!(probe.stop, [pad(vec![0, 0, 0x0b, 0x02])]);
        assert_eq!(probe.total, 2);
        assert_eq!(probe.steps.len(), 4);
        assert_eq!(probe.steps[0], Step::Say("hi".into()));
        assert_eq!(probe.steps[3], Step::Wait(Duration::from_millis(50)));
    }

    #[test]
    fn shipped_probe_scripts_parse() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/probes");
        let mut scripts = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "txt") {
                let script = std::fs::read_to_string(&path).unwrap();
                let probe = Probe::parse(&path.display().to_string(), &script).unwrap();
                assert!(probe.total > 0, "{}", path.display());
                scripts += 1;
            }
        }
        assert!(scripts >= 5);
    }

    #[test]
    fn refuses_to_power_off() {
        for script in ["00 00 08 c0", "00 01 {00..0e} c0", "stop 00 01 08 c0"] {
            assert!(Probe::parse("t", script).is_err(), "{script}");
        }
        assert!(Probe::parse("t", "00 00 08 {c1..c2}").is_ok());
    }

    fn gamepad(steering: i16) -> Vec<u8> {
        let mut report = vec![0x00, 0x01, 0x00, 0xf0, 0x00, 0x13, 0, 0, 0, 0];
        report.extend(steering.to_le_bytes());
        report.resize(29, 0);
        report
    }

    #[test]
    fn probe_pauses_when_the_wheel_moves_or_disconnects() {
        let mut probe = Probe::parse("t", "settle 0\n00 01\n00 02\n00 03\n").unwrap();
        let sent = std::cell::RefCell::new(Vec::new());
        let mut send = |p: &[u8]| {
            sent.borrow_mut().push(p[1]);
            Ok(())
        };
        probe.on_packet(&gamepad(100), 0.0);

        assert!(!probe.step(&mut send, 0));
        // Reports during the watch: the steering swings 2000 away from where it was.
        probe.on_packet(&gamepad(1100), 0.1);
        probe.on_packet(&gamepad(2100), 0.2);
        assert!(!probe.step(&mut send, 0));
        assert!(probe.paused);
        assert!(!probe.step(&mut send, 0));
        assert_eq!(*sent.borrow(), [0x01]);

        probe.paused = false;
        assert!(!probe.step(&mut send, 0));
        // Unusual packets are counted once each; a disconnect pauses.
        probe.on_packet(&[0x00, 0x05, 0x00, 0x00], 0.3);
        probe.on_packet(&[0x00, 0x05, 0x00, 0x00], 0.4);
        probe.on_packet(&[0x08, 0x00], 0.5);
        assert_eq!(probe.watch.as_ref().unwrap().new_packets, 2);
        assert!(!probe.step(&mut send, 0));
        assert!(probe.paused);

        probe.paused = false;
        assert!(!probe.step(&mut send, 0));
        assert!(probe.step(&mut send, 0));
        assert_eq!(*sent.borrow(), [0x01, 0x02, 0x03]);
        assert_eq!(probe.seen[&vec![0x00, 0x05, 0x00, 0x00]], 2);

        let mut stops = Vec::new();
        probe.finish(
            &mut |p: &[u8]| {
                stops.push(p.to_vec());
                Ok(())
            },
            0,
            "done",
        );
        // Rumble off always goes out first.
        assert_eq!(stops, [pad(vec![0x00, 0x01, 0x0f, 0xc0])]);
    }

    #[test]
    fn routine_packets_are_filtered() {
        assert!(is_routine(&[0x00, 0x01, 0x00, 0xf0, 0x00]));
        assert!(is_routine(&[0x00, 0x00, 0x00, 0x13, 0xe2]));
        assert!(!is_routine(&[0x08, 0x00]));
        assert!(!is_routine(&[0x00, 0x0f, 0x00, 0xf0]));
        assert!(!is_routine(&[0x00, 0x05, 0x00, 0x00]));
    }
}
