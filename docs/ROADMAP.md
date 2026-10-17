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
- Tray app and settings popup (`x360chatpad-tray`):
  - **Tray icon:** a StatusNotifierItem via `ksni`, sent as pixels rendered from our SVG
    in the panel's text colour. It shows status, connected players and the layout.
  - **Settings popup:** a borderless `egui` window (`--settings`) that closes when it loses
    focus. On KDE Wayland a temporary KWin script places it above the tray icon; on X11 it
    places itself.
  - **Look:** colours come from the KDE colour scheme or the desktop portal, and the font
    is the system UI font.
  - **Keymap editor:** keycaps show all three layers. Keys are selected by clicking or by
    pressing them on the chatpad, and changes save and apply instantly over D-Bus.
- **Daemon D-Bus service** `io.github.NewAbbreviations950.X360Chatpad`: `Reload`, `Status`,
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
- **Publish:** create the GitHub repo `NewAbbreviations950/X360ChatDriver`, push, and tag
  `v0.1.0` so the one-line installer works.
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
- **Headset audio:** the odd-numbered interfaces (protocol 0x82, interrupt endpoints) carry
  the headset. The format is undocumented. Research it, then expose a PipeWire sink and
  source.
  - No headset on hand yet (expected in a few weeks). The racing wheel has a headset jack too.
  - First test once it's here: read registers 0–4 (`docs/protocol.md`, register reads) on the
    controller and the wheel with the headset unplugged and plugged in. Register 0 reads `20`
    on a controller with the chatpad and `03` on the wheel, so it may be an attachment field.
  - The dump tool only reads the controller interfaces; make it log the audio endpoints too,
    so traffic there shows up during probes.
- **Unknown packets:** `00 f8 0N` (seen while the chatpad wasn't handshaked, and once from the
  wheel). The `00 0f` link-info packet carries register/value pairs (see the register reads in
  `docs/protocol.md`); only the battery (register 1) and device type (register 5) are known.
- **Non-ASCII chatpad legends** (€, š, ¨, «, »): these work only if the layout can type
  them. A compose/Unicode fallback could cover the rest.
- **Packaging extras:** aarch64 builds, AUR/OBS packages, maybe a Flatpak for the GUI only.
- **Power off controller** from the tray or popup (`00 00 08 c0`); needs a D-Bus method.
- **Racing wheel force feedback** (Xbox 360 Wireless Racing Wheel, WRW01). Games' effects
  are passed to the wheel through the receiver.
  - Done: a `p` probe command in `x360chatpad-dump` with sweep scripts (`docs/probes/`).
    The daemon can expose a slot as a joystick-style virtual wheel
    (`X360CHATPAD_WHEEL_SLOTS`) that accepts every evdev effect type and logs it
    (`X360CHATPAD_FF_LOG`). Effects are normalised in `crates/daemon/src/ffb.rs`.
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
    - Next: map `kk` = `0b` (its status reacts to payload bits, so it may be a device
      control such as "enable actuators") and `0c`, then movement tests of each `0b` value
      before and after `09`.
    - Then movement tests on the accepted space: the `09` operation and loop fields, duration
      and gain units, block control `05`/`06`/`07`, effect indices other than 1, other
      effect types (a spring condition is easy to feel), and the ramp (`02`/`05`).
    - Find what `0c` frees (the memory still fills) and what `0b` replies mean.
    - Write the encoder (`ffb::Op` → packets) once forces are found.
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
