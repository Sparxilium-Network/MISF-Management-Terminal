use crate::i18n::Translator;
use eframe::egui::{self, Color32, FontId, Vec2};

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

    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        });
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub surface: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub warning: Color32,
    pub error: Color32,
}

impl Palette {
    pub fn for_context(ctx: &egui::Context) -> Self {
        Self::new(ctx.theme() == egui::Theme::Dark)
    }

    fn new(dark: bool) -> Self {
        if dark {
            Self {
                background: Color32::from_rgb(17, 24, 35),
                surface: Color32::from_rgb(26, 37, 52),
                ink: Color32::from_rgb(231, 238, 248),
                muted: Color32::from_rgb(164, 181, 204),
                border: Color32::from_rgb(57, 75, 98),
                warning: Color32::from_rgb(246, 194, 113),
                error: Color32::from_rgb(255, 147, 147),
            }
        } else {
            Self {
                background: Color32::from_rgb(244, 247, 251),
                surface: Color32::WHITE,
                ink: Color32::from_rgb(34, 53, 77),
                muted: Color32::from_rgb(98, 116, 140),
                border: Color32::from_rgb(220, 229, 239),
                warning: Color32::from_rgb(146, 97, 33),
                error: Color32::from_rgb(174, 65, 65),
            }
        }
    }
}

pub fn install(ctx: &egui::Context) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let dark = theme == egui::Theme::Dark;
        let palette = Palette::new(dark);
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        style.visuals.override_text_color = Some(palette.ink);
        style.visuals.panel_fill = palette.background;
        style.visuals.window_fill = palette.surface;
        style.visuals.selection.bg_fill = if dark {
            Color32::from_rgb(44, 83, 124)
        } else {
            Color32::from_rgb(218, 235, 251)
        };
        style.spacing.item_spacing = Vec2::new(10.0, 10.0);
        style.spacing.button_padding = Vec2::new(14.0, 9.0);
        style.spacing.interact_size.y = 34.0;
        // Command and risk hints appear only after a deliberate pause on the control.
        style.interaction.tooltip_delay = 1.5;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
        ctx.set_style_of(theme, style);
    }
}
