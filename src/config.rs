//! Persistent settings, stored as JSON in %APPDATA%\RSnap\config.json.

use crate::model::{PRESETS, Rgb};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpg,
    Webp,
}

impl ImageFormat {
    pub const ALL: [ImageFormat; 3] = [ImageFormat::Png, ImageFormat::Jpg, ImageFormat::Webp];

    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpg => "jpg",
            ImageFormat::Webp => "webp",
        }
    }
}

/// Modifier flags use the RegisterHotKey values: Alt 1, Ctrl 2, Shift 4, Win 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub save_dir: Option<PathBuf>,
    pub format: ImageFormat,
    pub autostart: bool,
    pub hotkey: Hotkey,
    pub color: Rgb,
    pub thickness: f32,
    /// Whether new rectangles, circles and ellipses are solid instead of outlined.
    pub fill_shapes: bool,
    pub font: String,
    pub font_weight: u32,
    pub font_size: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            save_dir: None,
            format: ImageFormat::Png,
            autostart: true,
            hotkey: Hotkey {
                modifiers: 2,
                vk: 0x70,
            },
            color: PRESETS[0],
            thickness: 4.0,
            fill_shapes: false,
            font: "Microsoft JhengHei UI".to_string(),
            font_weight: 400,
            font_size: 24.0,
        }
    }
}

thread_local! {
    static CONFIG: RefCell<Config> = RefCell::new(load());
}

fn path() -> PathBuf {
    let dir = std::env::var_os("RSNAP_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("RSnap")))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join("config.json")
}

fn load() -> Config {
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn get() -> Config {
    CONFIG.with(|c| c.borrow().clone())
}

/// Applies a change and writes it to disk straight away, so a crash cannot lose it.
pub fn update(change: impl FnOnce(&mut Config)) {
    let snapshot = CONFIG.with(|c| {
        let mut config = c.borrow_mut();
        change(&mut config);
        config.clone()
    });
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec_pretty(&snapshot) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let c = Config::default();
        assert_eq!(c.format, ImageFormat::Png);
        assert!(c.autostart);
        assert_eq!(
            c.hotkey,
            Hotkey {
                modifiers: 2,
                vk: 0x70
            }
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let c: Config = serde_json::from_str(r#"{"format":"webp"}"#).unwrap();
        assert_eq!(c.format, ImageFormat::Webp);
        assert!(c.autostart);
        assert_eq!(c.font_size, 24.0);
        assert!(!c.fill_shapes);
    }

    #[test]
    fn round_trips_through_json() {
        let c = Config {
            format: ImageFormat::Jpg,
            thickness: 9.0,
            ..Config::default()
        };
        let parsed: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(parsed, c);
    }
}
