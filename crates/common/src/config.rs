//! User settings in `~/.config/x360truewireless/config.toml`. Every field is optional:
//!
//! ```toml
//! # Keyboard layout the chatpad should type for. Detected from the desktop if unset.
//! # Useful on Steam Deck game mode, which uses its own (US) layout.
//! layout = "no"
//! variant = ""
//!
//! # Remapped keys, per layer (normal, green, orange). Values are a single character,
//! # an evdev key name such as "KEY_F5", or "" to disable the key.
//! [keys.green]
//! E = "KEY_F5"
//!
//! # How a racing wheel shows up: "gamepad" (default) or "joystick". Applies when it connects.
//! wheel_layout = "joystick"
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::keymap::Overrides;

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wheel_layout: Option<WheelLayout>,
    #[serde(skip_serializing_if = "Overrides::is_empty")]
    pub keys: Overrides,
}

/// How a racing wheel shows up to games. Both keep the steering, pedals and force feedback.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WheelLayout {
    /// Gamepad buttons and trigger axes, so SDL, Steam and Proton treat it as a gamepad and
    /// game menus work with it.
    #[default]
    Gamepad,
    /// A plain joystick, for games that skip gamepads when they look for a wheel.
    Joystick,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        Some(config_dir()?.join("x360truewireless/config.toml"))
    }

    /// Load the config. A missing file gives the defaults; a broken one is an error.
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("reading {}: {e}", path.display())),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no home directory")?;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        // Write then rename, so the daemon never reads a half-written file.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("saving {}: {e}", path.display()))
    }

    /// The configured layout, if one is set.
    pub fn layout_name(&self) -> Option<(String, String)> {
        let layout = self.layout.clone().filter(|l| !l.is_empty())?;
        Some((layout, self.variant.clone().unwrap_or_default()))
    }
}

pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(Into::into)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Layer;

    #[test]
    fn roundtrip() {
        let text = "layout = \"no\"\n\n[keys.green]\nE = \"KEY_F5\"\n";
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(config.layout.as_deref(), Some("no"));
        assert_eq!(config.keys[&Layer::Green]["E"], "KEY_F5");
        assert_eq!(config.wheel_layout, None);
        let back: Config = toml::from_str(&toml::to_string_pretty(&config).unwrap()).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn wheel_layout() {
        let config: Config = toml::from_str("wheel_layout = \"joystick\"").unwrap();
        assert_eq!(config.wheel_layout, Some(WheelLayout::Joystick));
        assert!(toml::from_str::<Config>("wheel_layout = \"wheel\"").is_err());
    }

    #[test]
    fn empty_is_default() {
        assert_eq!(toml::from_str::<Config>("").unwrap(), Config::default());
    }
}
