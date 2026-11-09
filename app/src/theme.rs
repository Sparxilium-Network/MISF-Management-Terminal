use crate::i18n::Translator;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn code(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.code() == code)
    }

    pub fn label(self, t: Translator) -> &'static str {
        match self {
            Self::System => t.get("theme.system"),
            Self::Light => t.get("theme.light"),
            Self::Dark => t.get("theme.dark"),
        }
    }

    /// The value the Slint `Theme.appearance` property takes.
    pub fn index(self) -> i32 {
        match self {
            Self::System => 0,
            Self::Light => 1,
            Self::Dark => 2,
        }
    }
}
