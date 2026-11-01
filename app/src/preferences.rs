use crate::{i18n::Language, theme::Appearance};
use serde_json::json;
use std::{io::Write, path::Path};

/// The main window as the user last left it, in logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl WindowSize {
    pub const DEFAULT: [f32; 2] = [1280.0, 820.0];
    pub const MIN: [f32; 2] = [1024.0, 700.0];
    /// Larger than any real display; rejects corrupt values without guessing a screen size.
    const MAX: f32 = 16384.0;

    pub fn inner_size(self) -> [f32; 2] {
        [self.width, self.height]
    }

    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let [width, height] = size_from_json(value, Self::MIN)?;
        Some(Self {
            width,
            height,
            maximized: value["maximized"].as_bool().unwrap_or(false),
        })
    }
}

/// The settings window's content size, in logical points.
pub const SETTINGS_DEFAULT: [f32; 2] = [660.0, 340.0];
pub const SETTINGS_MIN: [f32; 2] = [560.0, 280.0];

/// Reads `width` and `height`, growing them to `min`; `None` when either is unusable.
fn size_from_json(value: &serde_json::Value, min: [f32; 2]) -> Option<[f32; 2]> {
    let size = |key: &str, min: f32| {
        let size = value[key].as_f64()? as f32;
        (size.is_finite() && size <= WindowSize::MAX).then(|| size.max(min))
    };
    Some([size("width", min[0])?, size("height", min[1])?])
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Preferences {
    pub language: Language,
    pub appearance: Appearance,
    pub window: Option<WindowSize>,
    pub settings_window: Option<[f32; 2]>,
}

impl Preferences {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.to_string()),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if value["version"] != 1 || !value.is_object() {
            return Err("Unsupported UI settings version".into());
        }
        Ok(Self {
            language: value["language"]
                .as_str()
                .and_then(Language::from_code)
                .unwrap_or_default(),
            appearance: value["appearance"]
                .as_str()
                .and_then(Appearance::from_code)
                .unwrap_or_default(),
            window: WindowSize::from_json(&value["window"]),
            settings_window: size_from_json(&value["settings_window"], SETTINGS_MIN),
        })
    }

    pub fn save(self, path: &Path) -> Result<(), String> {
        let directory = path.parent().ok_or("Invalid UI settings directory")?;
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
        let mut value = json!({
            "version": 1,
            "language": self.language.code(),
            "appearance": self.appearance.code(),
        });
        if let Some(window) = self.window {
            value["window"] = json!({
                "width": window.width,
                "height": window.height,
                "maximized": window.maximized,
            });
        }
        if let Some([width, height]) = self.settings_window {
            value["settings_window"] = json!({"width": width, "height": height});
        }
        let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.error.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_size_round_trips_and_rejects_unusable_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-settings.json");
        let window = WindowSize {
            width: 1500.0,
            height: 900.5,
            maximized: true,
        };
        Preferences {
            window: Some(window),
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        assert_eq!(Preferences::load(&path).unwrap().window, Some(window));
        let load = |window: serde_json::Value| {
            std::fs::write(&path, json!({"version": 1, "window": window}).to_string()).unwrap();
            Preferences::load(&path).unwrap().window
        };
        // Too small grows to the minimum; settings files without a size keep the default.
        assert_eq!(
            load(json!({"width": 10, "height": 20})),
            Some(WindowSize {
                width: 1024.0,
                height: 700.0,
                maximized: false
            })
        );
        assert_eq!(load(json!({"width": 1e9, "height": 800})), None);
        assert_eq!(load(json!({"width": "wide", "height": 800})), None);
        assert_eq!(load(json!(null)), None);
    }

    #[test]
    fn settings_window_size_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-settings.json");
        Preferences {
            settings_window: Some([700.0, 420.5]),
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        assert_eq!(
            Preferences::load(&path).unwrap().settings_window,
            Some([700.0, 420.5])
        );
        let load = |size: serde_json::Value| {
            std::fs::write(
                &path,
                json!({"version": 1, "settings_window": size}).to_string(),
            )
            .unwrap();
            Preferences::load(&path).unwrap().settings_window
        };
        assert_eq!(load(json!({"width": 10, "height": 20})), Some(SETTINGS_MIN));
        assert_eq!(load(json!({"width": "wide", "height": 300})), None);
    }
}
