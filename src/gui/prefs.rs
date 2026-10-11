//! GUI preferences in `~/.config/monitor-oc/gui.conf`, as `key=value` lines
//! like monitor-oc's other state files.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

pub const MIN_SCALE: f32 = 0.75;
pub const MAX_SCALE: f32 = 2.5;

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// UI zoom; `None` until the user picks one (then the app chooses by screen).
    pub scale: Option<f32>,
    pub dark: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            scale: None,
            dark: true,
        }
    }
}

impl Prefs {
    pub fn parse(text: &str) -> Prefs {
        let get = |key: &str| {
            text.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == key).then(|| v.trim().to_string())
            })
        };
        Prefs {
            scale: get("scale")
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|s| s.is_finite())
                .map(|s| s.clamp(MIN_SCALE, MAX_SCALE)),
            dark: get("theme").as_deref() != Some("light"),
        }
    }

    pub fn render(&self) -> String {
        let mut out = format!("theme={}\n", if self.dark { "dark" } else { "light" });
        if let Some(scale) = self.scale {
            out += &format!("scale={scale:.2}\n");
        }
        out
    }

    fn path() -> Option<PathBuf> {
        let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
        };
        Some(config.join("monitor-oc/gui.conf"))
    }

    pub fn load() -> Prefs {
        Prefs::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .map(|t| Prefs::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Prefs::path().context("HOME is not set")?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        fs::write(&path, self.render()).with_context(|| format!("writing {}", path.display()))
    }
}

/// Default UI zoom: handheld screens in Game Mode are 1080p at about 7", so
/// everything is drawn larger there (1.5x makes 1080p feel like the Steam
/// Deck's 1280x800 at 1x).
pub fn default_scale(game_mode: bool, screen_height_px: Option<f32>) -> f32 {
    match screen_height_px {
        Some(h) if game_mode && h >= 1000.0 => 1.5,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let p = Prefs {
            scale: Some(1.25),
            dark: false,
        };
        assert_eq!(Prefs::parse(&p.render()), p);
        assert_eq!(Prefs::parse(""), Prefs::default());
        assert_eq!(Prefs::parse("scale=9\n").scale, Some(MAX_SCALE));
        assert_eq!(Prefs::parse("scale=abc\n").scale, None);
    }

    #[test]
    fn handhelds_get_a_larger_default() {
        assert_eq!(default_scale(true, Some(1080.0)), 1.5);
        assert_eq!(default_scale(true, Some(800.0)), 1.0);
        assert_eq!(default_scale(false, Some(1080.0)), 1.0);
        assert_eq!(default_scale(true, None), 1.0);
    }
}
