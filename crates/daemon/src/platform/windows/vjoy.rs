//! vJoy (BrunnerInnovation's signed build, 2.2.x): virtual DirectInput joysticks with force
//! feedback. `vJoyInterface.dll` is loaded at runtime from vJoy's install folder, so the driver
//! runs (without wheels) where vJoy isn't installed. Structs follow the 2.2.2 SDK's
//! `vjoyinterface.h`.

use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use libloading::Library;

use crate::pid::{Control, Direction, EffectType, Operation, Report};

pub const HID_USAGE_X: u32 = 0x30;
pub const HID_USAGE_Y: u32 = 0x31;
pub const HID_USAGE_RZ: u32 = 0x35;
/// Axis values run 0..=AXIS_MAX.
pub const AXIS_MAX: i32 = 0x7fff;

// The effect report's AxesEnabledDirection bits.
const X_AXIS_ENABLE: u8 = 0x01;
const DIRECTION_ENABLE: u8 = 0x04;

const VJD_STAT_OWN: u32 = 0;
const VJD_STAT_FREE: u32 = 1;

// FFBPType: write reports, then feature reports (+0x10).
const PT_EFFREP: u32 = 0x01;
const PT_ENVREP: u32 = 0x02;
const PT_CONDREP: u32 = 0x03;
const PT_PRIDREP: u32 = 0x04;
const PT_CONSTREP: u32 = 0x05;
const PT_RAMPREP: u32 = 0x06;
const PT_EFOPREP: u32 = 0x0a;
const PT_BLKFRREP: u32 = 0x0b;
const PT_CTRLREP: u32 = 0x0c;
const PT_GAINREP: u32 = 0x0d;

#[repr(C)]
struct FfbData {
    size: u32,
    cmd: u32,
    data: *const u8,
}

#[repr(C)]
#[derive(Default)]
struct EffReport {
    effect_block_index: u8,
    effect_type: u32,
    duration: u16,
    trigger_repeat: u16,
    sample_period: u16,
    start_delay: u16,
    gain: u8,
    trigger_button: u8,
    axes_enabled_direction: u8,
    polar: i32,
    /// Polar: 0..=0x7fff for a full turn. Cartesian: X, two's complement.
    direction: u16,
    dir_y: u16,
}

#[repr(C)]
#[derive(Default)]
struct EffEnvelope {
    effect_block_index: u8,
    attack_level: u32,
    fade_level: u32,
    attack_time: u32,
    fade_time: u32,
}

#[repr(C)]
#[derive(Default)]
struct EffCondition {
    effect_block_index: u8,
    is_y: i32,
    center_point_offset: i32,
    pos_coeff: i32,
    neg_coeff: i32,
    pos_satur: u32,
    neg_satur: u32,
    dead_band: i32,
}

#[repr(C)]
#[derive(Default)]
struct EffPeriod {
    effect_block_index: u8,
    magnitude: u32,
    offset: i32,
    phase: u32,
    period: u32,
}

#[repr(C)]
#[derive(Default)]
struct EffConstant {
    effect_block_index: u8,
    magnitude: i32,
}

#[repr(C)]
#[derive(Default)]
struct EffRamp {
    effect_block_index: u8,
    start: i32,
    end: i32,
}

#[repr(C)]
#[derive(Default)]
struct EffOp {
    effect_block_index: u8,
    effect_op: u32,
    loop_count: u8,
}

type FfbCallback = unsafe extern "system" fn(*const FfbData, *mut c_void);
type Parse<T> = unsafe extern "C" fn(*const FfbData, *mut T) -> u32;

struct Api {
    _lib: Library,
    vjoy_enabled: unsafe extern "C" fn() -> i32,
    get_status: unsafe extern "C" fn(u32) -> u32,
    acquire: unsafe extern "C" fn(u32) -> i32,
    relinquish: unsafe extern "C" fn(u32),
    reset: unsafe extern "C" fn(u32) -> i32,
    set_axis: unsafe extern "C" fn(i32, u32, u32) -> i32,
    set_button: unsafe extern "C" fn(i32, u32, u8) -> i32,
    set_cont_pov: unsafe extern "C" fn(u32, u32, u8) -> i32,
    is_device_ffb: unsafe extern "C" fn(u32) -> i32,
    register_ffb: unsafe extern "C" fn(FfbCallback, *mut c_void),
    ffb_device_id: Parse<u32>,
    ffb_type: Parse<u32>,
    ffb_block_index: Parse<u32>,
    ffb_report: Parse<EffReport>,
    ffb_envelope: Parse<EffEnvelope>,
    ffb_condition: Parse<EffCondition>,
    ffb_period: Parse<EffPeriod>,
    ffb_constant: Parse<EffConstant>,
    ffb_ramp: Parse<EffRamp>,
    ffb_operation: Parse<EffOp>,
    ffb_control: Parse<u32>,
    ffb_gain: Parse<u8>,
}

/// Where vJoy installs its interface DLL, then the normal DLL search path.
fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for var in ["ProgramW6432", "ProgramFiles"] {
        if let Some(dir) = std::env::var_os(var) {
            paths.push(PathBuf::from(dir).join(r"vJoy\x64\vJoyInterface.dll"));
        }
    }
    paths.push("vJoyInterface.dll".into());
    paths
}

impl Api {
    fn load() -> Result<Api, String> {
        // SAFETY: vJoyInterface.dll's initialisation has no requirements on us.
        let lib = candidates()
            .into_iter()
            .find_map(|p| unsafe { Library::new(&p) }.ok())
            .ok_or("vJoy is not installed (no vJoyInterface.dll)")?;
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the type matches the SDK's prototype for this export.
                *unsafe { lib.get(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| format!("vJoyInterface.dll: {}: {e}", $name))?
            };
        }
        Ok(Api {
            vjoy_enabled: sym!("vJoyEnabled"),
            get_status: sym!("GetVJDStatus"),
            acquire: sym!("AcquireVJD"),
            relinquish: sym!("RelinquishVJD"),
            reset: sym!("ResetVJD"),
            set_axis: sym!("SetAxis"),
            set_button: sym!("SetBtn"),
            set_cont_pov: sym!("SetContPov"),
            is_device_ffb: sym!("IsDeviceFfb"),
            register_ffb: sym!("FfbRegisterGenCB"),
            ffb_device_id: sym!("Ffb_h_DeviceID"),
            ffb_type: sym!("Ffb_h_Type"),
            ffb_block_index: sym!("Ffb_h_EffectBlockIndex"),
            ffb_report: sym!("Ffb_h_Eff_Report"),
            ffb_envelope: sym!("Ffb_h_Eff_Envlp"),
            ffb_condition: sym!("Ffb_h_Eff_Cond"),
            ffb_period: sym!("Ffb_h_Eff_Period"),
            ffb_constant: sym!("Ffb_h_Eff_Constant"),
            ffb_ramp: sym!("Ffb_h_Eff_Ramp"),
            ffb_operation: sym!("Ffb_h_EffOp"),
            ffb_control: sym!("Ffb_h_DevCtrl"),
            ffb_gain: sym!("Ffb_h_DevGain"),
            _lib: lib,
        })
    }
}

fn api() -> io::Result<&'static Api> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    let api = API.get_or_init(|| {
        let api = Api::load()?;
        // SAFETY: plain calls into the loaded DLL.
        if unsafe { (api.vjoy_enabled)() } == 0 {
            return Err("the vJoy driver isn't running".into());
        }
        // One callback serves every device; it routes packets by device id.
        // SAFETY: `on_ffb` matches FfbGenCB and lives for the whole program.
        unsafe { (api.register_ffb)(on_ffb, std::ptr::null_mut()) };
        Ok(api)
    });
    api.as_ref().map_err(|e| io::Error::other(e.clone()))
}

/// Force-feedback reports for each acquired device, by vJoy device id.
fn routes() -> &'static Mutex<HashMap<u32, Sender<Report>>> {
    static ROUTES: OnceLock<Mutex<HashMap<u32, Sender<Report>>>> = OnceLock::new();
    ROUTES.get_or_init(Default::default)
}

/// An acquired vJoy device; released on drop.
pub struct Device {
    id: u32,
}

impl Device {
    /// Acquire the lowest-numbered free vJoy device. Games that use one wheel take the first
    /// joystick Windows lists, so a lone wheel has to be device 1 whichever player it is.
    pub fn acquire_free(ffb: Sender<Report>) -> io::Result<Device> {
        // One wheel at a time, so two connecting together don't pick the same device.
        static PICKING: Mutex<()> = Mutex::new(());
        let _picking = PICKING.lock().unwrap();
        let api = api()?;
        for id in 1..=16 {
            // SAFETY: plain call into the loaded DLL.
            if unsafe { (api.get_status)(id) } == VJD_STAT_FREE {
                return Device::acquire(id, ffb);
            }
        }
        Err(io::Error::other(
            "no free vJoy device; close programs using vJoy, or re-run the installer",
        ))
    }

    /// Acquire vJoy device `id` (1–16), sending its force-feedback reports to `ffb`.
    pub fn acquire(id: u32, ffb: Sender<Report>) -> io::Result<Device> {
        let api = api()?;
        // SAFETY: plain calls into the loaded DLL.
        unsafe {
            match (api.get_status)(id) {
                VJD_STAT_OWN | VJD_STAT_FREE => {}
                _ => {
                    return Err(io::Error::other(format!(
                        "vJoy device {id} is missing or in use by another program"
                    )));
                }
            }
            if (api.acquire)(id) == 0 {
                return Err(io::Error::other(format!("can't acquire vJoy device {id}")));
            }
            (api.reset)(id);
            if (api.is_device_ffb)(id) == 0 {
                eprintln!("vJoy device {id} has no force feedback; re-run the installer");
            }
        }
        routes().lock().unwrap().insert(id, ffb);
        Ok(Device { id })
    }

    pub fn set_axis(&self, usage: u32, value: i32) {
        // SAFETY: plain call into the loaded DLL.
        unsafe { (api_unchecked().set_axis)(value, self.id, usage) };
    }

    /// `n` is 1-based.
    pub fn set_button(&self, n: u8, down: bool) {
        // SAFETY: plain call into the loaded DLL.
        unsafe { (api_unchecked().set_button)(down as i32, self.id, n) };
    }

    /// Hundredths of a degree clockwise from north, or None for centred.
    pub fn set_pov(&self, angle: Option<u32>) {
        let value = angle.unwrap_or(u32::MAX);
        // SAFETY: plain call into the loaded DLL.
        unsafe { (api_unchecked().set_cont_pov)(value, self.id, 1) };
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        routes().lock().unwrap().remove(&self.id);
        let api = api_unchecked();
        // SAFETY: plain calls into the loaded DLL.
        unsafe {
            (api.reset)(self.id);
            (api.relinquish)(self.id);
        }
    }
}

/// The API, once a `Device` exists (so it loaded).
fn api_unchecked() -> &'static Api {
    api().expect("vJoy loaded before a device was acquired")
}

/// Whether to log games' force-feedback requests (X360TRUEWIRELESS_FF_LOG=1, as for the
/// driver's own log of them).
fn ff_log() -> bool {
    static LOG: OnceLock<bool> = OnceLock::new();
    *LOG.get_or_init(|| std::env::var_os("X360TRUEWIRELESS_FF_LOG").is_some())
}

/// vJoy's force-feedback callback, on vJoy's own thread.
unsafe extern "system" fn on_ffb(data: *const FfbData, _: *mut c_void) {
    let Ok(api) = api() else { return };
    let mut device = 0;
    // SAFETY: `data` is the packet vJoy passed us, valid for this call.
    if unsafe { (api.ffb_device_id)(data, &mut device) } != 0 {
        return;
    }
    // SAFETY: as above.
    let Some(report) = (unsafe { decode(api, data) }) else {
        return;
    };
    if let Some(route) = routes().lock().unwrap().get(&device) {
        let _ = route.send(report);
    }
}

/// Decode one packet with vJoy's helpers.
unsafe fn decode(api: &Api, data: *const FfbData) -> Option<Report> {
    // SAFETY (for every call below): `data` is a live packet and each out-parameter is the
    // struct the helper writes.
    let parse = |f: Parse<_>, out: &mut _| unsafe { f(data, out) } == 0;
    let mut kind = 0u32;
    if !parse(api.ffb_type, &mut kind) {
        return None;
    }
    let mut block = 0u32;
    let index = if parse(api.ffb_block_index, &mut block) {
        block as u8
    } else {
        0
    };
    Some(match kind {
        PT_EFFREP => {
            let mut r = EffReport::default();
            if unsafe { (api.ffb_report)(data, &mut r) } != 0 {
                return None;
            }
            if ff_log() {
                eprintln!(
                    "vJoy effect {}: type {}, axes/direction {:#04x}, polar {}, direction {} {}",
                    r.effect_block_index,
                    r.effect_type,
                    r.axes_enabled_direction,
                    r.polar,
                    r.direction,
                    r.dir_y
                );
            }
            Report::Effect {
                index: r.effect_block_index,
                kind: match r.effect_type {
                    1 => EffectType::Constant,
                    2 => EffectType::Ramp,
                    3 => EffectType::Square,
                    4 => EffectType::Sine,
                    5 => EffectType::Triangle,
                    6 => EffectType::SawUp,
                    7 => EffectType::SawDown,
                    8 => EffectType::Spring,
                    9 => EffectType::Damper,
                    10 => EffectType::Inertia,
                    11 => EffectType::Friction,
                    _ => return None,
                },
                duration_ms: (r.duration != 0xffff).then_some(r.duration),
                start_delay_ms: r.start_delay,
                gain: r.gain,
                // Without the direction bit, the effect works on the enabled axes, signed by
                // its own magnitude.
                direction: if r.axes_enabled_direction & DIRECTION_ENABLE == 0 {
                    Direction::Axes {
                        x: r.axes_enabled_direction & X_AXIS_ENABLE != 0,
                    }
                } else if r.polar != 0 {
                    Direction::Polar(r.direction as f32 / 32768.0)
                } else {
                    Direction::Cartesian {
                        x: r.direction as i16,
                    }
                },
            }
        }
        PT_ENVREP => {
            let mut e = EffEnvelope::default();
            if unsafe { (api.ffb_envelope)(data, &mut e) } != 0 {
                return None;
            }
            Report::Envelope {
                index: e.effect_block_index,
                attack_level: e.attack_level,
                fade_level: e.fade_level,
                attack_ms: e.attack_time,
                fade_ms: e.fade_time,
            }
        }
        PT_CONDREP => {
            let mut c = EffCondition::default();
            if unsafe { (api.ffb_condition)(data, &mut c) } != 0 {
                return None;
            }
            Report::Condition {
                index: c.effect_block_index,
                is_y: c.is_y != 0,
                center: c.center_point_offset,
                positive_coefficient: c.pos_coeff,
                negative_coefficient: c.neg_coeff,
                positive_saturation: c.pos_satur,
                negative_saturation: c.neg_satur,
                deadband: c.dead_band,
            }
        }
        PT_PRIDREP => {
            let mut p = EffPeriod::default();
            if unsafe { (api.ffb_period)(data, &mut p) } != 0 {
                return None;
            }
            Report::Periodic {
                index: p.effect_block_index,
                magnitude: p.magnitude,
                offset: p.offset,
                phase: p.phase,
                period_ms: p.period,
            }
        }
        PT_CONSTREP => {
            let mut c = EffConstant::default();
            if unsafe { (api.ffb_constant)(data, &mut c) } != 0 {
                return None;
            }
            Report::Constant {
                index: c.effect_block_index,
                magnitude: c.magnitude,
            }
        }
        PT_RAMPREP => {
            let mut r = EffRamp::default();
            if unsafe { (api.ffb_ramp)(data, &mut r) } != 0 {
                return None;
            }
            Report::Ramp {
                index: r.effect_block_index,
                start: r.start,
                end: r.end,
            }
        }
        PT_EFOPREP => {
            let mut o = EffOp::default();
            if unsafe { (api.ffb_operation)(data, &mut o) } != 0 {
                return None;
            }
            Report::Operation {
                index: o.effect_block_index,
                op: match o.effect_op {
                    1 => Operation::Start,
                    2 => Operation::Solo,
                    _ => Operation::Stop,
                },
                loop_count: o.loop_count,
            }
        }
        PT_BLKFRREP => Report::Free { index },
        PT_CTRLREP => {
            let mut c = 0u32;
            if !parse(api.ffb_control, &mut c) {
                return None;
            }
            Report::Control(match c {
                1 => Control::EnableActuators,
                2 => Control::DisableActuators,
                3 => Control::StopAll,
                4 => Control::Reset,
                5 => Control::Pause,
                _ => Control::Continue,
            })
        }
        PT_GAINREP => {
            let mut gain = 0u8;
            if unsafe { (api.ffb_gain)(data, &mut gain) } != 0 {
                return None;
            }
            Report::Gain(gain)
        }
        // Create-effect, block-load and pool reports are answered by the driver itself.
        _ => return None,
    })
}
