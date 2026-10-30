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
        let size = |key: &str, min: f32| {
            let size = value[key].as_f64()? as f32;
            (size.is_finite() && size <= Self::MAX).then(|| size.max(min))
        };
        Some(Self {
            width: size("width", Self::MIN[0])?,
            height: size("height", Self::MIN[1])?,
            maximized: value["maximized"].as_bool().unwrap_or(false),
        })
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Preferences {
    pub language: Language,
    pub appearance: Appearance,
    pub window: Option<WindowSize>,
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
}
