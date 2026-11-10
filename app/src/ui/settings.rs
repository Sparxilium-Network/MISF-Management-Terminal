//! The settings window: its category tree, the vault, preferences, log and about pages.

use super::{
    App, ColumnToggle, Ctx, SettingsState, SettingsWindow, TreeRow,
    app::{SettingsPage, VaultMode},
    logging::export_log,
    texts,
    vault::choose_backup,
};
use crate::{
    i18n::{Language, Translator},
    preferences::{
        AUTO_LOCK_DEFAULT, AUTO_LOCK_MAX, BatchColumn, BatchDetail, BatchOrder, Density,
        HostDetail, InterfaceFont, SETTINGS_DEFAULT, UnlockMethod,
    },
    theme::Appearance,
};
use misf_core::audit::{Action, Outcome};
use serde_json::json;
use slint::{ComponentHandle, ModelRc, VecModel};

impl SettingsPage {
    fn label(self, t: Translator) -> &'static str {
        match self {
            Self::Vault => t.get("settings.nav_vault"),
            Self::Appearance => t.get("settings.nav_appearance"),
            Self::Security => t.get("settings.nav_security"),
            Self::Batch => t.get("settings.nav_batch"),
            Self::Language => t.get("settings.nav_language"),
            Self::Log => t.get("settings.nav_log"),
            Self::About => t.get("settings.about"),
        }
    }
}

impl App {
    /// The settings tree: top-level pages, and the Preferences branch with its pages
    /// beneath it while expanded.
    pub fn settings_tree(&self) -> Vec<TreeRow> {
        let t = self.t();
        let in_branch = SettingsPage::PREFERENCES.contains(&self.settings_page);
        let mut rows = Vec::new();
        for page in SettingsPage::ALL {
            let child = SettingsPage::PREFERENCES.contains(&page);
            if page == SettingsPage::PREFERENCES[0] {
                rows.push(TreeRow {
                    page: -1,
                    label: t.get("settings.nav_preferences").into(),
                    level: 1,
                    selected: in_branch && !self.preferences_expanded,
                });
            }
            if child && !self.preferences_expanded {
                continue;
            }
            rows.push(TreeRow {
                page: page.index(),
                label: page.label(t).into(),
                level: if child { 2 } else { 0 },
                selected: self.settings_page == page,
            });
        }
        rows
    }

    /// The branch row expands or collapses the branch; from another page it opens the
    /// branch's first page.
    pub fn settings_branch(&mut self) {
        let in_branch = SettingsPage::PREFERENCES.contains(&self.settings_page);
        self.preferences_expanded = !self.preferences_expanded || !in_branch;
        if !in_branch {
            self.settings_page = SettingsPage::PREFERENCES[0];
        }
    }

    /// The heading above a page; a page in the branch shows its path.
    pub fn settings_title(&self) -> String {
        let t = self.t();
        let label = self.settings_page.label(t);
        if SettingsPage::PREFERENCES.contains(&self.settings_page) {
            format!("{}  ›  {label}", t.get("settings.nav_preferences"))
        } else {
            label.to_owned()
        }
    }

    /// Applies one preference from the settings pages and saves the preferences.
    pub fn set_preference(&mut self, key: &str, value: i32) {
        let before = self.preferences;
        let index = usize::try_from(value).unwrap_or(0);
        let prefs = &mut self.preferences;
        match key {
            "appearance" => {
                prefs.appearance = Appearance::ALL.get(index).copied().unwrap_or_default()
            }
            "font" => prefs.font = InterfaceFont::ALL.get(index).copied().unwrap_or_default(),
            "density" => prefs.density = Density::ALL.get(index).copied().unwrap_or_default(),
            "host_detail" => {
                prefs.host_detail = HostDetail::ALL.get(index).copied().unwrap_or_default()
            }
            "unlock_method" => {
                prefs.unlock_method = UnlockMethod::ALL.get(index).copied().unwrap_or_default()
            }
            "auto_lock" => {
                prefs.auto_lock_minutes =
                    (index == 1).then(|| prefs.auto_lock_minutes.unwrap_or(AUTO_LOCK_DEFAULT))
            }
            "auto_lock_minutes" if prefs.auto_lock_minutes.is_some() => {
                prefs.auto_lock_minutes =
                    Some(u16::try_from(value).unwrap_or(1).clamp(1, AUTO_LOCK_MAX))
            }
            "batch_order" => {
                prefs.batch_order = BatchOrder::ALL.get(index).copied().unwrap_or_default()
            }
            "batch_detail" => {
                prefs.batch_detail = BatchDetail::ALL.get(index).copied().unwrap_or_default()
            }
            "language" => prefs.language = Language::from_index(value),
            _ => {}
        }
        self.preferences_changed(before);
    }

    pub fn set_host_field(&mut self, index: usize, shown: bool) {
        let before = self.preferences;
        let t = self.t();
        if let Some((flag, _)) = self.preferences.host_fields.each(t).into_iter().nth(index) {
            *flag = shown;
        }
        self.preferences_changed(before);
    }

    pub fn set_batch_column(&mut self, index: usize, shown: bool) {
        // The host column is always shown.
        if index == 0 || index >= BatchColumn::ALL.len() {
            return;
        }
        let before = self.preferences;
        self.preferences.batch_columns.shown[index] = shown;
        self.preferences_changed(before);
    }

    pub fn reset_column_widths(&mut self) {
        let before = self.preferences;
        self.preferences.batch_columns.reset_widths();
        self.preferences_changed(before);
    }

    /// Saves changed preferences and records language and appearance changes.
    fn preferences_changed(&mut self, before: crate::preferences::Preferences) {
        if self.preferences == before {
            return;
        }
        let t = self.t();
        let saved = self.preferences.save(&self.settings_path);
        let outcome = if saved.is_ok() {
            Outcome::Succeeded
        } else {
            Outcome::Failed
        };
        if self.preferences.language != before.language {
            self.log(Action::LanguageChange, outcome);
        }
        if self.preferences.appearance != before.appearance {
            self.log(Action::AppearanceChange, outcome);
        }
        match saved {
            Ok(()) => self.set_status(t.get("settings.saved")),
            Err(error) => self.error = Some(t.format("settings.save_error", &[("error", error)])),
        }
    }
}

pub fn push(app: &mut App, w: &SettingsWindow) {
    let t = app.t();
    let state = w.global::<SettingsState>();
    state.set_page(app.settings_page.index());
    state.set_tree(ModelRc::new(VecModel::from(app.settings_tree())));
    state.set_expanded(app.preferences_expanded);
    state.set_title(app.settings_title().into());
    state.set_busy(app.busy);
    state.set_encrypted(app.encrypted);
    state.set_locked(app.locked);
    state.set_path(app.path.clone().into());
    state.set_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());
    let mut format = match app.storage_format.as_str() {
        "sqlite" => t.get("settings.format_sqlite").to_owned(),
        "json" => t.get("settings.format_json").to_owned(),
        _ => t.get("settings.format_unknown").to_owned(),
    };
    if app.encrypted && !app.storage_format.is_empty() {
        format.push_str(t.get("settings.encrypted_suffix"));
    }
    state.set_format(format.into());
    state.set_schema(
        match (app.storage_format.as_str(), app.schema_version) {
            ("sqlite", Some(version)) => format!("v{version}"),
            ("json", _) => t.get("settings.schema_json").to_owned(),
            _ => "—".to_owned(),
        }
        .into(),
    );
    state.set_can_convert(app.storage_format == "json");
    state.set_log_size(
        t.format(
            "log.size",
            &[
                ("count", app.session_log.len().to_string()),
                (
                    "size",
                    format!("{:.1}", app.session_log.bytes() as f64 / 1024.0),
                ),
            ],
        )
        .into(),
    );
    let prefs = app.preferences;
    let position = |found: Option<usize>| found.unwrap_or(0) as i32;
    state.set_appearances(texts(Appearance::ALL.map(|a| a.label(t))));
    state.set_appearance(prefs.appearance.index());
    state.set_fonts(texts(InterfaceFont::ALL.map(|f| f.label(t))));
    state.set_font(position(
        InterfaceFont::ALL.iter().position(|f| *f == prefs.font),
    ));
    state.set_densities(texts(Density::ALL.map(|d| d.label(t))));
    state.set_density(position(
        Density::ALL.iter().position(|d| *d == prefs.density),
    ));
    state.set_host_details(texts(HostDetail::ALL.map(|d| d.label(t))));
    state.set_host_detail(position(
        HostDetail::ALL.iter().position(|d| *d == prefs.host_detail),
    ));
    let mut fields = prefs.host_fields;
    let fields = fields.each(t);
    state.set_host_fields(ModelRc::new(VecModel::from(
        fields.iter().map(|(shown, _)| **shown).collect::<Vec<_>>(),
    )));
    state.set_host_field_labels(texts(fields.iter().map(|(_, label)| *label)));
    state.set_unlock_methods(texts(UnlockMethod::ALL.iter().map(|m| m.label(t))));
    state.set_unlock_method(position(
        UnlockMethod::ALL
            .iter()
            .position(|m| *m == prefs.unlock_method),
    ));
    state.set_auto_lock(prefs.auto_lock_minutes.is_some());
    state.set_auto_lock_minutes(i32::from(
        prefs.auto_lock_minutes.unwrap_or(AUTO_LOCK_DEFAULT),
    ));
    state.set_auto_lock_max(i32::from(AUTO_LOCK_MAX));
    state.set_batch_orders(texts(BatchOrder::ALL.map(|o| o.label(t))));
    state.set_batch_order(position(
        BatchOrder::ALL.iter().position(|o| *o == prefs.batch_order),
    ));
    state.set_batch_details(texts(BatchDetail::ALL.map(|d| d.label(t))));
    state.set_batch_detail(position(
        BatchDetail::ALL
            .iter()
            .position(|d| *d == prefs.batch_detail),
    ));
    let toggles: Vec<ColumnToggle> = BatchColumn::ALL
        .iter()
        .enumerate()
        .map(|(i, column)| ColumnToggle {
            index: i as i32,
            label: column.label(t).into(),
            shown: i == 0 || prefs.batch_columns.shown[i],
        })
        .collect();
    state.set_batch_columns(ModelRc::new(VecModel::from(
        toggles
            .chunks(3)
            .map(|line| ModelRc::new(VecModel::from(line.to_vec())))
            .collect::<Vec<_>>(),
    )));
    state.set_languages(texts(Language::ALL.map(Language::name)));
    state.set_language(prefs.language.index());
    let visible = w.window().is_visible();
    if app.settings_open && !visible {
        let [width, height] = prefs.settings_window.unwrap_or(SETTINGS_DEFAULT);
        w.window().set_size(slint::LogicalSize::new(width, height));
        let _ = w.show();
    } else if !app.settings_open && visible {
        let _ = w.hide();
    }
}

pub fn wire(ctx: &Ctx, w: &SettingsWindow) {
    let state = w.global::<SettingsState>();
    let c = ctx.clone();
    state.on_page_chosen(move |page| {
        c.update(|app| {
            if let Some(page) = SettingsPage::from_index(page) {
                app.settings_page = page;
            }
        })
    });
    let c = ctx.clone();
    state.on_branch_clicked(move || c.update(App::settings_branch));
    let c = ctx.clone();
    state.on_enable_vault(move || c.update(|app| app.open_vault_dialog(VaultMode::Create)));
    let c = ctx.clone();
    state.on_unlock(move || c.update(|app| app.open_vault_dialog(VaultMode::Unlock)));
    let c = ctx.clone();
    state.on_manage(move || c.update(|app| app.open_vault_dialog(VaultMode::Manage)));
    let c = ctx.clone();
    state.on_backup(move || choose_backup(&c));
    let c = ctx.clone();
    state.on_convert(move || c.update(|app| app.send(json!({"action":"convert_storage"}))));
    let c = ctx.clone();
    state.on_export_log(move || export_log(&c));
    let c = ctx.clone();
    state.on_preference(move |key, value| c.update(|app| app.set_preference(&key, value)));
    let c = ctx.clone();
    state.on_host_field(move |index, shown| {
        c.update(|app| app.set_host_field(index.max(0) as usize, shown))
    });
    let c = ctx.clone();
    state.on_batch_column(move |index, shown| {
        c.update(|app| app.set_batch_column(index.max(0) as usize, shown))
    });
    let c = ctx.clone();
    state.on_reset_widths(move || c.update(App::reset_column_widths));
    let c = ctx.clone();
    w.window().on_close_requested(move || {
        c.update(|app| {
            app.settings_open = false;
            app.log(Action::SettingsClose, Outcome::Succeeded);
        });
        slint::CloseRequestResponse::HideWindow
    });
}
