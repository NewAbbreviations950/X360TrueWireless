# Roadmap

Status and planned work. The original design plan (Rust daemon + tray app, installer
script, no Flatpak) is summarised under "Architecture decisions" at the end.

## Done
- Protocol capture and notes (`docs/protocol.md`), including the chatpad wake handshake.
- Daemon: all 4 slots → virtual Xbox 360 pads (buttons, axes, rumble, player LED) plus a
  shared chatpad keyboard. Receiver is handed back to `xpad` on exit.
- Layout-aware typing via libxkbcommon (dlopen'd, with a US fallback). Layout comes from
  KDE/GNOME/localed or `config.toml`.
- One-shot Shift/Green/Orange, Shift+Orange = Caps Lock, Green+People = Steam Quick Access
  (Guide+A), People = Meta.
- `install.sh` (release download / `--local` / `--uninstall`), systemd user unit, udev
  rule, and CI + release workflows.
- People opens Steam friends & chat (`steam://open/friends`).
- Quick Access sends Guide, then A 80 ms later, so Steam doesn't also register a stray A.
- Caps light synced with the system Caps Lock via an `LED_CAPSL` on the virtual keyboard.
- Tray app and settings popup (`x360truewireless-tray`):
  - **Tray icon:** a StatusNotifierItem via `ksni`, sent as pixels rendered from our SVG
    in the panel's text colour. It shows status, connected players and the layout.
  - **Settings popup:** a borderless `egui` window (`--settings`) that closes when it loses
    focus. On KDE Wayland a temporary KWin script places it above the tray icon; on X11 it
    places itself.
  - **Look:** colours come from the KDE colour scheme or the desktop portal, and the font
    is the system UI font.
  - **Keymap editor:** keycaps show all three layers. Keys are selected by clicking or by
    pressing them on the chatpad, and changes save and apply instantly over D-Bus.
- **Daemon D-Bus service** `io.github.NewAbbreviations950.X360TrueWireless`: `Reload`, `Status`,
  and a `ChatpadKey` signal. `crates/common` holds the shared config and keymap.
- **Key remapping** in `config.toml` under `[keys.normal|green|orange]`.
- **Layout changes at runtime:** "Reload" in the tray, or saving in the popup, re-detects
  the layout.
- **Icons:** a 360-style controller icon in full colour for the app menu and in monochrome
  for the tray (`packaging/icons`).
- Chatpad indicator lights:
  - Caps shows Caps Lock or a latched Shift; Green and Orange show latched or held.
  - Re-synced from the status byte whenever the chatpad wakes (see `docs/protocol.md`).


## Next
- **Publish the rename:** the repo is `NewAbbreviations950/X360ChatDriver` (releases up to
  v0.1.1, under the old names). Rename it to `X360TrueWireless` (GitHub redirects the old
  links), then push `dev`, merge into `main` and tag v0.2.0 together: the new installer
  downloads `x360truewireless-x86_64-linux.tar.gz` from the latest release, which v0.1.1
  doesn't have. Then update the Steam Deck with the README's curl command.
- **Test on Steam Deck** (desktop + game mode) and **Kubuntu**:
  - Steam sees exactly one pad per controller.
  - Chatpad types in game mode.
  - Check the layout in game mode: gamescope uses US unless configured.
  - Check that the udev rule in `/etc` survives a SteamOS update.
- **Test the tray and popup on other desktops:** GNOME, both with and without the
  AppIndicator extension (without it the popup should open from the app menu), plus
  Kubuntu, Deck desktop mode and X11. Check that the popup is placed next to the tray.
- **Release build size:** the tray binary is about 12 MB, and could be trimmed if that matters.

## Later
- **Steam controller list at startup:** if the pad already exists when Steam starts, Steam
  uses it but may not list it in Settings → Controller. It does list it once it's hotplugged
  while Steam runs. Check this again, especially when booting into Deck game mode.
- **Use for the People light:** it's currently always off.
- **Battery in the tray:** the popup header shows 4-segment bars per connected player, the
  tray tooltip lists each player's level, and the daemon's D-Bus `Battery` method and a
  low-battery notification are in place. Still to do: maybe show it in the tray menu's player
  lines too. Open protocol questions (the `c0`/`3c` codes and the 2.6–2.7 V full/medium
  boundary) are in `docs/protocol.md`.
- **Automatic layout change detection:** right now it needs a manual Reload from the tray.
  The daemon could watch `kxkbrc`/gsettings instead.
- **Headset audio (planned once the hardware arrives):** both the wired headset plugged into
  a controller's (or the wheel's) jack and the Xbox 360 Wireless Headset, which pairs with the
  receiver by itself. The odd-numbered interfaces (protocol 0x82, interrupt endpoints) carry
  headset audio. The format is undocumented. Research it, then expose a PipeWire sink and
  source.
  - No headset on hand yet (expected in a few weeks). The racing wheel has a headset jack too.
  - First test once it's here: read registers 0–4 (`docs/protocol.md`, register reads) on the
    controller and the wheel with the headset unplugged and plugged in. Register 0 reads `20`
    on a controller with the chatpad and `03` on the wheel, so it may be an attachment field.
  - Also compare the link info of a controller without its chatpad: register 5 (device type,
    used to detect wheels) reads `01` with the chatpad attached; check that no attachment
    changes it.
  - Wireless headset: find out whether it takes a slot of its own (its own presence and link
    info, and what register 5 reads) or shares its controller's, and make sure it doesn't get
    a virtual gamepad.
  - The dump tool only reads the controller interfaces; make it log the audio endpoints too,
    so traffic there shows up during probes.
- **Unknown packets:** `00 f8 0N` (seen while the chatpad wasn't handshaked, and once from the
  wheel). The `00 0f` link-info packet carries register/value pairs (see the register reads in
  `docs/protocol.md`); only the battery (register 1) and device type (register 5) are known.
- **Non-ASCII chatpad legends** (€, š, ¨, «, »): these work only if the layout can type
  them. A compose/Unicode fallback could cover the rest.
- **Packaging extras:** aarch64 builds, AUR/OBS packages, maybe a Flatpak for the GUI only.
- **Power off controller** from the tray or popup (`00 00 08 c0`); needs a D-Bus method.
- **Windows port** (the user has a Windows laptop to test on). The packet parsing
  (`proto`), the force-feedback encoder (`wheel_ff`), the keymap and the config are
  portable, and `nusb` runs on Windows over WinUSB. Steps, in order:
  1. **Split out the Linux-only parts** behind a common layer: virtual devices (uinput),
     chatpad keyboard output (xkbcommon layouts), the receiver handover (`xpad`), and the
     tray↔daemon link (D-Bus). This changes nothing on Linux.
  2. **First milestone, a plain `.exe` run by hand:** take the receiver over through WinUSB
     (switched with Zadig at first), virtual Xbox pads through ViGEmBus (signed, no longer
     maintained but working; Rust client available), and chatpad typing with `SendInput`
     (Unicode, so no layout handling).
  3. **Racing wheel:** a virtual DirectInput joystick with force feedback through vJoy
     (signed, passes games' effects to the program), mapped onto `ffb::Op` and the existing
     encoder. A virtual force-feedback device of our own would need a kernel driver and paid
     signing, so it's out. Check vJoy's force-feedback support with a few games, since it
     differs between versions.
  4. **Tray and settings:** egui already runs on Windows; the tray icon needs a Windows
     library, and the link to the daemon becomes a named pipe (or one process).
  5. **`X360TrueWireless-Setup.exe`** (Inno Setup or WiX), built by the release workflow on a
     Windows runner next to the Linux download. It installs the app, autostart and an
     uninstaller, assigns WinUSB to the receiver (one admin prompt; the driver package needs a
     locally created certificate like Zadig's, or a paid one), and installs ViGEmBus and vJoy.
     Without a code-signing certificate, SmartScreen warns about the setup at first. Later:
     winget.
  - Swapping the receiver to WinUSB turns off Windows' own Xbox support for it, so the
    program has to cover everything, as it does on Linux.
- **Racing wheel force feedback** (Xbox 360 Wireless Racing Wheel, WRW01). Games' effects
  are passed to the wheel through the receiver.
  - Done: a `p` probe command in `x360truewireless-dump` with sweep scripts (`docs/probes/`).
    The daemon can expose a slot as a joystick-style virtual wheel
    (`X360TRUEWIRELESS_WHEEL_SLOTS`) that accepts every evdev effect type and logs it
    (`X360TRUEWIRELESS_FF_LOG`). Effects are normalised in `crates/daemon/src/ffb.rs`.
  - Found (2026-10-14, details in `docs/protocol.md`): link info identifies the wheel
    (register 5 = `82`); classes 0–2 are register reads; `00 00 0b xx` switches the motor
    on/off (`02`/`04`) and sets a spring strength (`40`–`5f`, also `60`–`7f`); rumble
    reaches two motors with the bytes swapped. Header-only classes 0–8 and `0b` have no
    sideways force. Some payload packets reset the wheel.
  - Found (2026-10-16): the spring has no centre offset (`0b 5f`/`7f`/`80`/`c0` all return
    to centre) and `0b 80`–`ff` are ignored. `00 00 0f 81`–`bf` and payload type `03` on
    `0f c0` reset the wheel. `00 00 09 41` stops all input reports until a power cycle.
    Class-`0b` payloads and classes `0d`–`0f` don't push the rim.
    **The force-feedback channel is `00 01 0f c0 kk …`** (rumble is `kk` = `00`): the wheel
    acknowledges create-effect (`02`), envelope/condition/periodic/constant parameter blocks
    (`03`/`04`/`07`/`08`), an operation (`09`) and a free (`0c`) with a status and a payload
    checksum, and the effect types match the patent. Details in `docs/protocol.md`.
  - **Main goal: find the real force-feedback commands** (constant force and the other
    patent effects, played by the wheel's motor), and search for them exhaustively before
    settling for less. No PC driver ever supported this wheel's force feedback, so there's
    no capture to copy; the search is systematic probing (`docs/probes/scripts/`).
    - Nothing moves yet with create + constant parameters + `09`. The acknowledgement map of
      `02`, `03`, `08` and `09` is done: every value field accepts every value, so the ack
      can't show what's missing. The 2026-10-17 search ruled out every `09` byte 2/3 pair
      with a constant force loaded, and host conditions started with `09 01 01 01`.
    - Also ruled out (2026-10-17): every `0b` value around the create and `09`, other
      indices and block controls, duration/gain/loops, periodic waves and envelopes.
    - Also ruled out: the unacknowledged report types (`kk` = `01`, `06`, `0a`, `0d`–`7f`)
      and the other payload types (`00 TT 0f c0`) as a start.
    - Also ruled out: every motor mode (`00 00 0b 00`–`3f`) set before the effect is
      loaded; packets longer than 12 bytes (truncated); other header bytes 0/1.
    - Also ruled out: register writes in the unused header classes.
    - **Found (2026-10-18 morning): constant force and spring work.** Create, parameter
      block, envelope block (`03`, needed for constant/periodic), then start with
      `0b 08 00 01` (stop `0b 04 00 01`). Constant block byte 2 is a signed force; duration
      is in ~10 ms units (`00` = until stopped); gain scales; a new parameter block updates
      the playing effect live. The spring block is `CP PC NC PS NS DB`; damper/inertia/
      friction resist only weakly. `0b 40`–`7f` is the device gain for every force. Details
      in `docs/protocol.md`.
    - **Encoder done** (`crates/daemon/src/wheel_ff.rs`): constant force, spring, damper,
      inertia, friction and rumble; the daemon times delays, durations and repeats itself,
      coalesces updates into parameter blocks, applies the game's gain to the effects and sets
      device gain `7f`. Checked on the wheel with python-evdev effects through the virtual
      wheel. Wheels are detected on connect from the device type (register 5, carried in the
      link info; up to four at once, one per slot), with `X360TRUEWIRELESS_WHEEL_SLOTS` as a
      manual override. Still to do: try `fftest` and a game (SnowRunner),
      and check which way games expect a positive force to turn the rim (it's one constant,
      `CONSTANT_SIGN`; evdev direction `0x4000` currently pushes left).
    - Next, with the weight off and by feel: the periodic block (magnitude/offset/phase/
      period), the envelope fields, damper/inertia/friction strength, several effects at once (indices 2+, the `0b` slot bytes), and what block control `05`/`06`/`07`
      and start delay/loops do.
    - Find what `0c` frees (the memory still fills).
    - **Capture a real game instead (parked for now, brute force first).** On hand: an open
      RGH3 Xbox 360 (phat), a Raspberry Pi Pico and a racing game disc with force feedback.
      A forum pinout lists USB D+/D− on the front RF module next to its serial (LED/sync)
      pins ([AnandTech thread](https://forums.anandtech.com/threads/xbox-360-rf-module-on-computer.174013/)).
      If controller traffic crosses that USB link, a Pico running `usb-sniffer-lite`
      (full-speed USB sniffer firmware) would capture the console's packets while the game
      plays force feedback, likely in the same format as the PC receiver. First check that
      the pins really carry USB. Fallback: a DashLaunch plugin on the RGH console that logs
      the controller/USB calls. Over the air is out: the RTL-SDR on hand doesn't reach
      2.4 GHz, and the link is proprietary and may be encrypted.
    - Periodic and ramp effects are accepted but not played. Once the periodic block is
      mapped it goes in the encoder; the ramp (and periodic waves, if the block can't be
      mapped) can be played by streaming constant-force updates.
  - **Fallback only**, if the search above comes up empty: a centring force from `0b 04`
    (motor on), `0b 02` (off) and `0b 40`–`7f` (strength), which games could scale. Not
    a substitute for the search.

## Architecture decisions (from the initial plan)
- Rust. The daemon runs as a systemd **user** service, so it works in Steam Deck game
  mode. `nusb` handles USB (pure Rust, no libusb) and the `evdev` crate handles uinput.
- No Flatpak for the daemon: it needs a host udev rule and must autostart outside a
  sandbox.
- The only root step is the udev rule (`TAG+="uaccess"`). `/dev/uinput` access comes from
  Steam's own udev rules on most systems.
- Release builds run on ubuntu-22.04 to keep the glibc requirement low. The daemon isn't
  fully static because libxkbcommon is loaded at runtime.
