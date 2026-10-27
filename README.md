# X360TrueWireless

Full support for Xbox 360 wireless accessories on Linux and Windows through the official
Xbox 360 Wireless Receiver: controllers **with their chatpad**, and the **Wireless Racing Wheel with
force feedback**. It works on any desktop distro and on the Steam Deck, and it plays nicely
with Steam Input.

The kernel's `xpad` driver supports the controller but ignores the chatpad, and drives the
wheel only as a gamepad with rumble. This driver takes the receiver over from `xpad` (and
gives it back when stopped). It exposes:

- **one virtual Xbox 360 pad per controller**, with buttons, sticks, triggers, rumble and
  the player LED. Steam and games see a normal 360 controller.
- **one virtual keyboard** that all chatpads type into, following your keyboard layout.
- **a virtual racing wheel** for the Wireless Racing Wheel, with force feedback
  (experimental, see below).

Planned: headset audio, both the wired headset plugged into a controller (or the wheel) and
the Xbox 360 Wireless Headset, once the hardware is on hand to work out the protocol.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/NewAbbreviations950/X360TrueWireless/main/install.sh | bash
```

This installs to `~/.local/bin` and runs as a systemd user service that starts when you log
in, including Steam Deck game mode. The installer asks for `sudo` **once**, to add a udev
rule that gives your user access to the receiver. On Steam Deck, set a password first with
`passwd` in Konsole if you've never done so.

To update, run the same command again. To remove:

```sh
curl -fsSL https://raw.githubusercontent.com/NewAbbreviations950/X360TrueWireless/main/install.sh | bash -s -- --uninstall
```

From a checkout, `./install.sh --local` builds and installs the working tree (needs Rust).

This project used to be called x360chatpad. The installer replaces an install under the old
name (service, udev rule and menu entries) and keeps your settings.

## Windows (beta)

Download `X360TrueWireless-Setup-x86_64.exe` from the
[latest release](https://github.com/NewAbbreviations950/X360TrueWireless/releases) and run it
(Windows 10 1809 or later, 64-bit). Plug the receiver in first. Windows SmartScreen warns about
the setup because it isn't code-signed; choose "More info", then "Run anyway".

The setup installs:

- the driver and the tray app, both started at every sign-in;
- [ViGEmBus](https://github.com/nefarius/ViGEmBus) for the virtual Xbox 360 pads, and
  [vJoy](https://github.com/BrunnerInnovation/vJoy) for virtual racing wheels with force
  feedback, unless they're already installed. vJoy devices 1–4 are set up as wheels;
- Windows' WinUSB driver on the receiver, in place of Windows' own Xbox driver, the way
  [Zadig](https://zadig.akeo.ie/) does it: a driver package signed with a certificate made on
  your PC (its private key is deleted right after) and trusted on your PC only.

Windows binds the receiver per USB port, so if you move it to another port, open
**Set up receiver** from the Start menu or the tray menu.

Differences from Linux:

- The chatpad types through the keyboard layout of the window in front, and switches with
  it. Characters the layout has no key for are typed as text.
- Windows doesn't let programs type into apps running as administrator.
- The racing wheel is a vJoy joystick (`1234:BEAD`), not a Logitech or Thrustmaster wheel:
  making up another wheel's identity on Windows needs a kernel driver with a paid signature.
  Games that bind any DirectInput wheel work, with force feedback; games that only accept
  wheels they know (SnowRunner, for one) won't list it. The "Xbox controller" wheel mode works
  everywhere.
- Logs are in `%LOCALAPPDATA%\X360TrueWireless\daemon.log`, and the config file is
  `%APPDATA%\x360truewireless\config.toml` (its `layout` setting is ignored).

Uninstalling (Settings → Apps) gives the receiver back to Windows' Xbox driver. ViGEmBus and
vJoy stay installed, since other programs use them too; remove them from Settings → Apps if
you don't need them.

## Using the chatpad

| Keys | Result |
|---|---|
| Shift, Green, Orange | one-shot: tap it, then press a key (holding it works too); tap again to cancel |
| Green / Orange + key | the green / orange character printed on the key |
| Shift + Orange | Caps Lock on/off |
| People | open Steam friends & chat |
| Green + People | Steam Quick Access menu (Guide + A) |

Characters are typed through your desktop's keyboard layout, so symbols come out right
on non-US layouts too. Characters your layout can't type are skipped.

## Tray icon and settings

A controller icon in the system tray shows the driver status and the connected players.
Clicking it opens a small popup with a box for each connected slot, showing what's
connected (a controller, a controller with a chatpad, or a wheel) and its battery:
- **Remap keys** (controllers with a chatpad): click a key, or press it on the chatpad, then
  type a character or pick a special key, on each layer. The keys apply to every chatpad,
  and changes apply immediately.
- **Wheel mode** (racing wheels): how games see the wheel, see below.
- The gear button opens settings: the keyboard layout (automatic, or a fixed one), and
  restarting the driver.

The popup is also in your app menu as **X360TrueWireless Settings**. That's the way in on GNOME
unless you install the "AppIndicator and KStatusNotifierItem Support" extension.

## Racing wheel (experimental)

The Microsoft Xbox 360 Wireless Racing Wheel's force-feedback protocol was never documented;
it was worked out for this driver (see [docs/protocol.md](docs/protocol.md)). Games get
constant force, spring, damper, inertia and friction effects, and rumble. Periodic and ramp
effects are accepted but don't play yet. Keep the wheel on its AC adapter: force feedback
needs it.

Wheels are recognised when they connect, so there's nothing to set up, and any mix of up to
four wheels and controllers works. The wheel centres itself whenever no game is driving it.

Choose how wheels show up in the wheel's box in the popup. The choice applies to every
wheel, and right away, so switch before starting a game:
- **Logitech G920** (default): games see a Logitech G920, Logitech's Xbox-layout wheel, with
  force feedback. Most racing games that know any wheel have a G920 profile and set up the
  buttons and pedals themselves.
- **Thrustmaster TMX**: the same, shown as Thrustmaster's Xbox-layout wheel, for games that
  have a TMX profile but none for the G920.
- **Xbox controller**: an ordinary Xbox 360 controller, for games without wheel support.
  Steering is the left stick, and you get rumble and the wheel's own centring.

For both wheel modes, turn Steam Input off for generic controllers in Steam's controller
settings. Steam counts the wheel as one, and your Xbox controllers stay on Steam Input. With
it on, games get Steam's virtual pad as well as the wheel: it has no force feedback, and the
pedals, which read fully one way when released, look like sticks held over to one side
(stuck menu navigation in Wreckfest, for one). A game's own "Disable Steam Input" setting
works too, but it turns Steam Input off for every controller in that game, Xbox ones
included.

Tested on the Steam Deck with DiRT 4, Wreckfest and SnowRunner. SnowRunner needs Proton 6.3 for force
feedback (choose it under the game's Properties, Compatibility): Proton 7 and later reject the
way it asks for the wheel's force-feedback axis, so it sends no forces. Set the wheel to
**Logitech G920** in the popup before starting the game (SnowRunner sets up force feedback
only at launch), and in the game's controls choose DirectInput with the "Logitech G923
(Xbox)" layout. SnowRunner has no TMX preset, and its other Thrustmaster presets map the
buttons wrong.

In Wreckfest, menus may scroll by themselves and binding may pick the wrong control, because
it reads the released pedals (which a G920 reports at the end of their travel) as held
directions. Press each pedal once after starting the game, then in the controls unbind any
menu navigation on the pedal axes and bind it to the D-pad again. Real G920s need the same.

## Configuration

`~/.config/x360truewireless/config.toml` (optional):

```toml
# Keyboard layout to type for. Detected from KDE, GNOME or the system default if unset.
# Steam Deck game mode uses a US layout regardless of desktop settings, so set this there
# if your desktop layout differs.
layout = "no"
variant = ""

# Remapped keys per layer (normal, green, orange). Use a single character, an evdev key
# name such as "KEY_F5", or "" to disable the key. Key names: D1-D0, Q-P, A-L, Comma,
# Z-M, Period, Enter, Left, Space, Right, Backspace.
[keys.green]
E = "KEY_F5"
```

`wheel_mode` (a top-level setting, above any `[keys…]` section) is the popup's Racing wheel
choice: `"logitech"` (default), `"thrustmaster"` or `"controller"`.

The settings popup edits this file for you. After editing it by hand, choose "Reload" in
the tray menu, or run `systemctl --user restart x360truewireless`.

## Troubleshooting

- Logs: `journalctl --user -u x360truewireless -f`
- "open receiver" errors: the udev rule is missing. Rerun the installer, or replug the receiver.
- Stopping the service (`systemctl --user stop x360truewireless`) hands the receiver back to
  `xpad`, so the controller keeps working without the chatpad.

## Development

- `cargo test --workspace` runs packet-parsing, keymap and layout tests.
- Crates: `proto` (USB packets), `common` (config, keymap, D-Bus names), `daemon`, `tray`.
  What differs between Linux and Windows lives in `crates/daemon/src/platform/` and in the
  tray's `tray/` module. `rustup target add x86_64-pc-windows-gnu`, then
  `cargo clippy --workspace --target x86_64-pc-windows-gnu`, checks the Windows side from
  Linux; CI builds and tests it on Windows. The installer is
  `packaging/windows/x360truewireless.iss` (Inno Setup).
- `x360truewireless-dump` (built with the daemon) hex-dumps receiver traffic and sends packets
  typed on stdin. It's useful for protocol work; stop the service first.
- Protocol notes: [docs/protocol.md](docs/protocol.md). Status and plans: [docs/ROADMAP.md](docs/ROADMAP.md)
