use crate::{
    i18n::{Language, Translator},
    theme::Appearance,
};
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
pub const SETTINGS_DEFAULT: [f32; 2] = [660.0, 400.0];
pub const SETTINGS_MIN: [f32; 2] = [560.0, 280.0];

/// Reads `width` and `height`, growing them to `min`; `None` when either is unusable.
fn size_from_json(value: &serde_json::Value, min: [f32; 2]) -> Option<[f32; 2]> {
    let size = |key: &str, min: f32| {
        let size = value[key].as_f64()? as f32;
        (size.is_finite() && size <= WindowSize::MAX).then(|| size.max(min))
    };
    Some([size("width", min[0])?, size("height", min[1])?])
}

/// How much each host card in the sidebar shows besides its name.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum HostDetail {
    #[default]
    Full,
    NameOnly,
    Custom,
}

impl HostDetail {
    pub const ALL: [Self; 3] = [Self::Full, Self::NameOnly, Self::Custom];

    fn code(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::NameOnly => "name",
            Self::Custom => "custom",
        }
    }

    pub fn label(self, t: Translator) -> &'static str {
        match self {
            Self::Full => t.get("host_detail.full"),
            Self::NameOnly => t.get("host_detail.name_only"),
            Self::Custom => t.get("host_detail.custom"),
        }
    }
}

/// How tightly the sidebar packs its hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Density {
    /// A card per host.
    #[default]
    Comfortable,
    /// One short line per host, like a file tree.
    Compact,
}

impl Density {
    pub const ALL: [Self; 2] = [Self::Comfortable, Self::Compact];

    fn code(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Compact => "compact",
        }
    }

    pub fn label(self, t: Translator) -> &'static str {
        match self {
            Self::Comfortable => t.get("density.comfortable"),
            Self::Compact => t.get("density.compact"),
        }
    }
}

/// How the batch table orders its hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum BatchOrder {
    /// The sidebar's order, under the same group headings.
    Sidebar,
    /// Most pending updates first.
    #[default]
    Pending,
    /// Most recently checked or upgraded first.
    Recent,
    Name,
}

impl BatchOrder {
    pub const ALL: [Self; 4] = [Self::Sidebar, Self::Pending, Self::Recent, Self::Name];

    fn code(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Pending => "pending",
            Self::Recent => "recent",
            Self::Name => "name",
        }
    }

    pub fn label(self, t: Translator) -> &'static str {
        match self {
            Self::Sidebar => t.get("batch_order.sidebar"),
            Self::Pending => t.get("batch_order.pending"),
            Self::Recent => t.get("batch_order.recent"),
            Self::Name => t.get("batch_order.name"),
        }
    }
}

/// The lines a host card can show under its name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostFields {
    /// The dot coloured by the last connection check.
    pub status: bool,
    /// `user@address:port`.
    pub address: bool,
    /// The detected distribution.
    pub system: bool,
    /// The first line of the notes.
    pub notes: bool,
}

impl HostFields {
    const ALL: Self = Self {
        status: true,
        address: true,
        system: true,
        notes: true,
    };
    const NONE: Self = Self {
        status: false,
        address: false,
        system: false,
        notes: false,
    };
    const CODES: [&'static str; 4] = ["status", "address", "system", "notes"];

    /// Each field with its preference code and label, for the settings checkboxes.
    pub fn each(&mut self, t: Translator) -> [(&mut bool, &'static str); 4] {
        [
            (&mut self.status, t.get("host_detail.status")),
            (&mut self.address, t.get("host_detail.address")),
            (&mut self.system, t.get("host_detail.system")),
            (&mut self.notes, t.get("host_detail.notes")),
        ]
    }

    fn flags(self) -> [bool; 4] {
        [self.status, self.address, self.system, self.notes]
    }

    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let codes: Vec<&str> = value
            .as_array()?
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        let has = |code| codes.contains(&code);
        Some(Self {
            status: has("status"),
            address: has("address"),
            system: has("system"),
            notes: has("notes"),
        })
    }
}

impl Default for HostFields {
    /// What a card showed before it could be chosen.
    fn default() -> Self {
        Self {
            notes: false,
            ..Self::ALL
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Preferences {
    pub language: Language,
    pub appearance: Appearance,
    pub window: Option<WindowSize>,
    pub settings_window: Option<[f32; 2]>,
    pub host_detail: HostDetail,
    /// The fields kept for [`HostDetail::Custom`], even while another level is chosen.
    pub host_fields: HostFields,
    pub batch_order: BatchOrder,
    pub density: Density,
    /// Where the ungrouped hosts sit among the groups; `None` puts them last.
    pub ungrouped_at: Option<usize>,
}

impl Preferences {
    /// What host cards show at the chosen level.
    pub fn host_card_fields(&self) -> HostFields {
        match self.host_detail {
            HostDetail::Full => HostFields::ALL,
            HostDetail::NameOnly => HostFields::NONE,
            HostDetail::Custom => self.host_fields,
        }
    }

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
            host_detail: value["host_list"]["detail"]
                .as_str()
                .and_then(|code| HostDetail::ALL.into_iter().find(|d| d.code() == code))
                .unwrap_or_default(),
            host_fields: HostFields::from_json(&value["host_list"]["fields"]).unwrap_or_default(),
            density: value["host_list"]["density"]
                .as_str()
                .and_then(|code| Density::ALL.into_iter().find(|d| d.code() == code))
                .unwrap_or_default(),
            ungrouped_at: value["host_list"]["ungrouped_at"]
                .as_u64()
                .and_then(|at| usize::try_from(at).ok()),
            batch_order: value["batch_order"]
                .as_str()
                .and_then(|code| BatchOrder::ALL.into_iter().find(|o| o.code() == code))
                .unwrap_or_default(),
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
            "host_list": {
                "detail": self.host_detail.code(),
                "fields": HostFields::CODES
                    .into_iter()
                    .zip(self.host_fields.flags())
                    .filter_map(|(code, shown)| shown.then_some(code))
                    .collect::<Vec<_>>(),
                "density": self.density.code(),
                "ungrouped_at": self.ungrouped_at,
            },
            "batch_order": self.batch_order.code(),
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

    #[test]
    fn host_list_detail_and_batch_order_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-settings.json");
        let fields = HostFields {
            status: true,
            address: false,
            system: false,
            notes: true,
        };
        let preferences = Preferences {
            host_detail: HostDetail::Custom,
            host_fields: fields,
            batch_order: BatchOrder::Sidebar,
            density: Density::Compact,
            ungrouped_at: Some(1),
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        let loaded = Preferences::load(&path).unwrap();
        assert_eq!(loaded.host_detail, HostDetail::Custom);
        assert_eq!(loaded.host_card_fields(), fields);
        assert_eq!(loaded.batch_order, BatchOrder::Sidebar);
        assert_eq!(loaded.density, Density::Compact);
        assert_eq!(loaded.ungrouped_at, Some(1));
        // The custom choice is kept while another level is shown.
        let name_only = Preferences {
            host_detail: HostDetail::NameOnly,
            ..loaded
        };
        assert_eq!(name_only.host_card_fields(), HostFields::NONE);
        name_only.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap().host_fields, fields);
        // Older or unknown settings fall back to the full card.
        std::fs::write(
            &path,
            json!({"version": 1, "host_list": {"detail": "huge"}}).to_string(),
        )
        .unwrap();
        let loaded = Preferences::load(&path).unwrap();
        assert_eq!(loaded.host_detail, HostDetail::Full);
        assert_eq!(loaded.host_fields, HostFields::default());
        assert_eq!(loaded.batch_order, BatchOrder::Pending);
        assert_eq!(loaded.density, Density::Comfortable);
        assert_eq!(loaded.ungrouped_at, None);
    }
}
