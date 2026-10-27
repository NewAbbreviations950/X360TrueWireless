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
- Renamed to X360TrueWireless and released as v0.2.0 (2026-10-20), after pre-releases
  rc1–rc9 were tested on this machine and the Steam Deck.
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
- **Test on Steam Deck** (desktop + game mode) and **Kubuntu**:
  - Steam sees exactly one pad per controller.
  - Chatpad types in game mode.
  - Check the layout in game mode: gamescope uses US unless configured.
  - Check that the udev rule in `/etc` survives a SteamOS update.
- **Test the tray and popup on other desktops:** GNOME, both with and without the
  AppIndicator extension (without it the popup should open from the app menu), plus
  Kubuntu, Deck desktop mode and X11. Check that the popup is placed next to the tray.
- **Release build size:** the tray binary is about 12 MB, and could be trimmed if that matters.

- **Windows port: test on the Windows laptop** (code on the `windows-port` branch, built and
  tested by CI on Windows; nothing has run on Windows yet). Linux-only code is behind
  `crates/daemon/src/platform/` and the tray's `tray/` module. Decisions: pads through
  ViGEmBus, chatpad typing with `SendInput` (scancodes through the focused window's layout,
  text for characters it lacks), the wheel as a vJoy joystick with force feedback (HID PID
  reports → `ffb::Op` in `pid.rs`, then the existing encoder), the tray↔driver link as D-Bus
  peer to peer over a Unix socket, and an Inno Setup installer. Spoofing a G920/TMX identity
  as on Linux would need our own signed kernel driver (paid EV certificate + Microsoft
  attestation), so games that only accept known wheels (SnowRunner) won't see the vJoy wheel.
  WinUSB goes on the receiver through `packaging/windows/receiver-driver.ps1` (Zadig's method
  with built-in tools: a local certificate, a signed catalog, a forced driver update), since
  building libwdi's `wdi-simple` needs the old WDK 8 redistributables. To check, in order:
  1. The CI build's `x360truewireless-dump.exe` with WinUSB put on the receiver by Zadig:
     all four interfaces claim and packets flow (nusb with WinUSB on the whole device).
  2. `receiver-driver.ps1` as administrator: the self-signed catalog is accepted and the
     forced update replaces Windows' Xbox driver; `-Uninstall` gives it back.
  3. Pads in `joy.cpl` and Steam, rumble, the player LED, Guide+A quick menu.
  4. Chatpad typing on the laptop's layout and after switching layout (AltGr chords, dead
     keys, text fallback), the Caps light (`GetKeyState` from a background thread).
  5. Tray: icon colour, menu, popup placement, settings saving, the socket link
     (`ChatpadKey` signals for the key picker), low-battery toast.
  6. Installer: ViGEmBus silent switches (`/exenoui /qn`), vJoy (`/VERYSILENT`),
     `vJoyConfig` creating devices 1–4 with force feedback, autostart, upgrade, uninstall.
  7. Wheel: shows in `joy.cpl` with force feedback; DiRT 4 and Wreckfest natively; check
     the force direction (`CONSTANT_SIGN` and `pid.rs`'s polar angle).
  Later: winget, code signing, an `.ico` for the setup.

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
      `CONSTANT_SIGN`; evdev direction `0x4000` currently pushes left). In SnowRunner, steer
      into a turn while driving: the rim should be pulled back towards centre; if it's pushed
      further into the turn, flip the sign. `docs/probes/scripts/ff-test.py` checks the
      driver side on its own.
    - **First Deck test (rc1, SnowRunner):** with Steam Input on, the game only got Steam's
      virtual pad (after Steam's wheel wizard), so no forces; with Steam Input forced off for
      the game it saw no wheel input at all, menus included (the wheel was a plain joystick,
      with no gamepad mapping). The wheel never centred at rest: SDL turns the autocentre off
      on open and Steam keeps it open. Fixed for rc2: the autocentre stays on until a game
      uploads a force, and the wheel has a gamepad layout by default (SDL maps it as a
      gamepad, with haptics). rc3 adds the popup's Racing wheel modes (`wheel_mode`): sim
      wheel (that layout), Xbox controller (a plain 360 pad with rumble and the native
      autocentre, for games without wheel support) and joystick (the old layout); switching
      replaces the device at once. To test: Steam Input off for generic controllers only,
      then the menus and forces in SnowRunner, with `X360TRUEWIRELESS_FF_LOG`. If the game
      skips the wheel as a gamepad, try joystick. Also check the Xbox controller mode with
      Steam Input on, and switching with the wheel connected.
    - **rc3 result:** sim wheel (gamepad layout) worked for buttons and steering but SnowRunner
      showed no wheel and no forces; the joystick layout wasn't listed either. Cause, from
      Proton's source (`winebus.sys/bus_sdl.c`, `dinput/joystick_hid.c`): Proton presents a
      device as a DirectInput driving device (with all PID effects) only when SDL's joystick
      type is wheel, and SDL decides that from a fixed VID/PID list
      (`SDL_JOYSTICK_WHEEL_DEVICES` adds more). Otherwise it's a gamepad (an SDL-mapped
      device, also XInput) or a plain joystick. rc4: the sim wheel uses `1209:ffb0`
      (OpenFFBoard, in SDL's list since 2.30), and the joystick mode is gone (`joystick`
      still parses, as sim wheel). To test: whether SnowRunner lists the wheel, binds its
      buttons and plays forces.
    - **rc4 result (Deck, over SSH):** SnowRunner lists the wheel and binds its buttons, but
      never creates an effect: its Proton log (`WINEDEBUG=+dinput`) shows it enumerating
      constant force and the force-feedback axes, then nothing. **DiRT 4 works:** constant
      force, spring, damper and friction, with forces up to ~0.9 while driving. Logging the
      steering with each constant force showed `CONSTANT_SIGN = -1` is right (force opposes
      steering in 307 of 385 corner samples; flipped, 263 of 329 pushed into the corner).
      Proton restarts every effect each frame (~60/s), so a re-play of a playing endless
      effect and a parameter block that rounds to the same bytes are no longer sent. Fixed
      too: a wheel already connected when the driver starts sometimes missed the device-type
      request (now retried every 250 ms for 3 s, and a late answer replaces the gamepad
      guess), and wheels show no battery (on AC without batteries they report empty). Next:
      why SnowRunner skips generic wheels (profiles by VID/PID?), a mode switch for game
      mode (a wheel button combo), the periodic block.
    - **rc6: Logitech G920 mode (default).** DiRT 4 had to be rebound every session as a
      generic wheel, and SnowRunner ignores generic wheels, so the wheel can present itself
      as a G920 (046d:c262, the Xbox-layout Logitech wheel; in SDL's wheel list, and the one
      wheel Proton special-cases: its four axes become steering, throttle, brake and clutch
      usages, and it's typed as a dual-pedal wheel). Layout from DiRT 4's
      `input/actionmaps/lg_g920.xml`: buttons 1–11 = A, B, X, Y, right paddle, left paddle,
      Menu, View, RSB, LSB, Xbox; throttle Y and brake Z rotation, both reading full when
      released ("negative"). Proton's dinput (`joystick_hid.c`) renames a G920's HID axes
      to match Windows: Z becomes Z rotation "Brake" and Z rotation a slider "Clutch", so
      the brake goes on `ABS_Z` (rc6 had it on `ABS_RZ`, and DiRT saw it as the clutch). To test: DiRT 4 and SnowRunner pick it up without binding, the
      pedals and paddles work, and forces play. Caveat: G920 profiles assume 900° of
      rotation and the 360 wheel has about 270°. Later: older wheels with fewer buttons and
      pedals (Driving Force GT, G25/G27) if a game needs them.
    - **rc7: Thrustmaster TMX mode (default), replacing the generic wheel.** Picked over
      the T128, which is physically closer (two pedals, Xbox layout) but newer than most
      games' profiles (DiRT 4 has none) and missing from SDL's wheel list, so Proton would
      present it as a plain joystick. The TMX (044f:b67f, 2015) has two pedals and the Xbox
      layout too, is in SDL's list and has a DiRT 4 profile. Its HID descriptor, from the
      USB capture in emtek995/TMX-driver's `wheel.log`: X steering (16-bit), Y brake, Z
      rotation throttle, slider clutch (10-bit), 13 buttons, a hat. Proton gives non-G920
      devices X, Y, Z, X rotation, Y rotation, Z rotation, slider in SDL axis order, so the
      wheel declares `ABS_X`, `ABS_Y`, three idle axes, `ABS_RZ` and `ABS_THROTTLE`.
      Buttons 1–13 from what DiRT 4's `tm_tmx.xml` binds, matched against its G920 profile:
      left paddle, right paddle, X, LSB, A, two unused, Menu, B, Y, RSB, View, Xbox. LSB,
      RSB, View and Xbox are guesses (DiRT's TMX binds look back to button 4 and select to
      button 12); check them in DiRT's controls screen. The old `generic` setting now
      means the TMX.
    - **SnowRunner works on Proton 6.3 in G920 mode**, with DirectInput and its "Logitech
      G923 Xbox layout" preset: right controls and force feedback. TMX mode gets force
      feedback too, but no preset maps it: the game has no TMX preset, its TX preset acts
      as if the left-paddle modifier is held (Thrustmaster or DirectInput input), and T248
      (Xbox) gets the face buttons wrong. An earlier "TX works" was a controller's input.
      So the G920 is the default again, and the TMX the second choice.
    - **Wreckfest (Proton 11, G920 mode):** menus scrolled up-right after touching the
      D-pad, and binding picked random controls. The D-pad itself was fine (evdev events
      recorded, and Wine's joy.cpl showed it centring); the cause is the pedal axes, which
      rest at their maximum like a real G920's: Wreckfest had menu navigation on them, and
      Wine shows axes centred until the first report, so the first input looks like the
      pedals moving. Pressing the pedals once, then rebinding menu navigation to the D-pad,
      fixed it (real G920s hit the same). Force feedback works on Proton 11, the strongest
      of the three games so far. Switching modes mid-game loses force feedback
      (the game sets effects up on the wheel it found at launch). A switch also once killed
      slot 1's thread with EINVAL from a withdrawn force-feedback request; such requests are
      now skipped, and a failed slot restarts. Still to look at: SnowRunner forgets the
      wheel input type between launches (another controller connected, or the wheel's
      enumeration order changing under Proton 6.3?). On Proton 7 and later it calls `EnumObjects(DIDFT_FFACTUATOR | DIDFT_AXIS)`,
      which Wine's dinput rejects since the Wine 7 rewrite (Wine 6 filtered by type
      instead), so it never creates effects; checked in Proton 7.0/8.0/11 sources and on
      the Deck with 8, 11 and Experimental. On 6.3 it keeps a friction effect and a constant
      force, each re-uploaded thousands of times a drive, and the force opposed the steering
      in 90% of turning samples. Its menu test plays a 5 s sine, which the encoder can't play
      yet (periodic block). A Wine bug report is drafted outside the repo.
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
