use crate::{i18n::Language, theme::Appearance};
use serde_json::json;
use std::{io::Write, path::Path};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Preferences {
    pub language: Language,
    pub appearance: Appearance,
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
        })
    }

    pub fn save(self, path: &Path) -> Result<(), String> {
        let directory = path.parent().ok_or("Invalid UI settings directory")?;
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(&json!({
            "version": 1,
            "language": self.language.code(),
            "appearance": self.appearance.code(),
        }))
        .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.error.to_string())?;
        Ok(())
    }
}
