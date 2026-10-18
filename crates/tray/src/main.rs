//! x360truewireless-tray: tray icon for the X360TrueWireless driver, and (with `--settings`) its
//! settings window. The settings window runs as its own process because a GUI event loop can
//! only be created once per process, and the tray must stay alive after it closes.

mod client;
mod icon;
mod placement;
mod settings;
mod theme;
mod tray;

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--settings") => settings::run(
            std::env::args()
                .nth(2)
                .and_then(|a| placement::Anchor::parse(&a)),
        ),
        None => match tray::run() {
            Ok(()) => Ok(()),
            Err(e) => {
                // Desktops without a tray (plain GNOME) still have the settings window in the
                // app menu, so this isn't fatal.
                eprintln!("{e}; open \"X360TrueWireless Settings\" from the app menu instead");
                Ok(())
            }
        },
        Some(other) => anyhow::bail!("unknown argument {other:?} (expected --settings)"),
    }
}
