//! Desktop actions triggered from the chatpad.

use std::process::{Command, Stdio};
use std::thread;

const STEAM_CHAT_URL: &str = "steam://open/friends";

/// Open Steam's friends & chat window. Uses the native `steam` command when there is one
/// (Steam Deck, distro packages), otherwise `xdg-open`, which also reaches Flatpak Steam.
pub fn open_steam_chat() {
    thread::spawn(|| {
        let attempts: [(&str, &[&str]); 2] = [
            ("steam", &[STEAM_CHAT_URL]),
            ("xdg-open", &[STEAM_CHAT_URL]),
        ];
        for (program, args) in attempts {
            let spawned = Command::new(program)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match spawned {
                Ok(mut child) => {
                    // Reap it so it doesn't linger as a zombie.
                    let _ = child.wait();
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    eprintln!("opening Steam chat with {program}: {e}");
                    return;
                }
            }
        }
        eprintln!("opening Steam chat: neither `steam` nor `xdg-open` is installed");
    });
}
