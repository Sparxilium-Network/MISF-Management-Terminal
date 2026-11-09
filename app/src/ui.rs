use crate::{
    i18n::{Language, Translator},
    preferences::{
        BatchColumn, BatchDetail, BatchOrder, Density, HostDetail, Preferences, SETTINGS_DEFAULT,
        SETTINGS_MIN, UnlockMethod, WindowSize,
    },
    session_log::SessionLog,
    theme::{self, Appearance, Palette},
    worker::{Event, Worker},
};
use chrono::{DateTime, Local};
use eframe::egui::{self, Color32, RichText, Stroke};
use misf_core::model::{CheckResult, ConnectionTest, Host, HostGroup, RestartStatus};
use misf_core::{
    audit::{Action, AuditLog, Outcome},
    batch, package_manager,
    runtime::RuntimePaths,
    vault::{MIN_PASSPHRASE_CHARS, PassphraseProblem, passphrase_problem},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
use zeroize::Zeroize;

const NAVY: Color32 = Color32::from_rgb(19, 36, 59);
const BLUE: Color32 = Color32::from_rgb(33, 117, 195);
const RED: Color32 = Color32::from_rgb(178, 58, 58);

/// The group name dialog: `id` is `None` to add a group, or the group being renamed.
#[derive(Default)]
struct GroupDialog {
    id: Option<String>,
    name: String,
}

/// What the sidebar is dragging, carried as egui's drag-and-drop payload.
#[derive(Clone, PartialEq)]
enum DragItem {
    Host(String),
    Group(String),
}

/// Where a drag would land: before a host or group, or at the end when `before` is `None`.
enum DropAt {
    Host {
        group: String,
        before: Option<String>,
    },
    Group {
        before: Option<String>,
    },
}

/// Where the sidebar drew each section and host card this frame.
struct SidebarLayout {
    /// The scroll area's visible part; drops outside it are ignored.
    visible: egui::Rect,
    /// Heading and cards of each section, by group ID ("" = ungrouped).
    sections: Vec<(egui::Rect, String)>,
    /// Each card with its group ID and host ID.
    cards: Vec<(egui::Rect, String, String)>,
}

impl SidebarLayout {
    /// The drop for a host and the y of its insertion line.
    fn host_drop(&self, pointer: egui::Pos2, dragged: &str) -> Option<(DropAt, f32)> {
        if !self.visible.contains(pointer) {
            return None;
        }
        let card = self
            .cards
            .iter()
            .position(|(rect, ..)| (rect.top() - 3.0..=rect.bottom() + 3.0).contains(&pointer.y));
        if let Some(index) = card {
            let (rect, group, id) = &self.cards[index];
            if id == dragged {
                return None;
            }
            if pointer.y < rect.center().y {
                let at = DropAt::Host {
                    group: group.clone(),
                    before: Some(id.clone()),
                };
                return Some((at, rect.top() - 2.5));
            }
            let before = self.cards[index + 1..]
                .iter()
                .find(|(.., other)| other != dragged)
                .filter(|(_, other_group, _)| other_group == group)
                .map(|(.., other)| other.clone());
            let at = DropAt::Host {
                group: group.clone(),
                before,
            };
            return Some((at, rect.bottom() + 2.5));
        }
        // A heading or a section's empty space: the end of that section.
        let (rect, group) = self
            .sections
            .iter()
            .find(|(rect, _)| rect.y_range().contains(pointer.y))?;
        let at = DropAt::Host {
            group: group.clone(),
            before: None,
        };
        Some((at, rect.bottom() - 2.0))
    }

    /// The drop for a group and the y of its insertion line.
    fn group_drop(&self, pointer: egui::Pos2, dragged: &str) -> Option<(DropAt, f32)> {
        if !self.visible.contains(pointer) {
            return None;
        }
        let index = self
            .sections
            .iter()
            .position(|(rect, _)| rect.y_range().contains(pointer.y))?;
        let (rect, group) = &self.sections[index];
        if group == dragged {
            return None;
        }
        if pointer.y < rect.center().y {
            let at = DropAt::Group {
                before: Some(group.clone()),
            };
            return Some((at, rect.top()));
        }
        let before = self.sections[index + 1..]
            .iter()
            .find(|(_, other)| other != dragged)
            .map(|(_, other)| other.clone());
        Some((DropAt::Group { before }, rect.bottom()))
    }
}

struct Editor {
    host: Host,
    port: String,
    /// Key authentication through a stored key (`host.key_id`) rather than an inline one.
    saved_key: bool,
    show_secrets: bool,
    error: String,
}
/// A stored SSH key as the worker describes it: never the key material.
struct KeyRow {
    id: String,
    name: String,
    algorithm: String,
    fingerprint: String,
}
impl KeyRow {
    fn label(&self) -> String {
        format!("{} · {}", self.name, self.algorithm)
    }
}
#[derive(Default)]
struct KeyDialog {
    name: String,
    private_key: String,
    passphrase: String,
    show_secrets: bool,
    error: String,
    /// The key awaiting delete confirmation.
    delete: Option<String>,
}
impl Drop for KeyDialog {
    fn drop(&mut self) {
        self.private_key.zeroize();
        self.passphrase.zeroize();
    }
}
/// A page of the settings window, picked from its side list.
#[derive(Clone, Copy, PartialEq)]
enum SettingsPage {
    Vault,
    Preferences,
    Language,
    Log,
    About,
}
#[derive(Clone, Copy, PartialEq)]
enum View {
    Host,
    Batch,
}
/// An upgrade awaiting explicit confirmation; `batch` sends one request for all hosts.
struct UpgradeConfirm {
    ids: Vec<String>,
    batch: bool,
    acknowledged: bool,
}
/// A reboot awaiting two confirmations: the risks first, then a final yes.
struct RebootConfirm {
    id: String,
    name: String,
    address: String,
    command: &'static str,
    acknowledged: bool,
    final_step: bool,
}
/// A failed batch host the worker is waiting on: stop the batch or start the rest.
struct BatchFailure {
    name: String,
    upgrade: bool,
    error: String,
}
struct TrustDialog {
    host_id: String,
    address: String,
    old: String,
    actual: String,
    next: Option<String>,
}

struct SlotRow {
    index: usize,
    kind: String,
    name: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum VaultMode {
    Create,
    Unlock,
    Manage,
    AddKey,
    Passkey,
    Revoke(usize),
    Rename(usize),
}
struct VaultDialog {
    mode: VaultMode,
    /// Slot name for a new or renamed slot; not a secret.
    name: String,
    password: String,
    confirmation: String,
    error: String,
    hello: bool,
    acknowledged: bool,
}
impl Drop for VaultDialog {
    fn drop(&mut self) {
        self.password.zeroize();
        self.confirmation.zeroize();
    }
}

pub struct ManagementApp {
    audit: AuditLog,
    settings_path: PathBuf,
    /// False when ui-settings.json could not be read, so closing never overwrites it.
    window_loaded: bool,
    /// Main and settings window sizes as loaded, so closing saves only a change.
    saved_window: (Option<WindowSize>, Option<[f32; 2]>),
    debug: bool,
    passkey_allowed: bool,
    /// The preferred unlock method starts by itself only once per launch.
    default_unlock_started: bool,
    preferences: Preferences,
    settings_open: bool,
    settings_page: SettingsPage,
    worker: Worker,
    hosts: Vec<Host>,
    groups: Vec<HostGroup>,
    group_dialog: Option<GroupDialog>,
    selected: String,
    search: String,
    package_search: String,
    path: String,
    busy: bool,
    status: String,
    error: Option<String>,
    editor: Option<Editor>,
    delete_id: Option<String>,
    trust: Option<TrustDialog>,
    after_trust: Option<String>,
    tab: usize,
    view: View,
    batch_selected: BTreeSet<String>,
    /// Whether the chosen host's packages are open in a window ([`BatchDetail::Popup`]).
    batch_popup: bool,
    /// The one package manager the batch selection may contain.
    batch_manager: String,
    upgrade_confirm: Option<UpgradeConfirm>,
    batch_failure: Option<BatchFailure>,
    reboot_confirm: Option<RebootConfirm>,
    /// This run's full log; memory only, gone when the app closes.
    session_log: SessionLog,
    /// The error last copied into [`Self::session_log`], so each is logged once.
    logged_error: Option<String>,
    encrypted: bool,
    /// "sqlite" or "json" from the last host list; empty while locked.
    storage_format: String,
    schema_version: Option<u64>,
    locked: bool,
    plaintext_exists: bool,
    plaintext_path: String,
    plaintext_delete_prompt: bool,
    /// The older schema version of a just-decrypted vault, waiting for upgrade consent.
    schema_upgrade_prompt: Option<u64>,
    slots: Vec<SlotRow>,
    storage_warning: String,
    vault_dialog: Option<VaultDialog>,
    ssh_keys: Vec<KeyRow>,
    key_dialog: Option<KeyDialog>,
    #[cfg(feature = "capture")]
    capture_started: std::time::Instant,
    #[cfg(feature = "capture")]
    capture_requested: bool,
}

impl ManagementApp {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: RuntimePaths, audit: AuditLog) -> Self {
        let worker = Worker::new(cc.egui_ctx.clone(), paths.clone(), audit.clone());
        Self::initialize(&cc.egui_ctx, paths, audit, worker)
    }

    fn initialize(
        ctx: &egui::Context,
        paths: RuntimePaths,
        audit: AuditLog,
        worker: Worker,
    ) -> Self {
        let settings_path = paths.store.with_file_name("ui-settings.json");
        let loaded = Preferences::load(&settings_path);
        let preferences = loaded.clone().unwrap_or_default();
        let window_loaded = loaded.is_ok();
        theme::install(ctx);
        preferences.appearance.apply(ctx);
        let mut app = Self {
            audit,
            settings_path,
            window_loaded,
            saved_window: (preferences.window, preferences.settings_window),
            debug: paths.debug,
            passkey_allowed: paths.passkey_allowed(),
            default_unlock_started: false,
            preferences,
            settings_open: false,
            settings_page: SettingsPage::Vault,
            worker,
            hosts: vec![],
            groups: vec![],
            group_dialog: None,
            selected: String::new(),
            search: String::new(),
            package_search: String::new(),
            path: String::new(),
            busy: false,
            status: String::new(),
            error: loaded.err().map(|error| {
                Translator(preferences.language).format("settings.load_error", &[("error", error)])
            }),
            editor: None,
            delete_id: None,
            trust: None,
            after_trust: None,
            tab: 0,
            view: View::Host,
            batch_selected: BTreeSet::new(),
            batch_popup: false,
            batch_manager: String::new(),
            upgrade_confirm: None,
            batch_failure: None,
            reboot_confirm: None,
            session_log: SessionLog::new(),
            logged_error: None,
            encrypted: false,
            storage_format: String::new(),
            schema_version: None,
            locked: false,
            plaintext_exists: false,
            plaintext_path: String::new(),
            plaintext_delete_prompt: false,
            schema_upgrade_prompt: None,
            slots: vec![],
            storage_warning: String::new(),
            vault_dialog: None,
            ssh_keys: vec![],
            key_dialog: None,
            #[cfg(feature = "capture")]
            capture_started: std::time::Instant::now(),
            #[cfg(feature = "capture")]
            capture_requested: false,
        };
        app.send(json!({"action":"storage_status"}));
        #[cfg(feature = "capture")]
        if std::env::var("MISF_OPEN_EDITOR").as_deref() == Ok("1") {
            app.open_editor(None);
        }
        #[cfg(feature = "capture")]
        if std::env::var("MISF_OPEN_VAULT_SETUP").as_deref() == Ok("1") {
            app.open_vault_dialog(VaultMode::Create);
        }
        #[cfg(feature = "capture")]
        if std::env::var("MISF_OPEN_PASSKEY").as_deref() == Ok("1") {
            app.open_vault_dialog(VaultMode::Passkey);
        }
        #[cfg(feature = "capture")]
        if let Ok(page @ ("1" | "about" | "preferences")) =
            std::env::var("MISF_OPEN_SETTINGS").as_deref()
        {
            match page {
                "about" => app.settings_page = SettingsPage::About,
                "preferences" => app.settings_page = SettingsPage::Preferences,
                _ => {}
            }
            app.settings_open = true;
            app.log(Action::SettingsOpen, Outcome::Succeeded);
        }
        #[cfg(feature = "capture")]
        if std::env::var("MISF_OPEN_KEYS").as_deref() == Ok("1") {
            app.open_key_dialog();
        }
        #[cfg(feature = "capture")]
        if std::env::var("MISF_OPEN_BATCH").as_deref() == Ok("1") {
            app.view = View::Batch;
            app.log(Action::MainTab, Outcome::Succeeded);
        }
        app
    }

    /// Remembers the restored size and the maximized state, saved when the app closes.
    fn track_window(&mut self, ctx: &egui::Context) {
        let (rect, maximized, minimized) = ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.inner_rect, viewport.maximized, viewport.minimized)
        });
        if minimized == Some(true) {
            return;
        }
        let maximized = maximized.unwrap_or(false);
        let previous = self.preferences.window;
        let [width, height] = match (rect, maximized, previous) {
            // A maximized window keeps the size it returns to when restored.
            (_, true, Some(previous)) => previous.inner_size(),
            (Some(rect), _, _) => [rect.width(), rect.height()],
            (None, _, _) => return,
        };
        self.preferences.window = Some(WindowSize {
            width: width.max(WindowSize::MIN[0]),
            height: height.max(WindowSize::MIN[1]),
            maximized,
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let t = Translator(self.preferences.language);
        let mut open = self.settings_open;
        let pages = [
            (SettingsPage::Vault, t.get("settings.nav_vault")),
            (SettingsPage::Preferences, t.get("settings.nav_preferences")),
            (SettingsPage::Language, t.get("settings.nav_language")),
            (SettingsPage::Log, t.get("settings.nav_log")),
            (SettingsPage::About, t.get("settings.about")),
        ];
        let shown = egui::Window::new(t.get("settings.title"))
            .id(egui::Id::unique("settings_window"))
            .open(&mut open)
            .default_pos([16.0, 70.0])
            .default_size(self.preferences.settings_window.unwrap_or(SETTINGS_DEFAULT))
            .min_size(SETTINGS_MIN)
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        [170.0, ui.available_height()].into(),
                        egui::Layout::top_down_justified(egui::Align::LEFT),
                        |ui| {
                            for (page, label) in pages {
                                if ui
                                    .selectable_label(self.settings_page == page, label)
                                    .clicked()
                                {
                                    self.settings_page = page;
                                }
                            }
                        },
                    );
                    ui.separator();
                    ui.vertical(|ui| {
                        let title = pages
                            .iter()
                            .find(|(page, _)| *page == self.settings_page)
                            .map_or("", |(_, label)| label);
                        ui.heading(title);
                        ui.add_space(10.0);
                        match self.settings_page {
                            SettingsPage::Vault => self.vault_settings(ui),
                            SettingsPage::Preferences | SettingsPage::Language => {
                                egui::ScrollArea::vertical()
                                    .id_salt("preferences_page")
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        self.preferences_form(ui);
                                        ui.add_space(12.0);
                                        ui.label(
                                            RichText::new(t.get("settings.plaintext_notice"))
                                                .size(12.0)
                                                .color(Palette::for_context(ui.ctx()).muted),
                                        );
                                    });
                            }
                            SettingsPage::Log => self.log_settings(ui),
                            SettingsPage::About => {
                                if self.about_section(ui) {
                                    self.send(json!({"action":"convert_storage"}));
                                }
                            }
                        }
                    });
                });
            });
        if let Some(shown) = shown {
            let size = shown.response.rect.size();
            self.preferences.settings_window = Some([size.x.round(), size.y.round()]);
        }
        self.settings_open = open;
        if !self.settings_open {
            self.log(Action::SettingsClose, Outcome::Succeeded);
        }
    }

    /// LUKSbox status with the key slot and backup actions that used to sit in the top bar.
    fn vault_settings(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        ui.label(
            RichText::new(if self.encrypted {
                t.get("storage.encrypted")
            } else {
                t.get("storage.not_created")
            })
            .color(palette.warning),
        );
        ui.add(egui::Label::new(RichText::new(&self.path).size(11.0).color(palette.muted)).wrap());
        ui.add_space(10.0);
        ui.add_enabled_ui(!self.busy, |ui| {
            if !self.encrypted {
                ui.label(t.get("settings.vault_not_created"));
                ui.add_space(6.0);
                if ui.button(t.get("vault.enable_button")).clicked() {
                    self.open_vault_dialog(VaultMode::Create);
                }
            } else if self.locked {
                ui.label(t.get("settings.vault_locked"));
                ui.add_space(6.0);
                if ui.button(t.get("vault.unlock_host")).clicked() {
                    self.open_vault_dialog(VaultMode::Unlock);
                }
            } else {
                ui.label(t.get("settings.vault_description"));
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t.get("vault.manage")).clicked() {
                        self.open_vault_dialog(VaultMode::Manage);
                    }
                    if ui.button(t.get("vault.export")).clicked() {
                        self.choose_backup();
                    }
                });
            }
        });
    }

    /// Returns true when the user asked to convert a legacy JSON store.
    fn about_section(&self, ui: &mut egui::Ui) -> bool {
        let t = Translator(self.preferences.language);
        let mut format = match self.storage_format.as_str() {
            "sqlite" => t.get("settings.format_sqlite").to_owned(),
            "json" => t.get("settings.format_json").to_owned(),
            _ => t.get("settings.format_unknown").to_owned(),
        };
        if self.encrypted && !self.storage_format.is_empty() {
            format.push_str(t.get("settings.encrypted_suffix"));
        }
        let schema = match (self.storage_format.as_str(), self.schema_version) {
            ("sqlite", Some(version)) => format!("v{version}"),
            ("json", _) => t.get("settings.schema_json").to_owned(),
            _ => "—".to_owned(),
        };
        let action =
            (self.storage_format == "json").then(|| t.get("settings.convert_db").to_owned());
        egui::Grid::new("about_grid")
            .spacing([24.0, 6.0])
            .show(ui, |ui| {
                ui.label(t.get("settings.app_version"));
                ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                ui.end_row();
                ui.label(t.get("settings.storage_format"));
                ui.label(format);
                ui.end_row();
                ui.label(t.get("settings.schema_version"));
                ui.label(schema);
                ui.end_row();
            });
        // Outside the grid: a widget group in a cell would shrink that column to nothing.
        let convert = action.is_some_and(|label| {
            ui.add_space(4.0);
            ui.add_enabled(!self.busy, egui::Button::new(label))
                .clicked()
        });
        ui.add_space(14.0);
        let palette = Palette::for_context(ui.ctx());
        ui.label(
            RichText::new(t.get("app.development_warning"))
                .strong()
                .color(palette.warning),
        );
        ui.label(t.get("app.development_detail"));
        convert
    }

    /// The language row on the Language page, the appearance row on Preferences.
    fn preferences_form(&mut self, ui: &mut egui::Ui) {
        let before = self.preferences;
        let t = Translator(before.language);
        let language_page = self.settings_page == SettingsPage::Language;
        egui::Grid::new("preferences_form")
            .spacing([24.0, 16.0])
            .show(ui, |ui| {
                if !language_page {
                    ui.label(t.get("settings.appearance"));
                    egui::ComboBox::from_id_salt("appearance")
                        .selected_text(self.preferences.appearance.label(t))
                        .width(165.0)
                        .show_ui(ui, |ui| {
                            for appearance in Appearance::ALL {
                                ui.selectable_value(
                                    &mut self.preferences.appearance,
                                    appearance,
                                    appearance.label(t),
                                );
                            }
                        });
                    ui.end_row();
                    ui.label(t.get("density.title"));
                    egui::ComboBox::from_id_salt("density")
                        .selected_text(self.preferences.density.label(t))
                        .width(165.0)
                        .show_ui(ui, |ui| {
                            for density in Density::ALL {
                                ui.selectable_value(
                                    &mut self.preferences.density,
                                    density,
                                    density.label(t),
                                );
                            }
                        });
                    ui.end_row();
                    ui.label(t.get("host_detail.title"));
                    egui::ComboBox::from_id_salt("host_detail")
                        .selected_text(self.preferences.host_detail.label(t))
                        .width(165.0)
                        .show_ui(ui, |ui| {
                            for detail in HostDetail::ALL {
                                ui.selectable_value(
                                    &mut self.preferences.host_detail,
                                    detail,
                                    detail.label(t),
                                );
                            }
                        });
                    ui.end_row();
                    if self.preferences.host_detail == HostDetail::Custom {
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            for (shown, label) in self.preferences.host_fields.each(t) {
                                ui.checkbox(shown, label);
                            }
                        });
                        ui.end_row();
                    }
                    ui.label(t.get("unlock_method.title"));
                    egui::ComboBox::from_id_salt("unlock_method")
                        .selected_text(self.preferences.unlock_method.label(t))
                        .width(165.0)
                        .show_ui(ui, |ui| {
                            for &method in UnlockMethod::ALL {
                                ui.selectable_value(
                                    &mut self.preferences.unlock_method,
                                    method,
                                    method.label(t),
                                );
                            }
                        });
                    ui.end_row();
                    ui.label(t.get("batch_order.title"));
                    egui::ComboBox::from_id_salt("batch_order")
                        .selected_text(self.preferences.batch_order.label(t))
                        .width(165.0)
                        .show_ui(ui, |ui| {
                            for order in BatchOrder::ALL {
                                ui.selectable_value(
                                    &mut self.preferences.batch_order,
                                    order,
                                    order.label(t),
                                );
                            }
                        });
                    ui.end_row();
                    ui.label(t.get("batch_detail.title"));
                    egui::ComboBox::from_id_salt("batch_detail")
                        .selected_text(self.preferences.batch_detail.label(t))
                        .width(165.0)
                        .show_ui(ui, |ui| {
                            for detail in BatchDetail::ALL {
                                ui.selectable_value(
                                    &mut self.preferences.batch_detail,
                                    detail,
                                    detail.label(t),
                                );
                            }
                        });
                    ui.end_row();
                    return;
                }
                ui.label(t.get("settings.language"));
                egui::ComboBox::from_id_salt("language")
                    .selected_text(self.preferences.language.name())
                    .width(165.0)
                    .show_ui(ui, |ui| {
                        for language in Language::ALL {
                            ui.selectable_value(
                                &mut self.preferences.language,
                                language,
                                language.name(),
                            );
                        }
                    });
                ui.end_row();
            });
        if !language_page {
            // Below the grid, so its label stays at the top of this tall block.
            ui.add_space(16.0);
            ui.label(t.get("batch_columns.title"));
            ui.add_space(6.0);
            egui::Grid::new("batch_columns_grid")
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    let columns = &mut self.preferences.batch_columns;
                    for (i, column) in BatchColumn::ALL.into_iter().enumerate() {
                        if column == BatchColumn::Host {
                            ui.add_enabled(false, egui::Checkbox::new(&mut true, column.label(t)));
                        } else {
                            ui.checkbox(&mut columns.shown[i], column.label(t));
                        }
                        if i % 4 == 3 {
                            ui.end_row();
                        }
                    }
                });
            ui.add_space(6.0);
            if ui
                .button(t.get("batch_columns.reset_widths"))
                .on_hover_text(t.get("batch_columns.drag_hint"))
                .clicked()
            {
                self.preferences.batch_columns.reset_widths();
            }
        }
        if self.preferences != before {
            self.preferences.appearance.apply(ui.ctx());
            let t = Translator(self.preferences.language);
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
                Ok(()) => self.status = t.get("settings.saved").into(),
                Err(error) => {
                    self.error = Some(t.format("settings.save_error", &[("error", error)]))
                }
            }
            ui.ctx().request_repaint();
        }
    }

    fn current(&self) -> Option<&Host> {
        self.hosts.iter().find(|host| host.id == self.selected)
    }
    fn log(&mut self, action: Action, outcome: Outcome) {
        self.log_at_slot(action, outcome, None);
    }
    fn log_at_slot(&mut self, action: Action, outcome: Outcome, slot: Option<usize>) {
        let host = if matches!(
            action,
            Action::HostSelect
                | Action::EditorOpen
                | Action::DeletePrompt
                | Action::DeleteCancel
                | Action::TrustCancel
        ) {
            self.current()
        } else {
            None
        };
        let name = |value: Value| value.as_str().unwrap_or("?").to_owned();
        let mut entry = format!(
            "{} {}",
            name(serde_json::to_value(action).unwrap_or_default()),
            name(serde_json::to_value(outcome).unwrap_or_default())
        );
        if let Some(host) = host {
            entry.push_str(&format!(" host={:?}", host.name));
        }
        if let Some(slot) = slot {
            entry.push_str(&format!(" slot={slot}"));
        }
        if self.audit.record(action, outcome, host, slot).is_err() {
            self.error = Some(
                Translator(self.preferences.language)
                    .get("audit.failed")
                    .into(),
            );
        }
        self.session_log.add("UI", &entry, "");
    }
    fn send(&mut self, request: Value) {
        self.log_operation(&request);
        self.session_log.request(&request);
        let t = Translator(self.preferences.language);
        self.busy = true;
        self.status = t.get("status.working").into();
        if self.worker.requests.send(request).is_err() {
            self.busy = false;
            self.error = Some(t.get("status.worker_ended").into());
        }
    }

    fn process_events(&mut self) {
        let t = Translator(self.preferences.language);
        while let Ok(event) = self.worker.events.try_recv() {
            match event {
                Event::Progress(message) => {
                    self.session_log.add("PROGRESS", &message, "");
                    self.status = t.progress(&message);
                }
                Event::BatchHost { id, result } => {
                    let name = self.host_name(&id);
                    let host = self.hosts.iter().find(|h| h.id == id);
                    self.session_log.host(t, &name, host, &result);
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == id) {
                        record_result(host, result);
                    }
                }
                Event::BatchFailed { id, result } => {
                    let name = self
                        .hosts
                        .iter()
                        .find(|h| h.id == id)
                        .map_or(id, |h| h.name.clone());
                    self.session_log.add(
                        "BATCH",
                        &format!("{name}: {}, waiting for a decision", result.status),
                        "",
                    );
                    self.batch_failure = Some(BatchFailure {
                        name,
                        upgrade: result.operation == "upgrade",
                        error: result.error,
                    });
                }
                Event::Done { action, result } => {
                    self.busy = false;
                    self.batch_failure = None;
                    self.log_done(&action, &result);
                    match result {
                        Ok(data) => self.received(&action, data),
                        Err(error) => {
                            self.status = t.get("status.failed").into();
                            self.error = Some(error);
                            // The sidebar showed this change before it was saved.
                            if action == "arrange" || action == "group_collapse" || action == "test"
                            {
                                self.send(json!({"action":"list"}));
                            }
                            if action == "vault_create" {
                                self.vault_dialog = None;
                                self.send(json!({"action":"storage_status"}));
                            }
                        }
                    }
                }
            }
        }
    }

    fn received(&mut self, action: &str, data: Value) {
        let t = Translator(self.preferences.language);
        match action {
            "storage_status" => {
                self.apply_storage(&data);
                if !self.encrypted {
                    // There is no unencrypted mode: a vault must exist before any host.
                    self.clear_host_cache();
                    if self.vault_dialog.is_none() {
                        self.open_vault_dialog(VaultMode::Create);
                    }
                    self.status = t.get("vault.setup_required").into();
                } else if self.locked {
                    self.clear_host_cache();
                    self.open_vault_dialog(VaultMode::Unlock);
                    self.status = t.get("vault.locked").into();
                    self.start_default_unlock();
                } else {
                    self.check_plaintext_copy();
                    self.send(json!({"action":"list"}));
                }
            }
            "vault_create" | "vault_unlock" | "vault_unlock_passkey" | "vault_upgrade" => {
                self.apply_storage(&data);
                self.vault_dialog = None;
                self.schema_upgrade_prompt = data["pending_upgrade"].as_u64();
                if self.schema_upgrade_prompt.is_some() {
                    return;
                }
                if action == "vault_upgrade" {
                    self.status = t.format(
                        "settings.converted",
                        &[("version", misf_core::sqlite::SCHEMA_VERSION.to_string())],
                    );
                }
                self.check_plaintext_copy();
                self.send(json!({"action":"list"}));
            }
            "vault_delete_plaintext" => {
                self.apply_storage(&data);
                self.plaintext_delete_prompt = false;
                self.status = t.get("plaintext.deleted").into();
                // A legacy hosts.json may remain after the plaintext database is gone.
                self.check_plaintext_copy();
            }
            "vault_upgrade_cancel" => {
                self.apply_storage(&data);
                self.schema_upgrade_prompt = None;
                self.open_vault_dialog(VaultMode::Unlock);
                self.status = t.get("vault.locked").into();
            }
            "vault_lock" => {
                self.apply_storage(&data);
                self.schema_upgrade_prompt = None;
                self.clear_host_cache();
                self.vault_dialog = None;
                self.status = t.get("status.locked").into();
            }
            "vault_add_key" | "vault_add_passkey" => {
                self.apply_storage(&data["storage"]);
                self.open_vault_dialog(VaultMode::Manage);
                self.status = t.format("status.slot_added", &[("slot", data["slot"].to_string())]);
            }
            "vault_revoke_key" => {
                self.apply_storage(&data["storage"]);
                self.open_vault_dialog(VaultMode::Manage);
                self.status =
                    t.format("status.slot_revoked", &[("slot", data["slot"].to_string())]);
            }
            "vault_rename_key" => {
                self.apply_storage(&data["storage"]);
                self.open_vault_dialog(VaultMode::Manage);
                self.status =
                    t.format("status.slot_renamed", &[("slot", data["slot"].to_string())]);
            }
            "convert_storage" => {
                self.status = t.format(
                    "settings.converted",
                    &[("version", data["schema_version"].to_string())],
                );
                self.send(json!({"action":"list"}));
            }
            "vault_backup" => {
                self.status = t.format(
                    "status.backup_exported",
                    &[("path", data["path"].as_str().unwrap_or_default().into())],
                )
            }
            "list" => {
                match serde_json::from_value::<Vec<Host>>(data["hosts"].clone()) {
                    Ok(hosts) => self.hosts = hosts,
                    Err(error) => {
                        self.error =
                            Some(t.format("status.hosts_error", &[("error", error.to_string())]));
                        return;
                    }
                }
                self.groups = serde_json::from_value(data["groups"].clone()).unwrap_or_default();
                self.ssh_keys = data["keys"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|key| {
                        let text = |field: &str| key[field].as_str().unwrap_or_default().to_owned();
                        KeyRow {
                            id: text("id"),
                            name: text("name"),
                            algorithm: text("algorithm"),
                            fingerprint: text("fingerprint"),
                        }
                    })
                    .collect();
                self.path = data["path"].as_str().unwrap_or_default().into();
                self.encrypted = data["encrypted"].as_bool().unwrap_or(false);
                self.storage_format = data["format"].as_str().unwrap_or_default().into();
                self.schema_version = data["schema_version"].as_u64();
                self.prune_batch();
                if !self.hosts.iter().any(|h| h.id == self.selected) {
                    self.selected = self.hosts.first().map(|h| h.id.clone()).unwrap_or_default();
                }
                self.status = t.format(
                    "status.hosts_loaded",
                    &[("count", self.hosts.len().to_string())],
                );
            }
            "save" => {
                self.selected = data["id"].as_str().unwrap_or_default().into();
                self.editor = None;
                self.send(json!({"action":"list"}));
            }
            "delete" => {
                self.delete_id = None;
                self.selected.clear();
                self.send(json!({"action":"list"}));
            }
            "group_add" | "group_rename" | "group_delete" => {
                self.group_dialog = None;
                self.send(json!({"action":"list"}));
            }
            "arrange" | "group_collapse" => self.status = t.get("group.saved").into(),
            "key_add" => {
                if let Some(dialog) = &mut self.key_dialog {
                    dialog.name.clear();
                    dialog.private_key.zeroize();
                    dialog.passphrase.zeroize();
                    dialog.error.clear();
                }
                let name = data["key"]["name"].as_str().unwrap_or_default().to_owned();
                self.send(json!({"action":"list"}));
                self.status = t.format("status.key_added", &[("name", name)]);
            }
            "key_delete" => {
                if let Some(dialog) = &mut self.key_dialog {
                    dialog.delete = None;
                }
                self.send(json!({"action":"list"}));
                self.status = t.get("status.key_deleted").into();
            }
            "probe" => {
                if let Some(host) = self.current() {
                    let actual = data["fingerprint"].as_str().unwrap_or_default().to_owned();
                    if host.fingerprint == actual {
                        self.status = t.get("status.fingerprint_matches").into();
                        if let Some(action) = self.after_trust.take() {
                            self.send(json!({"action":action,"id":self.selected}));
                        }
                    } else {
                        self.trust = Some(TrustDialog {
                            host_id: host.id.clone(),
                            address: format!("{}:{}", host.address, host.port),
                            old: host.fingerprint.clone(),
                            actual,
                            next: self.after_trust.take(),
                        });
                        self.status = t.get("status.check_fingerprint").into();
                    }
                }
            }
            "trust" => {
                if let Some(trust) = self.trust.take() {
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == trust.host_id) {
                        host.fingerprint = trust.actual;
                    }
                    self.status = t.get("status.fingerprint_saved").into();
                    if let Some(action) = trust.next {
                        self.send(json!({"action":action,"id":self.selected}));
                    }
                }
            }
            "test" => {
                let distro = data["distro"].as_str().unwrap_or_default();
                let manager = data["package_manager"].as_str().unwrap_or_default();
                let test = serde_json::from_value::<ConnectionTest>(data["test"].clone()).ok();
                if let Some(host) = self.hosts.iter_mut().find(|h| h.id == self.selected) {
                    host.distro = distro.into();
                    host.package_manager = manager.into();
                    host.last_test = test;
                }
                self.prune_batch();
                let os = if distro.is_empty() {
                    data["os"].as_str().unwrap_or_default()
                } else {
                    distro
                };
                self.status = t.format(
                    "status.connected",
                    &[
                        ("os", os.into()),
                        ("manager", manager_label(t, manager).into()),
                    ],
                )
            }
            "reboot" => {
                let name = self.current().map(|h| h.name.clone()).unwrap_or_default();
                self.status = t.format("reboot.scheduled", &[("name", name)]);
            }
            "check" | "upgrade" => match serde_json::from_value::<CheckResult>(data) {
                Ok(result) => {
                    self.status = match (action, result.status == "success") {
                        ("check", true) => t.get("status.apt_done"),
                        ("check", false) => t.get("status.apt_failed"),
                        (_, true) => t.get("status.upgrade_done"),
                        (_, false) => t.get("status.upgrade_failed"),
                    }
                    .into();
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == self.selected) {
                        record_result(host, result);
                    }
                    self.prune_batch();
                }
                Err(error) => self.error = Some(error.to_string()),
            },
            "batch" => {
                let (mut succeeded, mut failed) = (0, 0);
                for entry in data["results"].as_array().into_iter().flatten() {
                    let Ok(result) = serde_json::from_value::<CheckResult>(entry["result"].clone())
                    else {
                        failed += 1;
                        continue;
                    };
                    if result.status == "success" {
                        succeeded += 1;
                    } else {
                        failed += 1;
                    }
                    if let Some(host) = self.hosts.iter_mut().find(|h| entry["id"] == h.id.as_str())
                    {
                        record_result(host, result);
                    }
                }
                self.prune_batch();
                let skipped = data["skipped"].as_array().map_or(0, Vec::len);
                let mut counts = vec![
                    ("succeeded", succeeded.to_string()),
                    ("failed", failed.to_string()),
                ];
                self.status = if skipped == 0 {
                    t.format("status.batch_done", &counts)
                } else {
                    counts.push(("skipped", skipped.to_string()));
                    t.format("status.batch_aborted", &counts)
                };
            }
            _ => {}
        }
    }

    fn start_connection_action(&mut self, action: &str) {
        if let Some(host) = self.current() {
            if host.fingerprint.is_empty() {
                self.after_trust = Some(action.into());
                self.send(json!({"action":"probe","id":self.selected}));
            } else {
                self.send(json!({"action":action,"id":self.selected}));
            }
        }
    }

    fn open_editor(&mut self, host: Option<Host>) {
        if self.locked {
            return;
        }
        self.log(Action::EditorOpen, Outcome::Succeeded);
        let host = host.unwrap_or_else(|| Host {
            port: 22,
            os: "Linux".into(),
            auth_type: "password".into(),
            use_pty: true,
            ..Default::default()
        });
        self.editor = Some(Editor {
            port: host.port.to_string(),
            saved_key: host.auth_type == "key" && !host.key_id.is_empty(),
            host,
            show_secrets: false,
            error: String::new(),
        });
    }

    fn open_key_dialog(&mut self) {
        if self.locked {
            return;
        }
        self.log(Action::SshKeyDialog, Outcome::Succeeded);
        self.key_dialog = Some(KeyDialog::default());
    }

    /// Asks for a private key file; None when the user cancels.
    fn pick_key_file(&mut self) -> Option<Result<String, String>> {
        let t = Translator(self.preferences.language);
        self.log(Action::ImportKey, Outcome::Started);
        let Some(path) = rfd::FileDialog::new()
            .set_title(t.get("auth.import_title"))
            .pick_file()
        else {
            self.log(Action::ImportKey, Outcome::Cancelled);
            return None;
        };
        let result = std::fs::metadata(&path).and_then(|m| {
            if m.len() <= 1024 * 1024 {
                std::fs::read_to_string(&path)
            } else {
                Err(std::io::Error::other(t.get("auth.key_too_large")))
            }
        });
        self.log(
            Action::ImportKey,
            if result.is_ok() {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            },
        );
        Some(
            result
                .map_err(|error| t.format("auth.key_read_error", &[("error", error.to_string())])),
        )
    }

    /// Group IDs in sidebar order, with "" where the ungrouped hosts sit.
    fn section_order(&self) -> Vec<String> {
        let mut order: Vec<String> = self.groups.iter().map(|g| g.id.clone()).collect();
        let at = (self.preferences.ungrouped_at)
            .unwrap_or(order.len())
            .min(order.len());
        order.insert(at, String::new());
        order
    }

    /// Host IDs under each sidebar heading, in sidebar order ("" = ungrouped).
    fn sections(&self) -> Vec<(String, Vec<String>)> {
        let mut sections: Vec<(String, Vec<String>)> = (self.section_order().into_iter())
            .map(|group| (group, Vec::new()))
            .collect();
        let ungrouped = sections.iter().position(|(group, _)| group.is_empty());
        for host in &self.hosts {
            let index = sections
                .iter()
                .position(|(group, _)| *group == host.group_id)
                .or(ungrouped)
                .unwrap_or_default();
            sections[index].1.push(host.id.clone());
        }
        sections
    }

    fn group_header(
        &mut self,
        ui: &mut egui::Ui,
        group: Option<&HostGroup>,
        count: usize,
        collapsed: bool,
        searching: bool,
    ) {
        let t = Translator(self.preferences.language);
        let compact = self.preferences.density == Density::Compact;
        ui.add_space(if compact { 2.0 } else { 4.0 });
        let sense = if searching {
            egui::Sense::hover()
        } else {
            egui::Sense::click_and_drag()
        };
        let height = if compact { 22.0 } else { 26.0 };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
        let name = group.map_or(t.get("group.ungrouped"), |g| g.name.as_str());
        let arrow = if collapsed { "▸ " } else { "▾ " };
        let color = if response.hovered() && !searching {
            Color32::from_rgb(220, 231, 245)
        } else {
            Color32::from_rgb(147, 174, 208)
        };
        let painter = ui.painter();
        painter.text(
            rect.left_center() + egui::vec2(2.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{arrow}{name}"),
            egui::FontId::proportional(13.0),
            color,
        );
        painter.text(
            rect.right_center() - egui::vec2(4.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(11.0),
            Color32::from_rgb(120, 144, 175),
        );
        if searching {
            return;
        }
        response.dnd_set_drag_payload(DragItem::Group(
            group.map_or(String::new(), |g| g.id.clone()),
        ));
        // The ungrouped heading moves and collapses like a group, but is never renamed or deleted.
        let Some(group) = group else {
            if response.clicked() {
                self.preferences.ungrouped_collapsed = !collapsed;
                if let Err(error) = self.preferences.save(&self.settings_path) {
                    self.error = Some(t.format("settings.save_error", &[("error", error)]));
                }
            }
            return;
        };
        if response.clicked() {
            if let Some(stored) = self.groups.iter_mut().find(|g| g.id == group.id) {
                stored.collapsed = !collapsed;
            }
            self.send(json!({"action":"group_collapse","id":group.id,"collapsed":!collapsed}));
        }
        response
            .on_hover_text(t.get("group.hint"))
            .context_menu(|ui| {
                if ui.button(t.get("group.rename")).clicked() {
                    self.group_dialog = Some(GroupDialog {
                        id: Some(group.id.clone()),
                        name: group.name.clone(),
                    });
                }
                if ui
                    .button(t.get("group.delete"))
                    .on_hover_text(t.get("group.delete_hint"))
                    .clicked()
                {
                    self.send(json!({"action":"group_delete","id":group.id}));
                }
            });
    }

    /// Draws one host, `indent` in from the left, and returns where it went, for placing a drop.
    fn host_card(
        &mut self,
        ui: &mut egui::Ui,
        host: &Host,
        indent: f32,
        searching: bool,
        dragged: Option<&DragItem>,
    ) -> egui::Rect {
        let palette = Palette::for_context(ui.ctx());
        let selected = self.selected == host.id;
        let opacity = if dragged == Some(&DragItem::Host(host.id.clone())) {
            0.4
        } else {
            1.0
        };
        let fields = self.preferences.host_card_fields();
        let mut lines = Vec::new();
        if fields.address {
            lines.push(format!("{}@{}:{}", host.username, host.address, host.port));
        }
        if fields.system && !host.distro.is_empty() {
            lines.push(host.distro.clone());
        }
        let note = (host.notes.lines())
            .map(str::trim)
            .find(|line| !line.is_empty())
            .filter(|_| fields.notes);
        lines.extend(note.map(str::to_owned));
        let success = host
            .last_check
            .as_ref()
            .is_some_and(|r| r.status == "success");
        let dot = if success {
            Color32::from_rgb(78, 201, 162)
        } else {
            palette.muted
        };
        let detail_color = Color32::from_rgb(166, 187, 213);
        let fill = if selected {
            Color32::from_rgb(40, 66, 95)
        } else {
            NAVY
        };
        let compact = self.preferences.density == Density::Compact;
        let rect = if compact {
            // One line like a file tree: the details follow the name, clipped at the edge.
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
            let rect = rect.with_min_x(rect.min.x + indent);
            let painter = ui.painter_at(rect);
            if selected || ui.rect_contains_pointer(rect) {
                let fill = if selected {
                    fill
                } else {
                    Color32::from_rgb(29, 49, 74)
                };
                painter.rect_filled(rect, 4, fill.gamma_multiply(opacity));
            }
            let mut x = rect.left() + 6.0;
            if fields.status {
                let center = egui::pos2(x + 3.0, rect.center().y);
                painter.circle_filled(center, 3.5, dot.gamma_multiply(opacity));
                x += 13.0;
            }
            let name = painter.layout_no_wrap(
                host.name.clone(),
                egui::FontId::proportional(13.0),
                Color32::WHITE.gamma_multiply(opacity),
            );
            let name_width = name.size().x;
            let top = rect.center().y - name.size().y / 2.0;
            painter.galley(egui::pos2(x, top), name, Color32::WHITE);
            if !lines.is_empty() {
                painter.text(
                    egui::pos2(x + name_width + 8.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    lines.join("  ·  "),
                    egui::FontId::proportional(11.0),
                    detail_color.gamma_multiply(opacity),
                );
            }
            rect
        } else {
            // A card with only its name is shorter, to fit more hosts.
            let margin = if lines.is_empty() {
                egui::Margin::symmetric(12, 7)
            } else {
                egui::Margin::same(12)
            };
            let card = egui::Frame::new()
                .fill(fill)
                .corner_radius(7)
                .inner_margin(margin)
                .outer_margin(egui::Margin {
                    left: indent as i8,
                    ..Default::default()
                })
                .multiply_with_opacity(opacity)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width() - 2.0);
                    ui.horizontal(|ui| {
                        if fields.status {
                            ui.label(RichText::new("●").size(10.0).color(dot));
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(&host.name)
                                    .strong()
                                    .size(15.0)
                                    .color(Color32::WHITE),
                            )
                            .truncate(),
                        );
                    });
                    for line in &lines {
                        ui.add(
                            egui::Label::new(RichText::new(line).size(11.0).color(detail_color))
                                .truncate(),
                        );
                    }
                })
                .response;
            ui.add_space(5.0);
            card.rect.with_min_x(card.rect.min.x + indent)
        };
        // Dragging is off while searching, where the list hides some hosts.
        let sense = if searching {
            egui::Sense::click()
        } else {
            egui::Sense::click_and_drag()
        };
        let mut response = ui
            .interact(rect, ui.scope_id().with(("host_card", &host.id)), sense)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        // A compact row may clip its details; the tooltip shows them whole.
        if compact && !lines.is_empty() {
            response = response.on_hover_text(lines.join("\n"));
        }
        response.dnd_set_drag_payload(DragItem::Host(host.id.clone()));
        if response.clicked() {
            self.selected = host.id.clone();
            self.log(Action::HostSelect, Outcome::Succeeded);
        }
        rect
    }

    /// Shows what is being dragged and where it would land, and drops it on release.
    fn drag_feedback(&mut self, ui: &egui::Ui, item: &DragItem, layout: &SidebarLayout) {
        let Some(pointer) = ui.ctx().pointer_interact_pos() else {
            return;
        };
        let (name, drop) = match item {
            DragItem::Host(id) => (
                self.hosts
                    .iter()
                    .find(|h| h.id == *id)
                    .map(|h| h.name.clone()),
                layout.host_drop(pointer, id),
            ),
            DragItem::Group(id) if id.is_empty() => (
                Some(
                    Translator(self.preferences.language)
                        .get("group.ungrouped")
                        .to_owned(),
                ),
                layout.group_drop(pointer, id),
            ),
            DragItem::Group(id) => (
                self.groups
                    .iter()
                    .find(|g| g.id == *id)
                    .map(|g| g.name.clone()),
                layout.group_drop(pointer, id),
            ),
        };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("sidebar_drag"),
        ));
        let galley = painter.layout_no_wrap(
            name.unwrap_or_default(),
            egui::FontId::proportional(13.0),
            Color32::WHITE,
        );
        let label = egui::Rect::from_min_size(
            pointer + egui::vec2(14.0, 6.0),
            galley.size() + egui::vec2(16.0, 8.0),
        );
        painter.rect_filled(label, 5, BLUE);
        painter.galley(label.min + egui::vec2(8.0, 4.0), galley, Color32::WHITE);
        if let Some((_, line)) = &drop {
            painter.hline(
                layout.visible.x_range(),
                *line,
                Stroke::new(2.0, Color32::from_rgb(120, 190, 255)),
            );
        }
        if !ui.input(|i| i.pointer.any_released()) {
            return;
        }
        egui::DragAndDrop::clear_payload(ui.ctx());
        match (item, drop) {
            (DragItem::Host(id), Some((DropAt::Host { group, before }, _))) => {
                self.move_host(id, &group, before.as_deref());
            }
            (DragItem::Group(id), Some((DropAt::Group { before }, _))) => {
                self.move_group(id, before.as_deref());
            }
            _ => {}
        }
    }

    fn move_host(&mut self, id: &str, group: &str, before: Option<&str>) {
        let mut sections = self.sections();
        for (_, ids) in &mut sections {
            ids.retain(|host| host != id);
        }
        if let Some((_, ids)) = sections.iter_mut().find(|(g, _)| g == group) {
            let at = before
                .and_then(|b| ids.iter().position(|host| host == b))
                .unwrap_or(ids.len());
            ids.insert(at, id.to_owned());
        }
        let order = self.groups.iter().map(|g| g.id.clone()).collect();
        self.arrange(&sections, order);
    }

    /// Moves a group, or with an empty `id` the ungrouped hosts, before another or to the end.
    fn move_group(&mut self, id: &str, before: Option<&str>) {
        let mut order: Vec<String> = (self.section_order().into_iter())
            .filter(|g| g != id)
            .collect();
        let at = before
            .and_then(|b| order.iter().position(|g| g == b))
            .unwrap_or(order.len());
        order.insert(at, id.to_owned());
        // Where the ungrouped hosts sit is a display preference, not part of the host store.
        let ungrouped =
            (order.iter().position(String::is_empty)).filter(|&at| at + 1 < order.len());
        if ungrouped != self.preferences.ungrouped_at {
            self.preferences.ungrouped_at = ungrouped;
            if let Err(error) = self.preferences.save(&self.settings_path) {
                let t = Translator(self.preferences.language);
                self.error = Some(t.format("settings.save_error", &[("error", error)]));
            }
        }
        order.retain(|g| !g.is_empty());
        self.arrange(&self.sections(), order);
    }

    /// Shows a drag's result at once and saves it; a refused one reloads the stored order.
    fn arrange(&mut self, sections: &[(String, Vec<String>)], order: Vec<String>) {
        let placement = |hosts: &[Host]| -> Vec<(String, String)> {
            hosts
                .iter()
                .map(|h| (h.id.clone(), h.group_id.clone()))
                .collect()
        };
        let before = placement(&self.hosts);
        let unchanged_groups = self.groups.iter().map(|g| &g.id).eq(order.iter());
        let mut remaining = std::mem::take(&mut self.hosts);
        for group in std::iter::once("").chain(order.iter().map(String::as_str)) {
            let Some((_, ids)) = sections.iter().find(|(g, _)| g == group) else {
                continue;
            };
            for id in ids {
                if let Some(index) = remaining.iter().position(|h| h.id == *id) {
                    let mut host = remaining.remove(index);
                    host.group_id = group.to_owned();
                    self.hosts.push(host);
                }
            }
        }
        self.hosts.append(&mut remaining);
        let after = placement(&self.hosts);
        if unchanged_groups && before == after {
            return;
        }
        self.groups
            .sort_by_key(|g| order.iter().position(|id| *id == g.id));
        let hosts: Vec<Value> = after
            .iter()
            .map(|(id, group)| json!({"id": id, "group_id": group}))
            .collect();
        self.send(json!({"action":"arrange","groups":order,"hosts":hosts}));
    }

    fn group_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.group_dialog.take() else {
            return;
        };
        if self.locked {
            return;
        }
        let t = Translator(self.preferences.language);
        let (mut save, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("group_dialog")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(if dialog.id.is_some() {
                t.get("group.rename_title")
            } else {
                t.get("group.add_title")
            });
            ui.add_space(8.0);
            ui.label(t.get("group.name"));
            let edit = ui.add(
                egui::TextEdit::singleline(&mut dialog.name)
                    .char_limit(64)
                    .desired_width(f32::INFINITY),
            );
            if ui.memory(|m| m.focused().is_none()) {
                edit.request_focus();
            }
            save = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(12.0);
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal(|ui| {
                    save |= ui
                        .add_enabled(!dialog.name.trim().is_empty(), primary(t.get("group.save")))
                        .clicked();
                    cancel = ui.button(t.get("common.cancel")).clicked();
                });
            });
        });
        if cancel || modal.should_close() {
            return;
        }
        if save && !self.busy && !dialog.name.trim().is_empty() {
            self.send(match &dialog.id {
                Some(id) => json!({"action":"group_rename","id":id,"name":dialog.name}),
                None => json!({"action":"group_add","name":dialog.name}),
            });
        }
        self.group_dialog = Some(dialog);
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        ui.visuals_mut().override_text_color = Some(Color32::from_rgb(220, 231, 245));
        ui.add_space(9.0);
        ui.label(
            RichText::new("MISF")
                .size(34.0)
                .strong()
                .color(Color32::WHITE),
        );
        ui.label(
            RichText::new("MANAGEMENT TERMINAL")
                .size(10.0)
                .color(Color32::from_rgb(143, 166, 197)),
        );
        ui.add_space(23.0);
        ui.scope(|ui| {
            ui.visuals_mut().override_text_color = Some(palette.ink);
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.search)
                        .text_color(palette.ink)
                        .background_color(palette.surface)
                        .hint_text(RichText::new(t.get("host.search")).color(palette.muted))
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.log(Action::HostSearch, Outcome::Succeeded);
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(t.format(
                    "host.list_count",
                    &[("count", self.hosts.len().to_string())],
                ))
                .size(12.0)
                .color(Color32::from_rgb(147, 174, 208)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let add = ui
                    .add_enabled(
                        !self.locked,
                        egui::Button::new(RichText::new(t.get("group.add")).size(12.0))
                            .frame(false),
                    )
                    .on_hover_text(t.get("group.hint"));
                if add.clicked() {
                    self.group_dialog = Some(GroupDialog::default());
                }
            });
        });
        ui.add_space(6.0);
        let query = self.search.to_lowercase();
        let searching = !query.is_empty();
        let matches = |h: &Host| {
            format!("{} {} {}", h.name, h.address, h.notes)
                .to_lowercase()
                .contains(&query)
        };
        let any_visible = self.hosts.iter().any(matches);
        let show_headers = !self.groups.is_empty();
        let dragged = egui::DragAndDrop::payload::<DragItem>(ui.ctx());
        let mut layout = SidebarLayout {
            visible: egui::Rect::NOTHING,
            sections: Vec::new(),
            cards: Vec::new(),
        };
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 98.0).max(80.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                layout.visible = ui.clip_rect();
                if self.preferences.density == Density::Compact {
                    ui.spacing_mut().item_spacing.y = 0.0;
                }
                if !any_visible && !show_headers {
                    ui.add_space(30.0);
                    ui.label(
                        RichText::new(if self.hosts.is_empty() {
                            t.get("host.empty")
                        } else {
                            t.get("host.no_match")
                        })
                        .color(Color32::from_rgb(143, 166, 197)),
                    );
                }
                for (group_id, ids) in self.sections() {
                    let hosts: Vec<Host> = ids
                        .iter()
                        .filter_map(|id| self.hosts.iter().find(|h| h.id == *id))
                        .filter(|h| matches(h))
                        .cloned()
                        .collect();
                    if searching && hosts.is_empty() {
                        continue;
                    }
                    let group = self.groups.iter().find(|g| g.id == group_id).cloned();
                    let collapsed = !searching
                        && group
                            .as_ref()
                            .map_or(self.preferences.ungrouped_collapsed, |g| g.collapsed);
                    let top = ui.cursor().top();
                    if show_headers {
                        self.group_header(ui, group.as_ref(), hosts.len(), collapsed, searching);
                    }
                    // Hosts in a group sit under its heading; ungrouped ones stay at the edge.
                    let indent = if show_headers && !group_id.is_empty() {
                        16.0
                    } else {
                        0.0
                    };
                    if !collapsed {
                        for host in hosts {
                            let rect =
                                self.host_card(ui, &host, indent, searching, dragged.as_deref());
                            layout.cards.push((rect, group_id.clone(), host.id));
                        }
                    }
                    let bottom = ui.cursor().top();
                    let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=bottom);
                    layout.sections.push((rect, group_id));
                }
            });
        if let Some(item) = dragged {
            self.drag_feedback(ui, &item, &layout);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.label(
                RichText::new(t.get("app.local_mode"))
                    .size(11.0)
                    .color(Color32::from_rgb(143, 166, 197)),
            );
            if ui
                .add_enabled(
                    !self.locked,
                    primary(t.get("host.add_button")).min_size([ui.available_width(), 38.0].into()),
                )
                .clicked()
            {
                self.open_editor(None);
            }
        });
    }

    fn workspace(&mut self, ui: &mut egui::Ui, viewport_height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        if self.locked {
            ui.label(
                RichText::new(t.get("app.vault_heading"))
                    .size(10.0)
                    .strong()
                    .color(palette.muted),
            );
            ui.add_space(12.0);
            let setup = !self.encrypted;
            ui.heading(
                RichText::new(if setup {
                    t.get("vault.setup_heading")
                } else {
                    t.get("vault.locked")
                })
                .size(29.0),
            );
            ui.label(if setup {
                t.get("vault.setup_description")
            } else {
                t.get("vault.unlock_description")
            });
            ui.add_space(24.0);
            card(ui).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add_space(15.0);
                ui.label(t.get("vault.algorithm"));
                ui.add_space(12.0);
                if setup {
                    if ui.add(primary(t.get("vault.enable_button"))).clicked() {
                        self.open_vault_dialog(VaultMode::Create);
                    }
                } else if ui.add(primary(t.get("vault.unlock_host"))).clicked() {
                    self.open_vault_dialog(VaultMode::Unlock);
                }
                ui.add_space(15.0);
            });
            return;
        }
        ui.horizontal(|ui| {
            let before = self.view;
            ui.selectable_value(&mut self.view, View::Host, t.get("view.host"));
            ui.selectable_value(
                &mut self.view,
                View::Batch,
                t.format(
                    "view.batch",
                    &[("count", self.batch_selected.len().to_string())],
                ),
            );
            if before != self.view {
                self.log(Action::MainTab, Outcome::Succeeded);
            }
        });
        ui.add_space(8.0);
        if self.view == View::Batch {
            self.batch_view(ui, viewport_height);
            return;
        }
        let host = self.current().cloned();
        ui.label(
            RichText::new(t.get("app.host_overview"))
                .size(10.0)
                .strong()
                .color(palette.muted),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(
                    host.as_ref()
                        .map(|h| h.name.as_str())
                        .unwrap_or(t.get("host.first_title")),
                )
                .size(29.0)
                .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let delete = ui.add_enabled(
                    host.is_some(),
                    egui::Button::new(RichText::new(t.get("common.delete")).color(palette.error)),
                );
                if hint(
                    delete,
                    t,
                    t.get("hint.delete_title"),
                    &[],
                    t.get("hint.delete_effect"),
                    t.get("hint.delete_risk"),
                )
                .clicked()
                {
                    self.delete_id = Some(self.selected.clone());
                    self.log(Action::DeletePrompt, Outcome::Succeeded);
                }
                if ui
                    .add_enabled(host.is_some(), egui::Button::new(t.get("host.edit")))
                    .clicked()
                {
                    self.open_editor(host.clone());
                }
            });
        });
        let Some(host) = host else {
            ui.label(RichText::new(t.get("host.intro")).color(palette.muted));
            ui.add_space(50.0);
            card(ui).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add_space(24.0);
                ui.label(RichText::new(t.get("host.start")).size(23.0).strong());
                ui.add_space(9.0);
                ui.label(t.get("host.start_description"));
                ui.add_space(18.0);
                if ui.add(primary(t.get("host.add_first"))).clicked() {
                    self.open_editor(None);
                }
                ui.add_space(24.0);
            });
            return;
        };
        ui.label(
            RichText::new(format!(
                "{}@{}:{}  ·  {}  ·  {}",
                host.username,
                host.address,
                host.port,
                system_label(t, &host),
                if host.auth_type == "key" {
                    "SSH Key"
                } else {
                    t.get("auth.password_login")
                }
            ))
            .color(palette.muted),
        );
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            let test = ui.button(t.get("host.test"));
            if hint(
                test,
                t,
                t.get("hint.test_title"),
                &[package_manager::DETECT],
                t.get("hint.test_effect"),
                t.get("hint.test_risk"),
            )
            .clicked()
            {
                self.start_connection_action("test");
            }
            let probe = ui.button(t.get("host.fingerprint"));
            if hint(
                probe,
                t,
                t.get("hint.probe_title"),
                &[],
                t.get("hint.probe_effect"),
                t.get("hint.probe_risk"),
            )
            .clicked()
            {
                self.after_trust = None;
                self.send(json!({"action":"probe","id":self.selected}));
            }
            let detected = package_manager::Manager::from_id(host.manager());
            let manager = [("manager", manager_label(t, host.manager()).to_owned())];
            let (check_label, upgrade_label, check_effect) = match detected {
                Some(_) => (
                    t.format("apt.check", &manager),
                    t.format("apt.upgrade", &manager),
                    t.get("hint.check_effect").to_owned(),
                ),
                None => (
                    t.get("apt.check_detect").to_owned(),
                    t.get("apt.upgrade_detect").to_owned(),
                    format!(
                        "{}\n{}",
                        t.get("hint.detect_note"),
                        t.get("hint.check_effect")
                    ),
                ),
            };
            let check = ui.add(primary(&check_label));
            if hint(
                check,
                t,
                &t.format("hint.check_title", &manager),
                &package_manager::package_commands(&host, false),
                &check_effect,
                t.get("hint.check_risk"),
            )
            .clicked()
            {
                self.start_connection_action("check");
            }
            // An upgrade confirmation must show real commands, so it waits for detection.
            let upgrade = ui.add_enabled(detected.is_some(), danger(&upgrade_label));
            if hint(
                upgrade.on_disabled_hover_text(t.get("apt.upgrade_needs_detect")),
                t,
                &t.format("hint.upgrade_title", &manager),
                &package_manager::package_commands(&host, true),
                t.get("hint.upgrade_effect"),
                t.get("hint.upgrade_risk"),
            )
            .clicked()
            {
                self.prompt_upgrade(vec![host.id.clone()], false);
            }
            // Only APT hosts with a confirmed key; the command is fixed per account type.
            let can_reboot = host.manager() == "apt" && !host.fingerprint.is_empty();
            let reboot = ui.add_enabled(can_reboot, danger(t.get("reboot.button")));
            if hint(
                reboot.on_disabled_hover_text(t.get("reboot.needs_apt")),
                t,
                t.get("hint.reboot_title"),
                &[misf_core::reboot::command(&host)],
                t.get("hint.reboot_effect"),
                t.get("hint.reboot_risk"),
            )
            .clicked()
            {
                self.reboot_confirm = Some(RebootConfirm {
                    id: host.id.clone(),
                    name: host.name.clone(),
                    address: host.address.clone(),
                    command: misf_core::reboot::command(&host),
                    acknowledged: false,
                    final_step: false,
                });
                self.log(Action::RebootPrompt, Outcome::Succeeded);
            }
            if ui.button(t.get("common.reload")).clicked() {
                self.send(json!({"action":"list"}));
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(if host.fingerprint.is_empty() {
                t.get("host.first_trust").into()
            } else {
                t.format(
                    "host.fingerprint_value",
                    &[("fingerprint", host.fingerprint.clone())],
                )
            })
            .size(11.0)
            .color(palette.muted),
        );
        ui.add_space(12.0);
        let result = host.last_check.as_ref();
        let successful = result.is_some_and(|r| r.status == "success");
        ui.columns(3, |columns| {
            stat(
                &mut columns[0],
                t.get("apt.pending"),
                &if successful {
                    result.unwrap().packages.len().to_string()
                } else {
                    "—".into()
                },
            );
            stat(&mut columns[1], t.get("apt.status"), status_label(t, &host));
            stat(
                &mut columns[2],
                t.get("apt.last_check"),
                &result
                    .map(|r| format_time(&r.checked_at))
                    .unwrap_or("—".into()),
            );
        });
        if let Some(restart) = result.and_then(|r| r.restart.as_ref())
            && let Some(summary) = restart_summary(t, restart)
        {
            ui.add_space(10.0);
            ui.colored_label(
                palette.warning,
                t.format("restart.banner", &[("summary", summary)]),
            );
        }
        ui.add_space(16.0);
        self.result_panel(ui, &host, (viewport_height - 500.0).max(100.0));
    }

    /// Pending, upgraded and raw output for one host, shared by the host and batch views.
    fn result_panel(&mut self, ui: &mut egui::Ui, host: &Host, available: f32) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        let result = host.last_check.as_ref();
        let upgrade = result.filter(|r| r.operation == "upgrade");
        card(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let before = self.tab;
                ui.selectable_value(&mut self.tab, 0, t.get("apt.packages"));
                ui.selectable_value(&mut self.tab, 1, t.get("apt.upgraded"));
                ui.selectable_value(&mut self.tab, 4, t.get("restart.tab"));
                ui.selectable_value(&mut self.tab, 2, t.get("apt.output"));
                ui.selectable_value(&mut self.tab, 3, t.get("conn.tab"));
                if before != self.tab {
                    self.log(Action::ResultTab, Outcome::Succeeded);
                }
            });
            ui.separator();
            if self.tab == 4 {
                egui::ScrollArea::vertical()
                    .id_salt("restart_info")
                    .max_height(available)
                    .auto_shrink([false, false])
                    .show(ui, |ui| restart_panel(ui, t, host));
                return;
            }
            if self.tab == 3 {
                egui::ScrollArea::vertical()
                    .id_salt("connection_info")
                    .max_height(available)
                    .auto_shrink([false, false])
                    .show(ui, |ui| connection_panel(ui, t, host.last_test.as_ref()));
                return;
            }
            if self.tab == 2 {
                egui::ScrollArea::both()
                    .id_salt("apt_output")
                    .max_height(available)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let text = result
                            .map(|r| format!("{}\n{}", r.output, r.error))
                            .unwrap_or(t.get("apt.not_run").into());
                        ui.add(
                            egui::Label::new(
                                RichText::new(misf_core::transcript::readable_output(&text))
                                    .monospace()
                                    .size(12.0),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend)
                            .selectable(true),
                        );
                    });
                return;
            }
            let (message, failed, packages, headers) = if self.tab == 1 {
                let message = match upgrade {
                    Some(r) if r.status == "success" && r.upgraded.is_empty() => {
                        t.get("apt.upgraded_none")
                    }
                    Some(r) if r.status == "success" => t.get("apt.upgraded_description"),
                    Some(r) => r.error.as_str(),
                    None => t.get("apt.upgrade_not_run"),
                };
                (
                    message,
                    upgrade.is_some_and(|r| r.status == "failed"),
                    upgrade.map(|r| r.upgraded.as_slice()).unwrap_or_default(),
                    [
                        t.get("apt.package"),
                        t.get("apt.previous"),
                        t.get("apt.installed"),
                        t.get("apt.architecture"),
                    ],
                )
            } else {
                let message = match result {
                    Some(r) if r.status == "success" => {
                        match (r.operation == "upgrade", r.packages.is_empty()) {
                            (false, true) => t.get("apt.up_to_date"),
                            (false, false) => t.get("apt.available_description"),
                            (true, true) => t.get("apt.upgrade_clean"),
                            (true, false) => t.get("apt.held_back"),
                        }
                    }
                    Some(r) => r.error.as_str(),
                    None => t.get("apt.check_hint"),
                };
                (
                    message,
                    result.is_some_and(|r| r.status == "failed"),
                    result.map(|r| r.packages.as_slice()).unwrap_or_default(),
                    [
                        t.get("apt.package"),
                        t.get("apt.current"),
                        t.get("apt.candidate"),
                        t.get("apt.architecture"),
                    ],
                )
            };
            ui.label(RichText::new(message).color(if failed {
                palette.error
            } else {
                palette.muted
            }));
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.package_search)
                        .hint_text(t.get("apt.filter"))
                        .desired_width(270.0),
                )
                .changed()
            {
                self.log(Action::PackageSearch, Outcome::Succeeded);
            }
            ui.add_space(5.0);
            let filter = self.package_search.to_lowercase();
            egui::ScrollArea::both()
                .id_salt("apt_packages")
                .max_height((available - 75.0).max(70.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("packages")
                        .striped(true)
                        .min_col_width(130.0)
                        .spacing([20.0, 14.0])
                        .show(ui, |ui| {
                            for caption in headers {
                                ui.label(RichText::new(caption).strong().color(palette.muted));
                            }
                            ui.end_row();
                            for package in packages
                                .iter()
                                .filter(|p| p.name.to_lowercase().contains(&filter))
                            {
                                ui.label(&package.name);
                                ui.label(&package.current_version);
                                ui.label(&package.candidate_version);
                                ui.label(&package.architecture);
                                ui.end_row();
                            }
                        });
                    if packages.is_empty() {
                        ui.add_space(28.0);
                        ui.label(RichText::new(t.get("apt.no_data")).color(palette.muted));
                    }
                });
        });
    }

    /// Keeps the selection to ready hosts of one package manager, defaulting to the most common.
    fn prune_batch(&mut self) {
        let managers = batch_managers(&self.hosts);
        if !managers.iter().any(|(id, _)| *id == self.batch_manager) {
            self.batch_manager = managers
                .first()
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
        }
        let (hosts, manager) = (&self.hosts, &self.batch_manager);
        self.batch_selected.retain(|id| {
            hosts
                .iter()
                .any(|h| h.id == *id && batch_ready(h) && h.manager() == manager)
        });
    }

    fn batch_view(&mut self, ui: &mut egui::Ui, viewport_height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        ui.label(RichText::new(t.get("batch.title")).size(29.0).strong());
        ui.label(RichText::new(t.get("batch.description")).color(palette.muted));
        ui.add_space(10.0);
        self.prune_batch();
        let managers = batch_managers(&self.hosts);
        ui.horizontal_wrapped(|ui| {
            ui.label(t.get("batch.manager"));
            if managers.is_empty() {
                ui.label(RichText::new(t.get("batch.no_manager")).color(palette.muted));
            }
            for (id, count) in &managers {
                let label = format!("{} ({count})", manager_label(t, id));
                if ui
                    .selectable_label(self.batch_manager == *id, label)
                    .clicked()
                    && self.batch_manager != *id
                {
                    self.batch_manager = id.clone();
                    self.batch_selected.clear();
                    self.log(Action::BatchSelect, Outcome::Succeeded);
                }
            }
        });
        let manager = [("manager", manager_label(t, &self.batch_manager).to_owned())];
        let commands = |upgrade| {
            package_manager::Manager::from_id(&self.batch_manager)
                .map(|m| package_manager::manager_commands(m, false, upgrade))
                .unwrap_or_default()
        };
        let (check_commands, upgrade_commands) = (commands(false), commands(true));
        let (order, headings) = self.batch_order(t);
        // Display order, never the set's order, so the request matches what the user sees.
        let selected: Vec<String> = order
            .iter()
            .map(|&i| &self.hosts[i])
            .filter(|h| self.batch_selected.contains(&h.id))
            .map(|h| h.id.clone())
            .collect();
        let ready: BTreeSet<String> = self
            .hosts
            .iter()
            .filter(|h| batch_ready(h) && h.manager() == self.batch_manager)
            .map(|h| h.id.clone())
            .collect();
        let count = [("count", selected.len().to_string()), manager[0].clone()];
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !ready.is_empty(),
                    egui::Button::new(t.get("batch.select_all")),
                )
                .clicked()
            {
                self.batch_selected = ready.clone();
                self.log(Action::BatchSelect, Outcome::Succeeded);
            }
            if ui
                .add_enabled(
                    !selected.is_empty(),
                    egui::Button::new(t.get("batch.clear")),
                )
                .clicked()
            {
                self.batch_selected.clear();
                self.log(Action::BatchSelect, Outcome::Succeeded);
            }
            ui.label(t.format("batch.selected", &count));
            let check_label = t.format("batch.check", &count);
            let check = ui.add_enabled(!selected.is_empty(), primary(&check_label));
            if hint(
                check,
                t,
                &t.format("hint.batch_check_title", &manager),
                &check_commands,
                t.get("hint.batch_check_effect"),
                t.get("hint.batch_check_risk"),
            )
            .clicked()
            {
                self.send(json!({"action":"batch","operation":"check","ids":selected}));
            }
            let upgrade_label = t.format("batch.upgrade", &count);
            let upgrade = ui.add_enabled(!selected.is_empty(), danger(&upgrade_label));
            if hint(
                upgrade,
                t,
                &t.format("hint.batch_upgrade_title", &manager),
                &upgrade_commands,
                t.get("hint.batch_upgrade_effect"),
                t.get("hint.batch_upgrade_risk"),
            )
            .clicked()
            {
                self.prompt_upgrade(selected.clone(), true);
            }
        });
        ui.add_space(8.0);
        // Only display fields are copied out of the host list; credentials stay in place.
        let rows: Vec<BatchRow> = order
            .iter()
            .map(|&i| &self.hosts[i])
            .map(|h| {
                let reason = if h.fingerprint.is_empty() {
                    Some(t.get("batch.untrusted").to_owned())
                } else if !batch_ready(h) {
                    Some(t.get("batch.undetected").to_owned())
                } else if h.manager() != self.batch_manager {
                    Some(t.format(
                        "batch.other_manager",
                        &[("manager", manager_label(t, h.manager()).into())],
                    ))
                } else {
                    None
                };
                let result = h.last_check.as_ref();
                let success = result.filter(|r| r.status == "success");
                let upgrade = success.filter(|r| r.operation == "upgrade");
                let dash = || "—".to_owned();
                let cells = BatchColumn::ALL.map(|column| match column {
                    BatchColumn::Host => h.name.clone(),
                    BatchColumn::User => h.username.clone(),
                    BatchColumn::Address => format!("{}:{}", h.address, h.port),
                    BatchColumn::System if h.distro.is_empty() => dash(),
                    BatchColumn::System => system_label(t, h),
                    BatchColumn::Operation => result.map_or_else(dash, |r| {
                        if r.operation == "upgrade" {
                            t.get("batch.operation_upgrade").to_owned()
                        } else {
                            t.get("batch.operation_check").to_owned()
                        }
                    }),
                    BatchColumn::Status => status_label(t, h).to_owned(),
                    BatchColumn::Pending => {
                        success.map_or_else(dash, |r| r.packages.len().to_string())
                    }
                    BatchColumn::Phased => {
                        upgrade.map_or_else(dash, |r| r.phased.len().to_string())
                    }
                    BatchColumn::Upgraded => {
                        upgrade.map_or_else(dash, |r| r.upgraded.len().to_string())
                    }
                    BatchColumn::Checked => {
                        result.map_or_else(dash, |r| format_time(&r.checked_at))
                    }
                    BatchColumn::Restart => match result.and_then(|r| r.restart.as_ref()) {
                        Some(restart) => restart_summary(t, restart)
                            .unwrap_or_else(|| t.get("restart.none").to_owned()),
                        None => dash(),
                    },
                });
                BatchRow {
                    id: h.id.clone(),
                    reason,
                    cells,
                    failed: result.is_some_and(|r| r.status == "failed"),
                    phased: upgrade
                        .map(|r| r.phased.join(", "))
                        .filter(|names| !names.is_empty()),
                }
            })
            .collect();
        card(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if rows.is_empty() {
                ui.label(RichText::new(t.get("host.empty")).color(palette.muted));
                return;
            }
            egui::ScrollArea::both()
                .id_salt("batch_hosts")
                .max_height(240.0)
                .auto_shrink([false, true])
                .show(ui, |ui| self.batch_table(ui, t, rows, &headings, &ready));
        });
        let Some(host) = self.current().cloned() else {
            self.batch_popup = false;
            return;
        };
        if self.preferences.batch_detail == BatchDetail::Inline {
            self.batch_popup = false;
            ui.add_space(12.0);
            self.batch_detail_heading(ui, t, &host, &order);
            self.result_panel(ui, &host, (viewport_height - 560.0).max(160.0));
            return;
        }
        if !self.batch_popup {
            return;
        }
        let screen = ui.ctx().content_rect();
        let modal = egui::Modal::new(egui::Id::unique("batch_detail")).show(ui.ctx(), |ui| {
            ui.set_width((screen.width() - 120.0).clamp(480.0, 1400.0));
            let close = ui
                .horizontal(|ui| {
                    self.batch_detail_heading(ui, t, &host, &order);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.button(t.get("common.close")).clicked()
                    })
                    .inner
                })
                .inner;
            self.result_panel(ui, &host, (screen.height() - 300.0).max(160.0));
            close
        });
        // Escape and a click outside close it too.
        if modal.inner || modal.should_close() {
            self.batch_popup = false;
        }
    }

    /// Previous and next buttons and the title above the chosen host's packages.
    fn batch_detail_heading(
        &mut self,
        ui: &mut egui::Ui,
        t: Translator,
        host: &Host,
        order: &[usize],
    ) {
        ui.horizontal(|ui| {
            // Previous and next follow the table's sorted order.
            let index = order.iter().position(|&i| self.hosts[i].id == host.id);
            let previous = index.and_then(|i| i.checked_sub(1));
            let next = index.map(|i| i + 1).filter(|i| *i < order.len());
            if ui
                .add_enabled(
                    previous.is_some(),
                    egui::Button::new(t.get("batch.previous")),
                )
                .clicked()
                && let Some(i) = previous
            {
                self.selected = self.hosts[order[i]].id.clone();
                self.log(Action::HostSelect, Outcome::Succeeded);
            }
            if ui
                .add_enabled(next.is_some(), egui::Button::new(t.get("batch.next")))
                .clicked()
                && let Some(i) = next
            {
                self.selected = self.hosts[order[i]].id.clone();
                self.log(Action::HostSelect, Outcome::Succeeded);
            }
            ui.label(
                RichText::new(t.format("batch.detail", &[("name", host.name.clone())]))
                    .size(18.0)
                    .strong(),
            );
        });
    }

    /// Host indexes in table order, and in sidebar order each group's heading:
    /// the row it comes before, its name and its host IDs.
    #[allow(clippy::type_complexity)]
    fn batch_order(&self, t: Translator) -> (Vec<usize>, Vec<(usize, String, Vec<String>)>) {
        // Stable sorts keep the sidebar order among equals.
        let mut order: Vec<usize> = (0..self.hosts.len()).collect();
        let mut headings = Vec::new();
        match self.preferences.batch_order {
            BatchOrder::Sidebar if !self.groups.is_empty() => {
                order.clear();
                for (group, ids) in self.sections() {
                    if ids.is_empty() {
                        continue;
                    }
                    let name = self
                        .groups
                        .iter()
                        .find(|g| g.id == group)
                        .map_or(t.get("group.ungrouped"), |g| g.name.as_str());
                    headings.push((order.len(), name.to_owned(), ids.clone()));
                    order.extend(
                        ids.iter()
                            .filter_map(|id| self.hosts.iter().position(|h| h.id == *id)),
                    );
                }
            }
            BatchOrder::Sidebar => {}
            BatchOrder::Pending => {
                order.sort_by_key(|&i| std::cmp::Reverse(pending_count(&self.hosts[i])));
            }
            BatchOrder::Recent => order.sort_by_key(|&i| {
                std::cmp::Reverse(self.hosts[i].last_check.as_ref().and_then(|r| {
                    DateTime::parse_from_rfc3339(&r.checked_at)
                        .ok()
                        .map(|time| time.timestamp())
                }))
            }),
            BatchOrder::Name => order.sort_by_cached_key(|&i| self.hosts[i].name.to_lowercase()),
        }
        (order, headings)
    }

    /// The batch table: the columns share its whole width in the ratios the user dragged.
    fn batch_table(
        &mut self,
        ui: &mut egui::Ui,
        t: Translator,
        rows: Vec<BatchRow>,
        headings: &[(usize, String, Vec<String>)],
        ready: &BTreeSet<String>,
    ) {
        const CHECK: f32 = 40.0;
        const ROW: f32 = 34.0;
        const MIN: f32 = 52.0;
        let palette = Palette::for_context(ui.ctx());
        let columns = self.preferences.batch_columns.visible();
        let width = ui.available_width().max(CHECK + MIN * columns.len() as f32);
        let spare = width - CHECK;
        let weights = self.preferences.batch_columns.weights;
        let total: f32 = columns.iter().map(|(i, _)| weights[*i]).sum();
        // Each column's left edge from the table's left, then the table's right edge.
        let mut edges = vec![CHECK];
        for (i, _) in &columns {
            edges.push(edges[edges.len() - 1] + spare * weights[*i] / total);
        }
        let next_row = |ui: &mut egui::Ui| {
            ui.allocate_exact_size(egui::vec2(width, ROW), egui::Sense::hover())
                .0
        };
        let span = |row: egui::Rect, from: f32, to: f32| {
            egui::Rect::from_x_y_ranges(row.left() + from..=row.left() + to, row.y_range())
        };
        let stripe = ui.visuals().faint_bg_color;

        let header = next_row(ui);
        for (k, (_, column)) in columns.iter().enumerate() {
            table_cell(ui, span(header, edges[k], edges[k + 1]), ("caption", k)).add(
                egui::Label::new(RichText::new(column.label(t)).strong().color(palette.muted))
                    .truncate(),
            );
        }
        let mut save = false;
        for k in 1..columns.len() {
            let x = header.left() + edges[k];
            let handle = egui::Rect::from_center_size(
                egui::pos2(x, header.center().y),
                egui::vec2(8.0, ROW),
            );
            let response = ui
                .interact(
                    handle,
                    ui.scope_id().with(("batch_column_edge", k)),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_text(t.get("batch_columns.drag_hint"));
            let active = response.hovered() || response.dragged();
            if active {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            let stroke = if active {
                ui.visuals().selection.stroke
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke
            };
            ui.painter().vline(x, header.y_range().shrink(8.0), stroke);
            let dx = response.drag_delta().x;
            if dx != 0.0 {
                // Width moves between the two neighbours; the others keep theirs.
                let per_point = total / spare;
                let floor = MIN * per_point;
                let (left, right) = (columns[k - 1].0, columns[k].0);
                let w = &mut self.preferences.batch_columns.weights;
                let shift =
                    (dx * per_point).clamp((floor - w[left]).min(0.0), (w[right] - floor).max(0.0));
                w[left] += shift;
                w[right] -= shift;
            }
            if response.double_clicked() {
                self.preferences.batch_columns.reset_widths();
            }
            save |= response.drag_stopped() || response.double_clicked();
        }
        if save && let Err(error) = self.preferences.save(&self.settings_path) {
            self.error = Some(t.format("settings.save_error", &[("error", error)]));
        }

        let mut line = 0;
        for (row, mut data) in rows.into_iter().enumerate() {
            if let Some((_, heading, ids)) = headings.iter().find(|(at, ..)| *at == row) {
                let rect = next_row(ui);
                ui.painter().rect_filled(rect, 0.0, stripe);
                line = 0;
                self.batch_heading(ui, t, rect, CHECK, heading, ids, ready);
            }
            let rect = next_row(ui);
            if line % 2 == 1 {
                ui.painter().rect_filled(rect, 0.0, stripe);
            }
            line += 1;
            let mut checked = self.batch_selected.contains(&data.id);
            let mut response = table_cell(ui, span(rect, 0.0, CHECK), ("check", row)).add_enabled(
                data.reason.is_none(),
                egui::Checkbox::without_text(&mut checked),
            );
            if let Some(reason) = &data.reason {
                response = response.on_disabled_hover_text(reason);
            }
            if response.changed() {
                if checked {
                    self.batch_selected.insert(data.id.clone());
                } else {
                    self.batch_selected.remove(&data.id);
                }
                self.log(Action::BatchSelect, Outcome::Succeeded);
            }
            for (k, (i, column)) in columns.iter().enumerate() {
                let mut cell = table_cell(ui, span(rect, edges[k], edges[k + 1]), (row, k));
                let text = std::mem::take(&mut data.cells[*i]);
                match column {
                    BatchColumn::Host => {
                        let button = egui::Button::selectable(self.selected == data.id, text);
                        let popup = self.preferences.batch_detail == BatchDetail::Popup;
                        let mut response = cell.add(button.truncate());
                        if popup {
                            response = response.on_hover_text(t.get("batch_detail.open_hint"));
                        }
                        if response.clicked() {
                            self.selected = data.id.clone();
                            self.log(Action::HostSelect, Outcome::Succeeded);
                        }
                        if popup && response.double_clicked() {
                            self.batch_popup = true;
                        }
                    }
                    BatchColumn::Status => {
                        let color = if data.failed {
                            palette.error
                        } else {
                            palette.ink
                        };
                        cell.add(egui::Label::new(RichText::new(text).color(color)).truncate());
                    }
                    BatchColumn::Phased => {
                        let response = cell.add(egui::Label::new(text).truncate());
                        if let Some(names) = &data.phased {
                            response.on_hover_text(
                                t.format("batch.phased_hint", &[("packages", names.clone())]),
                            );
                        }
                    }
                    _ => {
                        cell.add(egui::Label::new(text).truncate());
                    }
                }
            }
        }
    }

    /// A group's row in the batch table; its checkbox picks every host there that can run.
    #[allow(clippy::too_many_arguments)]
    fn batch_heading(
        &mut self,
        ui: &mut egui::Ui,
        t: Translator,
        rect: egui::Rect,
        check: f32,
        name: &str,
        ids: &[String],
        ready: &BTreeSet<String>,
    ) {
        let split = rect.left() + check;
        let mut boxes = table_cell(
            ui,
            egui::Rect::from_x_y_ranges(rect.left()..=split, rect.y_range()),
            ("heading_check", name),
        );
        let mut label = table_cell(
            ui,
            egui::Rect::from_x_y_ranges(split..=rect.right(), rect.y_range()),
            ("heading", name),
        );
        let ui = &mut boxes;
        let eligible: Vec<&String> = ids.iter().filter(|id| ready.contains(*id)).collect();
        let chosen = eligible
            .iter()
            .filter(|id| self.batch_selected.contains(**id))
            .count();
        let mut all = !eligible.is_empty() && chosen == eligible.len();
        let partly = chosen > 0 && !all;
        let response = ui
            .add_enabled(
                !eligible.is_empty(),
                egui::Checkbox::without_text(&mut all).indeterminate(partly),
            )
            .on_hover_text(t.get("batch_order.group_select"));
        if response.changed() {
            for id in eligible {
                if all {
                    self.batch_selected.insert(id.clone());
                } else {
                    self.batch_selected.remove(id);
                }
            }
            self.log(Action::BatchSelect, Outcome::Succeeded);
        }
        label.label(
            RichText::new(format!("{name} · {}", ids.len()))
                .strong()
                .color(Color32::from_rgb(147, 174, 208)),
        );
    }

    fn prompt_upgrade(&mut self, ids: Vec<String>, batch: bool) {
        self.upgrade_confirm = Some(UpgradeConfirm {
            ids,
            batch,
            acknowledged: false,
        });
        self.log(Action::UpgradePrompt, Outcome::Succeeded);
    }

    fn upgrade_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut confirm) = self.upgrade_confirm.take() else {
            return;
        };
        let palette = Palette::for_context(ctx);
        let t = Translator(self.preferences.language);
        let targets: Vec<&Host> = confirm
            .ids
            .iter()
            .filter_map(|id| self.hosts.iter().find(|h| h.id == *id))
            .collect();
        // A batch shares one manager; root targets simply skip sudo.
        let commands = match targets.first() {
            Some(host) if !confirm.batch => package_manager::package_commands(host, true),
            Some(host) => package_manager::Manager::from_id(host.manager())
                .map(|m| package_manager::manager_commands(m, false, true))
                .unwrap_or_default(),
            None => vec![],
        };
        let manager = targets
            .first()
            .map(|h| manager_label(t, h.manager()))
            .unwrap_or_default();
        let names: Vec<String> = targets.iter().map(|h| h.name.clone()).collect();
        let mut proceed = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("upgrade_confirm")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.heading(t.format(
                "upgrade.title",
                &[
                    ("count", names.len().to_string()),
                    ("manager", manager.into()),
                ],
            ));
            ui.label(t.get("upgrade.hosts"));
            egui::ScrollArea::vertical()
                .id_salt("upgrade_hosts")
                .max_height(110.0)
                .show(ui, |ui| {
                    for name in &names {
                        ui.label(format!("•  {name}"));
                    }
                });
            ui.label(t.get("hint.upgrade_effect"));
            ui.colored_label(palette.warning, t.get("hint.upgrade_risk"));
            if confirm.batch {
                ui.label(t.get("upgrade.batch_notice"));
            }
            ui.label(RichText::new(t.get("hint.commands")).strong());
            for command in &commands {
                ui.add(
                    egui::Label::new(RichText::new(*command).monospace().size(11.0))
                        .wrap()
                        .selectable(true),
                );
            }
            ui.checkbox(&mut confirm.acknowledged, t.get("upgrade.ack"));
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal(|ui| {
                    proceed = ui
                        .add_enabled(
                            confirm.acknowledged && !names.is_empty() && commands.len() > 1,
                            danger(t.get("upgrade.confirm")),
                        )
                        .clicked();
                    cancel = ui.button(t.get("common.cancel")).clicked();
                });
            });
        });
        if proceed {
            if confirm.batch {
                self.send(json!({"action":"batch","operation":"upgrade","ids":confirm.ids}));
            } else {
                self.selected = confirm.ids[0].clone();
                self.start_connection_action("upgrade");
            }
        } else if cancel {
            self.log(Action::UpgradeCancel, Outcome::Cancelled);
        } else {
            self.upgrade_confirm = Some(confirm);
        }
    }

    /// Two confirmations before a reboot: the risks with an acknowledgement, then a final yes.
    fn reboot_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut confirm) = self.reboot_confirm.take() else {
            return;
        };
        let palette = Palette::for_context(ctx);
        let t = Translator(self.preferences.language);
        let (mut proceed, mut back, mut cancel) = (false, false, false);
        let name = [("name", confirm.name.clone())];
        egui::Modal::new(egui::Id::unique("reboot_confirm")).show(ctx, |ui| {
            ui.set_width(560.0);
            if confirm.final_step {
                ui.heading(t.get("reboot.final_title"));
                ui.add_space(6.0);
                ui.label(t.format(
                    "reboot.final_question",
                    &[
                        ("name", confirm.name.clone()),
                        ("address", confirm.address.clone()),
                    ],
                ));
                ui.colored_label(palette.warning, t.get("reboot.cancel_hint"));
            } else {
                ui.heading(t.format("reboot.title", &name));
                ui.add_space(6.0);
                ui.label(t.get("hint.reboot_effect"));
                ui.colored_label(palette.warning, t.get("hint.reboot_risk"));
                ui.label(RichText::new(t.get("hint.commands")).strong());
                ui.add(
                    egui::Label::new(RichText::new(confirm.command).monospace().size(11.0))
                        .wrap()
                        .selectable(true),
                );
                ui.checkbox(&mut confirm.acknowledged, t.get("reboot.ack"));
            }
            ui.add_space(6.0);
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal(|ui| {
                    if confirm.final_step {
                        proceed = ui.add(danger(t.get("reboot.confirm"))).clicked();
                        back = ui.button(t.get("reboot.back")).clicked();
                    } else {
                        proceed = ui
                            .add_enabled(confirm.acknowledged, danger(t.get("reboot.next")))
                            .clicked();
                    }
                    cancel = ui.button(t.get("common.cancel")).clicked();
                });
            });
        });
        if cancel {
            self.log(Action::RebootCancel, Outcome::Cancelled);
        } else if back {
            confirm.final_step = false;
            self.reboot_confirm = Some(confirm);
        } else if proceed && !confirm.final_step {
            confirm.final_step = true;
            self.reboot_confirm = Some(confirm);
        } else if proceed {
            self.selected = confirm.id.clone();
            self.send(json!({"action":"reboot","id":confirm.id}));
        } else {
            self.reboot_confirm = Some(confirm);
        }
    }

    /// Asks whether a batch keeps going after a host failed. It cannot be dismissed:
    /// the worker starts no new host until one of the two answers is sent.
    fn batch_failure_dialog(&mut self, ctx: &egui::Context) {
        let Some(failure) = self.batch_failure.as_ref() else {
            return;
        };
        let palette = Palette::for_context(ctx);
        let t = Translator(self.preferences.language);
        let mut decision = None;
        egui::Modal::new(egui::Id::unique("batch_failure")).show(ctx, |ui| {
            ui.set_width(560.0);
            let operation = if failure.upgrade {
                t.get("batch.operation_upgrade")
            } else {
                t.get("batch.operation_check")
            };
            ui.heading(t.format(
                "batch_failure.title",
                &[
                    ("name", failure.name.clone()),
                    ("operation", operation.into()),
                ],
            ));
            ui.add_space(6.0);
            if !failure.error.is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt("batch_failure_error")
                    .max_height(120.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&failure.error).color(palette.error))
                                .wrap()
                                .selectable(true),
                        );
                    });
            }
            ui.label(t.get("batch_failure.paused"));
            ui.colored_label(palette.warning, t.get("batch_failure.abort_notice"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(danger(t.get("batch_failure.abort"))).clicked() {
                    decision = Some(batch::Decision::Abort);
                }
                if ui.add(primary(t.get("batch_failure.continue"))).clicked() {
                    decision = Some(batch::Decision::Continue);
                }
            });
        });
        if let Some(decision) = decision {
            self.session_log
                .add("UI", &format!("batch_decision {decision:?}"), "");
            self.batch_failure = None;
            if self.worker.decisions.send(decision).is_err() {
                self.error = Some(t.get("status.worker_ended").into());
            }
        }
    }

    /// Storage mode and path, right-aligned in the top bar.
    fn storage_location(&self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
            ui.label(
                RichText::new(if self.encrypted {
                    t.get("storage.encrypted")
                } else {
                    t.get("storage.not_created")
                })
                .size(11.0)
                .color(palette.warning),
            );
            ui.label(RichText::new(&self.path).size(10.0).color(palette.muted));
        });
    }

    /// The development warning and storage problems, under the top bar's buttons.
    fn storage_notes(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        if !self.storage_warning.is_empty() {
            ui.colored_label(palette.error, &self.storage_warning);
        }
        if self.encrypted && self.plaintext_exists {
            ui.colored_label(palette.warning, t.get("storage.original_remains"));
            if ui
                .add_enabled(
                    !self.locked && !self.busy,
                    egui::Button::new(t.get("plaintext.review")),
                )
                .clicked()
            {
                self.check_plaintext_copy();
            }
        }
    }

    /// Vault and SSH key buttons, on the same row as Settings.
    fn storage_buttons(&mut self, ui: &mut egui::Ui) {
        let t = Translator(self.preferences.language);
        ui.add_enabled_ui(!self.busy, |ui| {
            if self.encrypted {
                if self.locked {
                    if ui.button(t.get("vault.unlock_host")).clicked() {
                        self.open_vault_dialog(VaultMode::Unlock);
                    }
                } else {
                    if ui.button(t.get("vault.lock")).clicked() {
                        self.send(json!({"action":"vault_lock"}));
                    }
                }
            } else if ui.button(t.get("vault.enable_button")).clicked() {
                self.open_vault_dialog(VaultMode::Create);
            }
            if !self.locked && ui.button(t.get("keys.manage")).clicked() {
                self.open_key_dialog();
            }
        });
    }

    fn key_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.key_dialog.take() else {
            return;
        };
        if self.locked {
            return;
        }
        let palette = Palette::for_context(ctx);
        let t = Translator(self.preferences.language);
        let (mut add, mut close, mut confirm_delete) = (false, false, false);
        egui::Modal::new(egui::Id::unique("ssh_keys")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.heading(t.get("keys.title"));
            ui.label(t.get("keys.description"));
            ui.add_space(8.0);
            ui.add_enabled_ui(!self.busy, |ui| {
                if let Some(id) = dialog.delete.clone() {
                    let name = self
                        .ssh_keys
                        .iter()
                        .find(|key| key.id == id)
                        .map(|key| key.name.clone())
                        .unwrap_or_default();
                    ui.label(t.format("keys.delete_prompt", &[("name", name)]));
                    ui.horizontal(|ui| {
                        confirm_delete = ui.add(danger(t.get("keys.confirm_delete"))).clicked();
                        if ui.button(t.get("common.cancel")).clicked() {
                            dialog.delete = None;
                        }
                    });
                    return;
                }
                if self.ssh_keys.is_empty() {
                    ui.label(RichText::new(t.get("keys.none")).color(palette.muted));
                }
                for key in &self.ssh_keys {
                    let users = self.hosts.iter().filter(|h| h.key_id == key.id).count();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&key.name).strong());
                        ui.label(RichText::new(&key.algorithm).color(palette.muted));
                        ui.label(t.format("keys.used_by", &[("count", users.to_string())]));
                        let delete = ui
                            .add_enabled(
                                users == 0,
                                egui::Button::new(
                                    RichText::new(t.get("common.delete")).color(palette.error),
                                ),
                            )
                            .on_disabled_hover_text(t.get("keys.in_use"));
                        if delete.clicked() {
                            dialog.delete = Some(key.id.clone());
                        }
                    });
                    ui.label(
                        RichText::new(&key.fingerprint)
                            .monospace()
                            .size(11.0)
                            .color(palette.muted),
                    );
                }
                ui.separator();
                ui.label(RichText::new(t.get("keys.add_title")).strong());
                field(
                    ui,
                    t.get("keys.name"),
                    &mut dialog.name,
                    t.get("keys.name_hint"),
                    false,
                );
                ui.label(t.get("auth.key_format"));
                ui.add(
                    egui::TextEdit::multiline(&mut dialog.private_key)
                        .password(!dialog.show_secrets)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text(t.get("auth.key_hint")),
                );
                if ui.button(t.get("auth.import_key")).clicked() {
                    match self.pick_key_file() {
                        Some(Ok(key)) => {
                            dialog.private_key = key;
                            dialog.error.clear();
                        }
                        Some(Err(error)) => dialog.error = error,
                        None => {}
                    }
                }
                field(
                    ui,
                    t.get("auth.key_passphrase"),
                    &mut dialog.passphrase,
                    t.get("auth.key_passphrase_hint"),
                    !dialog.show_secrets,
                );
                if ui
                    .checkbox(&mut dialog.show_secrets, t.get("auth.show_secrets"))
                    .changed()
                {
                    self.log(Action::RevealSecrets, Outcome::Succeeded);
                }
                if !dialog.error.is_empty() {
                    ui.colored_label(palette.error, &dialog.error);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    add = ui.add(primary(t.get("keys.add"))).clicked();
                    close = ui.button(t.get("common.close")).clicked();
                });
            });
        });
        if add {
            if dialog.name.trim().is_empty() || dialog.private_key.trim().is_empty() {
                dialog.error = t.get("keys.incomplete").into();
                self.log(Action::ValidationRejected, Outcome::Failed);
            } else {
                dialog.error.clear();
                self.send(json!({
                    "action": "key_add",
                    "name": dialog.name,
                    "private_key": dialog.private_key,
                    "passphrase": dialog.passphrase,
                }));
            }
        }
        if confirm_delete && let Some(id) = &dialog.delete {
            self.send(json!({"action":"key_delete","id":id}));
        }
        if !close {
            self.key_dialog = Some(dialog);
        }
    }

    fn apply_storage(&mut self, data: &Value) {
        self.encrypted = data["encrypted"].as_bool().unwrap_or(false);
        self.locked = data["locked"].as_bool().unwrap_or(false);
        self.plaintext_exists = data["plaintext_exists"].as_bool().unwrap_or(false);
        self.plaintext_path = data["plaintext_path"].as_str().unwrap_or_default().into();
        self.path = data["path"].as_str().unwrap_or_default().into();
        self.storage_warning = data["warning"].as_str().unwrap_or_default().into();
        self.slots = data["slots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|slot| {
                slot["index"].as_u64().map(|index| SlotRow {
                    index: index as usize,
                    kind: slot["kind"].as_str().unwrap_or_default().to_owned(),
                    name: slot["name"].as_str().unwrap_or_default().to_owned(),
                })
            })
            .collect();
    }

    fn check_plaintext_copy(&mut self) {
        if self.encrypted
            && !self.locked
            && self.plaintext_exists
            && !self.plaintext_path.is_empty()
        {
            self.plaintext_delete_prompt = true;
            self.log(Action::PlaintextDeletePrompt, Outcome::Succeeded);
        }
    }

    fn plaintext_delete_dialog(&mut self, ctx: &egui::Context) {
        if !self.plaintext_delete_prompt || self.locked {
            return;
        }
        let t = Translator(self.preferences.language);
        let mut delete = false;
        let mut keep = false;
        egui::Modal::new(egui::Id::unique("delete_plaintext_copy")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(t.get("plaintext.title"));
            ui.label(t.get("plaintext.description"));
            ui.add_space(8.0);
            ui.label(RichText::new(t.get("plaintext.path_label")).strong());
            ui.add(
                egui::Label::new(RichText::new(&self.plaintext_path).monospace())
                    .wrap()
                    .selectable(true),
            );
            ui.add_space(8.0);
            ui.label(t.get("plaintext.delete_notice"));
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal(|ui| {
                    keep = ui.button(t.get("plaintext.keep")).clicked();
                    delete = ui
                        .button(
                            RichText::new(t.get("plaintext.confirm_delete"))
                                .color(Palette::for_context(ctx).error),
                        )
                        .clicked();
                });
            });
        });
        if keep {
            self.respond_plaintext_delete(false);
        }
        if delete {
            self.respond_plaintext_delete(true);
        }
    }

    /// Asks before writing a decrypted older database back at this build's version.
    fn schema_upgrade_dialog(&mut self, ctx: &egui::Context) {
        let Some(from) = self.schema_upgrade_prompt else {
            return;
        };
        let t = Translator(self.preferences.language);
        let versions = [
            ("from", from.to_string()),
            ("to", misf_core::sqlite::SCHEMA_VERSION.to_string()),
        ];
        let mut upgrade = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("schema_upgrade")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(t.get("schema_upgrade.title"));
            ui.label(t.format("schema_upgrade.description", &versions));
            ui.add_space(8.0);
            ui.label(t.get("schema_upgrade.notice"));
            ui.add_space(8.0);
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal(|ui| {
                    upgrade = ui
                        .add(primary(&t.format("schema_upgrade.confirm", &versions)))
                        .clicked();
                    cancel = ui.button(t.get("schema_upgrade.cancel")).clicked();
                });
            });
        });
        if upgrade {
            self.send(json!({"action":"vault_upgrade"}));
        }
        if cancel {
            self.send(json!({"action":"vault_upgrade_cancel"}));
        }
    }

    fn respond_plaintext_delete(&mut self, delete: bool) {
        if delete {
            self.send(json!({"action":"vault_delete_plaintext", "path":self.plaintext_path, "confirmed":true}));
        } else {
            self.plaintext_delete_prompt = false;
            self.log(Action::PlaintextDeleteCancel, Outcome::Cancelled);
        }
    }

    fn clear_host_cache(&mut self) {
        for host in &mut self.hosts {
            wipe_host(host);
        }
        self.hosts.clear();
        self.groups.clear();
        self.group_dialog = None;
        self.storage_format.clear();
        self.schema_version = None;
        self.selected.clear();
        self.search.zeroize();
        self.package_search.zeroize();
        if let Some(editor) = &mut self.editor {
            wipe_host(&mut editor.host);
        }
        self.editor = None;
        self.trust = None;
        self.delete_id = None;
        self.after_trust = None;
        self.batch_selected.clear();
        self.upgrade_confirm = None;
        self.ssh_keys.clear();
        self.key_dialog = None;
    }

    /// Starts the unlock method chosen in preferences, so a passkey prompt appears without
    /// first choosing it in the dialog. A PIN is still typed where the platform asks for one.
    fn start_default_unlock(&mut self) {
        if std::mem::replace(&mut self.default_unlock_started, true) || !self.passkey_allowed {
            return;
        }
        let hello = match self.preferences.unlock_method {
            UnlockMethod::Password => return,
            UnlockMethod::WindowsHello => true,
            UnlockMethod::SecurityKey => false,
        };
        let Some(dialog) = self.vault_dialog.as_mut() else {
            return;
        };
        dialog.mode = VaultMode::Passkey;
        dialog.hello = hello;
        if cfg!(target_os = "windows") {
            self.send(json!({"action":"vault_unlock_passkey","hello":hello,"name":"","pin":""}));
        }
    }

    fn open_vault_dialog(&mut self, mode: VaultMode) {
        self.log(Action::VaultDialogOpen, Outcome::Succeeded);
        self.vault_dialog = Some(VaultDialog {
            mode,
            name: String::new(),
            password: String::new(),
            confirmation: String::new(),
            error: String::new(),
            hello: cfg!(target_os = "windows"),
            acknowledged: false,
        });
    }

    /// "#1（Google Titan）", or "#1" for an unnamed slot.
    fn slot_title(&self, index: usize) -> String {
        let t = Translator(self.preferences.language);
        match self.slots.iter().find(|slot| slot.index == index) {
            Some(slot) if !slot.name.is_empty() => t.format(
                "vault.slot_named",
                &[("slot", index.to_string()), ("name", slot.name.clone())],
            ),
            _ => format!("#{index}"),
        }
    }

    fn host_name(&self, id: &str) -> String {
        self.hosts
            .iter()
            .find(|h| h.id == id)
            .map_or_else(|| id.to_owned(), |h| h.name.clone())
    }

    /// Copies a finished request into the session log: check and upgrade results as host
    /// blocks with their remote output, everything else in full with secrets masked.
    fn log_done(&mut self, action: &str, result: &Result<Value, String>) {
        let t = Translator(self.preferences.language);
        let data = match result {
            Ok(data) => data,
            // The message itself is logged once as an ERROR entry.
            Err(_) => {
                return self
                    .session_log
                    .add("RESULT", &format!("{action} failed"), "");
            }
        };
        match action {
            "check" | "upgrade" => match serde_json::from_value::<CheckResult>(data.clone()) {
                Ok(check) => {
                    let name = self.host_name(&self.selected);
                    let host = self.hosts.iter().find(|h| h.id == self.selected);
                    self.session_log.host(t, &name, host, &check);
                }
                Err(_) => self.session_log.response(action, data),
            },
            "batch" => {
                let results = data["results"].as_array().map_or(&[][..], Vec::as_slice);
                let succeeded = results
                    .iter()
                    .filter(|entry| entry["result"]["status"] == "success")
                    .count();
                self.session_log.add(
                    "RESULT",
                    &format!(
                        "batch finished: succeeded={succeeded} failed={} skipped={}",
                        results.len() - succeeded,
                        data["skipped"]
                    ),
                    "",
                );
            }
            "reboot" => self.session_log.add(
                "RESULT",
                &format!("reboot id={:?}", self.selected),
                data["output"].as_str().unwrap_or_default(),
            ),
            _ => self.session_log.response(action, data),
        }
    }

    /// Opens a titled section in the session log for an operation on remote hosts.
    fn log_operation(&mut self, request: &Value) {
        let t = Translator(self.preferences.language);
        let action = request["action"].as_str().unwrap_or_default();
        let name = self.host_name(request["id"].as_str().unwrap_or_default());
        let operation = |upgrade: bool| {
            if upgrade {
                t.get("batch.operation_upgrade")
            } else {
                t.get("batch.operation_check")
            }
            .to_owned()
        };
        let title = match action {
            "batch" => t.format(
                "log.title_batch",
                &[
                    ("manager", manager_label(t, &self.batch_manager).to_owned()),
                    ("operation", operation(request["operation"] == "upgrade")),
                    (
                        "count",
                        request["ids"].as_array().map_or(0, Vec::len).to_string(),
                    ),
                ],
            ),
            "check" | "upgrade" => {
                let manager = self
                    .hosts
                    .iter()
                    .find(|h| request["id"] == h.id.as_str())
                    .map_or("", |h| h.manager());
                t.format(
                    "log.title_single",
                    &[
                        ("manager", manager_label(t, manager).to_owned()),
                        ("operation", operation(action == "upgrade")),
                        ("name", name),
                    ],
                )
            }
            "test" => t.format("log.title_test", &[("name", name)]),
            "probe" => t.format("log.title_probe", &[("name", name)]),
            "trust" => t.format("log.title_trust", &[("name", name)]),
            "reboot" => t.format("log.title_reboot", &[("name", name)]),
            _ => return,
        };
        self.session_log.banner(&title);
    }

    /// The LOG settings page: what the in-memory log holds and how to export it.
    fn log_settings(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        ui.label(t.get("log.description"));
        ui.add_space(6.0);
        ui.label(t.format(
            "log.size",
            &[
                ("count", self.session_log.len().to_string()),
                (
                    "size",
                    format!("{:.1}", self.session_log.bytes() as f64 / 1024.0),
                ),
            ],
        ));
        ui.add_space(6.0);
        ui.colored_label(palette.warning, t.get("log.warning"));
        ui.add_space(10.0);
        if ui.add(primary(t.get("log.export"))).clicked() {
            self.export_log();
        }
    }

    fn export_log(&mut self) {
        let t = Translator(self.preferences.language);
        self.log(Action::LogExport, Outcome::Started);
        let file_name = format!("misf-log-{}.txt", Local::now().format("%Y%m%d-%H%M%S"));
        let Some(path) = rfd::FileDialog::new()
            .set_title(t.get("log.export_title"))
            .set_file_name(&file_name)
            .add_filter("Text", &["txt", "log"])
            .save_file()
        else {
            return self.log(Action::LogExport, Outcome::Cancelled);
        };
        match self.session_log.export(&path) {
            Ok(()) => {
                self.log(Action::LogExport, Outcome::Succeeded);
                self.status = t.format("log.exported", &[("path", path.display().to_string())]);
            }
            Err(error) => {
                self.log(Action::LogExport, Outcome::Failed);
                self.error = Some(t.format("log.export_failed", &[("error", error.to_string())]));
            }
        }
    }

    fn choose_backup(&mut self) {
        self.log(Action::BackupDialog, Outcome::Started);
        let t = Translator(self.preferences.language);
        if let Some(path) = rfd::FileDialog::new()
            .set_title(t.get("vault.export_title"))
            .set_file_name("misf-backup.lbx")
            .add_filter("LUKSbox", &["lbx"])
            .save_file()
        {
            self.send(json!({"action":"vault_backup","path":path}));
        } else {
            self.log(Action::BackupDialog, Outcome::Cancelled);
        }
    }

    fn vault_dialog(&mut self, ctx: &egui::Context) {
        let palette = Palette::for_context(ctx);
        let t = Translator(self.preferences.language);
        let Some(mut dialog) = self.vault_dialog.take() else {
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        let mut add = false;
        let mut backup = false;
        let mut passkey = false;
        let mut use_password = false;
        let mut revoke = None;
        let mut rename = None;
        egui::Modal::new(egui::Id::unique("vault_dialog")).show(ctx, |ui| {
            ui.set_width(570.0);
            ui.heading(match dialog.mode {
                VaultMode::Create => t.get("vault.enable"),
                VaultMode::Unlock => t.get("vault.unlock_host"),
                VaultMode::Manage => t.get("vault.manage"),
                VaultMode::AddKey => t.get("vault.add_password"),
                VaultMode::Passkey => t.get("passkey.title"),
                VaultMode::Revoke(_) => t.get("vault.revoke"),
                VaultMode::Rename(_) => t.get("vault.rename_title"),
            });
            ui.add_space(12.0);
            ui.add_enabled_ui(!self.busy, |ui| {
                if dialog.mode == VaultMode::Manage {
                    ui.label(t.format(
                        "vault.slot_count",
                        &[("count", self.slots.len().to_string())],
                    ));
                    for slot in &self.slots {
                        ui.horizontal(|ui| {
                            ui.label(t.format("vault.slot", &[("slot", slot.index.to_string())]));
                            ui.label(if slot.kind == "Fido2HmacSecret" {
                                t.get("passkey.title")
                            } else {
                                t.get("auth.password")
                            });
                            if slot.name.is_empty() {
                                ui.label(
                                    RichText::new(t.get("vault.slot_unnamed")).color(palette.muted),
                                );
                            } else {
                                ui.label(RichText::new(&slot.name).strong());
                            }
                            if ui.button(t.get("vault.rename")).clicked() {
                                rename = Some((slot.index, slot.name.clone()));
                            }
                            if ui
                                .add_enabled(
                                    self.slots.len() > 1,
                                    egui::Button::new(t.get("vault.revoke")),
                                )
                                .clicked()
                            {
                                revoke = Some(slot.index);
                            }
                        });
                    }
                    ui.label(t.get("vault.revoke_limit"));
                    ui.label(t.get("vault.keep_backup"));
                    ui.horizontal(|ui| {
                        add = ui
                            .add_enabled(self.slots.len() < 8, primary(t.get("vault.add_password")))
                            .clicked();
                        passkey = ui
                            .add_enabled(
                                self.passkey_allowed && self.slots.len() < 8,
                                egui::Button::new(t.get("passkey.add")),
                            )
                            .clicked();
                        backup = ui.button(t.get("vault.export")).clicked();
                        cancel = ui.button(t.get("common.close")).clicked();
                    });
                } else if let VaultMode::Rename(slot) = dialog.mode {
                    ui.label(t.format("vault.slot", &[("slot", slot.to_string())]));
                    field(
                        ui,
                        t.get("vault.slot_name"),
                        &mut dialog.name,
                        t.get("vault.slot_name_hint"),
                        false,
                    );
                    ui.horizontal(|ui| {
                        submit = ui.add(primary(t.get("vault.rename_save"))).clicked();
                        cancel = ui.button(t.get("common.cancel")).clicked();
                    });
                } else if let VaultMode::Revoke(slot) = dialog.mode {
                    ui.label(t.format("vault.revoke_prompt", &[("slot", self.slot_title(slot))]));
                    ui.colored_label(palette.warning, t.get("vault.revoke_backup_warning"));
                    ui.checkbox(&mut dialog.acknowledged, t.get("vault.revoke_ack"));
                    ui.horizontal(|ui| {
                        submit = ui
                            .add_enabled(
                                dialog.acknowledged,
                                egui::Button::new(t.get("vault.revoke_confirm")),
                            )
                            .clicked();
                        cancel = ui.button(t.get("common.cancel")).clicked();
                    });
                } else if dialog.mode == VaultMode::Passkey {
                    ui.label(t.get("passkey.description"));
                    #[cfg(target_os = "windows")]
                    ui.horizontal(|ui| {
                        let before = dialog.hello;
                        ui.radio_value(&mut dialog.hello, true, "Windows Hello");
                        ui.radio_value(&mut dialog.hello, false, t.get("passkey.security_key"));
                        if before != dialog.hello {
                            self.log(Action::PasskeyMethod, Outcome::Succeeded);
                        }
                    });
                    if !self.locked {
                        field(
                            ui,
                            t.get("vault.slot_name"),
                            &mut dialog.name,
                            t.get("passkey.name_hint"),
                            false,
                        );
                    }
                    #[cfg(not(target_os = "windows"))]
                    field(
                        ui,
                        t.get("passkey.pin"),
                        &mut dialog.password,
                        t.get("passkey.pin_hint"),
                        true,
                    );
                    ui.horizontal(|ui| {
                        submit = ui
                            .add_enabled(
                                self.passkey_allowed,
                                primary(if self.locked {
                                    t.get("passkey.unlock")
                                } else {
                                    t.get("passkey.add")
                                }),
                            )
                            .clicked();
                        if self.locked {
                            use_password = ui.button(t.get("passkey.use_password")).clicked();
                        }
                        cancel = ui.button(t.get("common.cancel")).clicked();
                    });
                } else {
                    if dialog.mode == VaultMode::Create {
                        ui.label(t.get("vault.create_description"));
                        ui.colored_label(palette.warning, t.get("vault.experimental"));
                        // Only an older profile has unencrypted hosts to convert.
                        if self.plaintext_exists {
                            ui.label(t.get("vault.retain_plaintext"));
                        }
                    }
                    if dialog.mode != VaultMode::Unlock {
                        field(
                            ui,
                            t.get("vault.slot_name"),
                            &mut dialog.name,
                            t.get("vault.slot_name_hint"),
                            false,
                        );
                    }
                    field(
                        ui,
                        t.get("vault.password"),
                        &mut dialog.password,
                        if dialog.mode == VaultMode::Unlock {
                            t.get("vault.password_hint")
                        } else {
                            t.get("vault.new_password_hint")
                        },
                        true,
                    );
                    if dialog.mode != VaultMode::Unlock {
                        field(
                            ui,
                            t.get("vault.confirm_password"),
                            &mut dialog.confirmation,
                            t.get("vault.confirm_hint"),
                            true,
                        );
                    }
                    if !dialog.error.is_empty() {
                        ui.colored_label(palette.error, &dialog.error);
                    }
                    ui.horizontal(|ui| {
                        submit = ui
                            .add(primary(match dialog.mode {
                                VaultMode::Create => t.get("vault.encrypt_verify"),
                                VaultMode::Unlock => t.get("vault.unlock"),
                                _ => t.get("vault.add_key"),
                            }))
                            .clicked();
                        cancel = ui.button(t.get("common.cancel")).clicked();
                    });
                    if dialog.mode == VaultMode::Unlock {
                        passkey = ui
                            .add_enabled(
                                self.passkey_allowed,
                                egui::Button::new(t.get("passkey.unlock")),
                            )
                            .clicked();
                    }
                }
                if self.debug {
                    ui.colored_label(
                        palette.warning,
                        if self.passkey_allowed {
                            t.get("debug.hardware_notice")
                        } else {
                            t.get("debug.notice")
                        },
                    );
                }
            });
            if self.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t.get("vault.working"));
                });
            }
        });
        if add {
            dialog.mode = VaultMode::AddKey;
            self.log(Action::VaultDialogOpen, Outcome::Succeeded);
        }
        if passkey {
            dialog.password.zeroize();
            dialog.confirmation.zeroize();
            dialog.mode = VaultMode::Passkey;
            self.log(Action::VaultDialogOpen, Outcome::Succeeded);
        }
        if use_password {
            dialog.password.zeroize();
            dialog.error.clear();
            dialog.mode = VaultMode::Unlock;
            self.log(Action::VaultDialogOpen, Outcome::Succeeded);
        }
        if let Some((slot, name)) = rename {
            dialog.mode = VaultMode::Rename(slot);
            dialog.name = name;
            dialog.error.clear();
            self.log_at_slot(Action::VaultDialogOpen, Outcome::Succeeded, Some(slot));
        }
        if let Some(slot) = revoke {
            dialog.mode = VaultMode::Revoke(slot);
            dialog.acknowledged = false;
            self.log_at_slot(Action::VaultRevokePrompt, Outcome::Succeeded, Some(slot));
        }
        if backup {
            self.choose_backup();
        }
        if submit {
            if let VaultMode::Revoke(slot) = dialog.mode {
                self.send(json!({"action":"vault_revoke_key","slot":slot}));
            } else if let VaultMode::Rename(slot) = dialog.mode {
                self.send(json!({"action":"vault_rename_key","slot":slot,"name":dialog.name}));
            } else if dialog.mode == VaultMode::Passkey {
                let action = if self.locked {
                    "vault_unlock_passkey"
                } else {
                    "vault_add_passkey"
                };
                self.send(json!({"action":action,"hello":dialog.hello,"name":dialog.name,"pin":std::mem::take(&mut dialog.password)}));
            } else if dialog.mode != VaultMode::Unlock && dialog.password != dialog.confirmation {
                dialog.error = t.get("vault.password_mismatch").into();
            } else if let Some(problem) = (dialog.mode != VaultMode::Unlock)
                .then(|| passphrase_problem(&dialog.password))
                .flatten()
            {
                dialog.error = match problem {
                    PassphraseProblem::TooShort => t.format(
                        "vault.password_short",
                        &[("min", MIN_PASSPHRASE_CHARS.to_string())],
                    ),
                    PassphraseProblem::TooFewKinds => t.get("vault.password_kinds").into(),
                };
            } else if dialog.password.is_empty() {
                dialog.error = t.get("vault.password_required").into();
            } else {
                dialog.error.clear();
                let action = match dialog.mode {
                    VaultMode::Create => "vault_create",
                    VaultMode::Unlock => "vault_unlock",
                    _ => "vault_add_key",
                };
                let mut request = json!({"action":action});
                if dialog.mode != VaultMode::Unlock {
                    request["name"] = Value::String(dialog.name.clone());
                }
                request["passphrase"] = Value::String(std::mem::take(&mut dialog.password));
                dialog.confirmation.zeroize();
                self.send(request);
            }
            if !dialog.error.is_empty() {
                self.log(Action::ValidationRejected, Outcome::Failed);
            }
        }
        if cancel {
            if let VaultMode::Revoke(slot) = dialog.mode {
                self.log_at_slot(Action::VaultRevokeCancel, Outcome::Cancelled, Some(slot));
            } else {
                self.log(Action::VaultDialogClose, Outcome::Succeeded);
            }
        }
        if !cancel {
            self.vault_dialog = Some(dialog);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let palette = Palette::for_context(ctx);
        let t = Translator(self.preferences.language);
        if let Some(mut editor) = self.editor.take() {
            let mut cancel = false;
            let mut save = false;
            let mut editor_changed = false;
            egui::Modal::new(egui::Id::unique("host_editor")).show(ctx, |ui| {
                ui.set_width(560.0);
                ui.heading(if editor.host.id.is_empty() {
                    t.get("host.add")
                } else {
                    t.get("host.edit")
                });
                ui.add_space(10.0);
                ui.add_enabled_ui(!self.busy, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 180.0).max(300.0))
                        .show(ui, |ui| {
                            editor_changed |= field(
                                ui,
                                t.get("host.name"),
                                &mut editor.host.name,
                                t.get("host.name_hint"),
                                false,
                            );
                            ui.columns(2, |columns| {
                                editor_changed |= field(
                                    &mut columns[0],
                                    t.get("host.address"),
                                    &mut editor.host.address,
                                    t.get("host.address_hint"),
                                    false,
                                );
                                editor_changed |= field(
                                    &mut columns[1],
                                    t.get("host.port"),
                                    &mut editor.port,
                                    "22",
                                    false,
                                );
                            });
                            editor_changed |= field(
                                ui,
                                t.get("auth.username"),
                                &mut editor.host.username,
                                t.get("auth.username_hint"),
                                false,
                            );
                            let choices_before = (
                                editor.host.os.clone(),
                                editor.host.auth_type.clone(),
                                editor.saved_key,
                            );
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_label(t.get("host.os"))
                                    .selected_text(&editor.host.os)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut editor.host.os,
                                            "Linux".into(),
                                            "Linux",
                                        );
                                        ui.selectable_value(
                                            &mut editor.host.os,
                                            "BSD".into(),
                                            "BSD",
                                        );
                                    });
                                let saved = editor.host.auth_type == "key" && editor.saved_key;
                                egui::ComboBox::from_label(t.get("auth.method"))
                                    .selected_text(match (editor.host.auth_type.as_str(), saved) {
                                        ("key", true) => t.get("auth.saved_key"),
                                        ("key", false) => t.get("auth.private_key"),
                                        _ => t.get("auth.password"),
                                    })
                                    .show_ui(ui, |ui| {
                                        for (auth, stored, label) in [
                                            ("password", false, t.get("auth.password")),
                                            ("key", false, t.get("auth.private_key")),
                                            ("key", true, t.get("auth.saved_key")),
                                        ] {
                                            let current =
                                                editor.host.auth_type == auth && saved == stored;
                                            if ui.selectable_label(current, label).clicked() {
                                                editor.host.auth_type = auth.into();
                                                editor.saved_key = stored;
                                            }
                                        }
                                    });
                            });
                            editor_changed |= choices_before
                                != (
                                    editor.host.os.clone(),
                                    editor.host.auth_type.clone(),
                                    editor.saved_key,
                                );
                            let managers = misf_core::model::manager_choices(&editor.host.os);
                            // Switching between Linux and BSD drops a choice the new OS lacks.
                            if !managers.contains(&editor.host.manager_choice.as_str()) {
                                editor.host.manager_choice.clear();
                                editor_changed = true;
                            }
                            let auto = if editor.host.package_manager.is_empty() {
                                t.get("host.manager_auto").to_owned()
                            } else {
                                t.format(
                                    "host.manager_auto_detected",
                                    &[(
                                        "manager",
                                        manager_label(t, &editor.host.package_manager).into(),
                                    )],
                                )
                            };
                            let choice_label = |id: &str| {
                                if id.is_empty() {
                                    auto.clone()
                                } else {
                                    manager_label(t, id).to_owned()
                                }
                            };
                            let choice_before = editor.host.manager_choice.clone();
                            egui::ComboBox::from_label(t.get("host.manager"))
                                .selected_text(choice_label(&editor.host.manager_choice))
                                .show_ui(ui, |ui| {
                                    for id in managers {
                                        ui.selectable_value(
                                            &mut editor.host.manager_choice,
                                            (*id).into(),
                                            choice_label(id),
                                        );
                                    }
                                });
                            editor_changed |= choice_before != editor.host.manager_choice;
                            if ui
                                .checkbox(&mut editor.show_secrets, t.get("auth.show_secrets"))
                                .changed()
                            {
                                self.log(Action::RevealSecrets, Outcome::Succeeded);
                            }
                            if editor.host.auth_type == "password" {
                                editor_changed |= field(
                                    ui,
                                    t.get("auth.ssh_password"),
                                    &mut editor.host.password,
                                    t.get("auth.ssh_password_hint"),
                                    !editor.show_secrets,
                                );
                            } else if editor.saved_key {
                                let chosen =
                                    self.ssh_keys.iter().find(|k| k.id == editor.host.key_id);
                                if self.ssh_keys.is_empty() {
                                    ui.label(
                                        RichText::new(t.get("keys.none")).color(palette.muted),
                                    );
                                } else {
                                    let before = editor.host.key_id.clone();
                                    egui::ComboBox::from_label(t.get("keys.choose_label"))
                                        .selected_text(
                                            chosen
                                                .map(KeyRow::label)
                                                .unwrap_or(t.get("keys.choose").into()),
                                        )
                                        .width(300.0)
                                        .show_ui(ui, |ui| {
                                            for key in &self.ssh_keys {
                                                ui.selectable_value(
                                                    &mut editor.host.key_id,
                                                    key.id.clone(),
                                                    key.label(),
                                                );
                                            }
                                        });
                                    editor_changed |= before != editor.host.key_id;
                                }
                                if let Some(key) = chosen {
                                    ui.label(
                                        RichText::new(&key.fingerprint)
                                            .monospace()
                                            .size(11.0)
                                            .color(palette.muted),
                                    );
                                }
                                if ui.button(t.get("keys.manage")).clicked() {
                                    self.open_key_dialog();
                                }
                            } else {
                                ui.label(t.get("auth.key_format"));
                                editor_changed |= ui
                                    .add(
                                        egui::TextEdit::multiline(&mut editor.host.private_key)
                                            .password(!editor.show_secrets)
                                            .desired_rows(3)
                                            .desired_width(f32::INFINITY)
                                            .hint_text(t.get("auth.key_hint")),
                                    )
                                    .changed();
                                if ui.button(t.get("auth.import_key")).clicked() {
                                    match self.pick_key_file() {
                                        Some(Ok(key)) => {
                                            editor.host.private_key = key;
                                            editor.error.clear();
                                        }
                                        Some(Err(error)) => editor.error = error,
                                        None => {}
                                    }
                                }
                                editor_changed |= field(
                                    ui,
                                    t.get("auth.key_passphrase"),
                                    &mut editor.host.key_passphrase,
                                    t.get("auth.key_passphrase_hint"),
                                    !editor.show_secrets,
                                );
                            }
                            ui.separator();
                            editor_changed |= field(
                                ui,
                                t.get("auth.sudo_password"),
                                &mut editor.host.sudo_password,
                                t.get("auth.sudo_hint"),
                                !editor.show_secrets,
                            );
                            editor_changed |= ui
                                .checkbox(&mut editor.host.use_pty, t.get("auth.pty"))
                                .changed();
                            editor_changed |= field(
                                ui,
                                t.get("host.notes"),
                                &mut editor.host.notes,
                                t.get("common.optional"),
                                false,
                            );
                            ui.label(
                                RichText::new(t.get("storage.credentials_encrypted"))
                                    .size(12.0)
                                    .color(palette.muted),
                            );
                            if !editor.error.is_empty() {
                                ui.colored_label(palette.error, &editor.error);
                            }
                        });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        save = ui.add(primary(t.get("host.save"))).clicked();
                        cancel = ui.button(t.get("common.cancel")).clicked();
                    });
                });
                if self.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t.get("host.saving"));
                    });
                }
            });
            if editor_changed {
                self.log(Action::FormEdit, Outcome::Succeeded);
            }
            if save {
                let saved = editor.host.auth_type == "key" && editor.saved_key;
                if !saved {
                    editor.host.key_id.clear();
                }
                match editor.port.trim().parse::<u16>() {
                    _ if saved && editor.host.key_id.is_empty() => {
                        editor.error = t.get("keys.choose_error").into();
                        self.log(Action::ValidationRejected, Outcome::Failed);
                    }
                    Ok(port) if port > 0 => {
                        editor.host.port = port;
                        self.send(json!({"action":"save","host":editor.host}));
                    }
                    _ => {
                        editor.error = t.get("host.port_invalid").into();
                        self.log(Action::ValidationRejected, Outcome::Failed);
                    }
                }
            }
            if !cancel {
                self.editor = Some(editor);
            } else {
                self.log(Action::EditorClose, Outcome::Succeeded);
            }
        }
        if let Some(trust) = &self.trust {
            let mut accept = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("trust")).show(ctx, |ui| {
                ui.set_width(580.0);
                ui.heading(if trust.old.is_empty() {
                    t.get("trust.confirm")
                } else {
                    t.get("trust.changed")
                });
                ui.label(&trust.address);
                ui.add_space(10.0);
                ui.label(t.get("trust.received"));
                ui.monospace(&trust.actual);
                if !trust.old.is_empty() {
                    ui.label(t.get("trust.previous"));
                    ui.monospace(&trust.old);
                }
                ui.label(t.get("trust.description"));
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal(|ui| {
                        accept = ui.add(primary(t.get("trust.accept"))).clicked();
                        cancel = ui.button(t.get("common.cancel")).clicked();
                    });
                });
            });
            if accept {
                self.send(json!({"action":"trust","id":trust.host_id,"fingerprint":trust.actual}));
            }
            if cancel {
                self.log(Action::TrustCancel, Outcome::Succeeded);
                self.trust = None;
                self.after_trust = None;
                self.status = t.get("trust.cancelled").into();
            }
        }
        if let Some(id) = self.delete_id.clone() {
            let name = self
                .hosts
                .iter()
                .find(|h| h.id == id)
                .map(|h| h.name.as_str())
                .unwrap_or(t.get("host.generic"));
            let mut yes = false;
            let mut no = false;
            egui::Modal::new(egui::Id::unique("delete")).show(ctx, |ui| {
                ui.set_width(400.0);
                ui.heading(t.get("host.delete"));
                ui.label(t.format("host.delete_prompt", &[("name", name.into())]));
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal(|ui| {
                        yes = ui.button(t.get("host.confirm_delete")).clicked();
                        no = ui.button(t.get("common.cancel")).clicked();
                    });
                });
            });
            if yes {
                self.send(json!({"action":"delete","id":id}));
            }
            if no {
                self.log(Action::DeleteCancel, Outcome::Succeeded);
                self.delete_id = None;
            }
        }
        self.upgrade_dialog(ctx);
        self.batch_failure_dialog(ctx);
        self.reboot_dialog(ctx);
        self.vault_dialog(ctx);
        self.schema_upgrade_dialog(ctx);
        self.plaintext_delete_dialog(ctx);
        // After the editor, so it opens on top when chosen from there.
        self.key_dialog(ctx);
        self.group_dialog(ctx);
        if self.error != self.logged_error {
            if let Some(error) = &self.error {
                self.session_log.add("ERROR", error, "");
            }
            self.logged_error = self.error.clone();
        }
        if let Some(error) = self.error.clone() {
            let mut close = false;
            egui::Modal::new(egui::Id::unique("error")).show(ctx, |ui| {
                ui.set_width(540.0);
                ui.heading(t.get("error.title"));
                ui.label(error);
                close = ui.button(t.get("common.close")).clicked();
            });
            if close {
                self.error = None;
                self.log(Action::ErrorDismiss, Outcome::Succeeded);
            }
        }
    }
}

impl Drop for ManagementApp {
    fn drop(&mut self) {
        // Settings that failed to load are left as they are rather than overwritten.
        let windows = (self.preferences.window, self.preferences.settings_window);
        if self.window_loaded && windows != self.saved_window {
            let _ = self.preferences.save(&self.settings_path);
        }
        let _ = self
            .audit
            .record(Action::AppExit, Outcome::Succeeded, None, None);
        self.clear_host_cache();
    }
}

impl eframe::App for ManagementApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_events();
        let ctx = ui.ctx().clone();
        self.track_window(&ctx);
        egui::Panel::top("preferences")
            .frame(
                egui::Frame::new()
                    .fill(Palette::for_context(&ctx).surface)
                    .inner_margin(12),
            )
            .show(ui, |ui| {
                let t = Translator(self.preferences.language);
                ui.horizontal(|ui| {
                    if ui.button(t.get("settings.title")).clicked() {
                        self.settings_open = !self.settings_open;
                        self.log(
                            if self.settings_open {
                                Action::SettingsOpen
                            } else {
                                Action::SettingsClose
                            },
                            Outcome::Succeeded,
                        );
                    }
                    ui.separator();
                    self.storage_buttons(ui);
                    self.storage_location(ui);
                });
                self.storage_notes(ui);
                if self.debug {
                    ui.colored_label(
                        Palette::for_context(&ctx).warning,
                        if self.passkey_allowed {
                            t.get("debug.hardware_notice")
                        } else {
                            t.get("debug.notice")
                        },
                    );
                }
            });
        let palette = Palette::for_context(&ctx);
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(palette.surface).inner_margin(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.busy {
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.status).size(12.0).color(palette.muted));
                });
            });
        egui::Panel::left("hosts")
            .exact_size(280.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(NAVY).inner_margin(20))
            .show(ui, |ui| {
                ui.add_enabled_ui(!self.busy, |ui| self.sidebar(ui));
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.background).inner_margin(28))
            .show(ui, |ui| {
                let viewport_height = ui.available_height();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!self.busy, |ui| self.workspace(ui, viewport_height));
                    });
            });
        self.settings_window(&ctx);
        self.dialogs(&ctx);
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        #[cfg(feature = "capture")]
        if let Some(path) = std::env::var_os("MISF_SCREENSHOT_TO") {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            if !self.capture_requested
                && !self.busy
                && self.capture_started.elapsed().as_secs_f32() > 1.0
            {
                self.capture_requested = true;
                let context = ctx.clone();
                ctx.request_screenshot(move |image| {
                    image::save_buffer(
                        std::path::PathBuf::from(path),
                        image.as_raw(),
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    )
                    .expect("Failed to save UI screenshot");
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
        }
    }
}

fn card(ui: &egui::Ui) -> egui::Frame {
    let palette = Palette::for_context(ui.ctx());
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(8)
        .inner_margin(18)
}

fn wipe_host(host: &mut Host) {
    host.password.zeroize();
    host.private_key.zeroize();
    host.key_passphrase.zeroize();
    host.sudo_password.zeroize();
    host.name.zeroize();
    host.address.zeroize();
    host.username.zeroize();
    host.notes.zeroize();
    if let Some(result) = &mut host.last_check {
        result.output.zeroize();
        result.error.zeroize();
    }
}
fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(BLUE)
}
fn danger(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(RED)
}
/// Batch operations need a confirmed fingerprint and a detected, supported package manager.
fn batch_ready(host: &Host) -> bool {
    !host.fingerprint.is_empty() && package_manager::Manager::from_id(host.manager()).is_some()
}
/// Batch-ready package managers with their host counts, most hosts first.
fn batch_managers(hosts: &[Host]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for host in hosts.iter().filter(|h| batch_ready(h)) {
        match counts.iter_mut().find(|(id, _)| id == host.manager()) {
            Some((_, count)) => *count += 1,
            None => counts.push((host.manager().into(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}
/// Label and value rows for the last "test connection"; empty strings mean not reached.
fn connection_rows(t: Translator, test: &ConnectionTest) -> Vec<(&'static str, String)> {
    let or_dash = |value: &str| {
        if value.is_empty() {
            "—".to_owned()
        } else {
            value.to_owned()
        }
    };
    let with_fingerprint = |algorithm: &str, fingerprint: &str| match (algorithm, fingerprint) {
        ("", "") => "—".to_owned(),
        (algorithm, "") => algorithm.to_owned(),
        ("", fingerprint) => fingerprint.to_owned(),
        (algorithm, fingerprint) => format!("{algorithm}\n{fingerprint}"),
    };
    let negotiated = !test.kex.is_empty();
    let kex = if test.kex.contains("mlkem") || test.kex.contains("sntrup") {
        format!("{}  ·  {}", test.kex, t.get("conn.post_quantum"))
    } else {
        or_dash(&test.kex)
    };
    // AEAD ciphers authenticate by themselves, so SSH negotiates no separate MAC.
    let mac = match (test.client_mac.as_str(), test.server_mac.as_str()) {
        ("", "") => "—".to_owned(),
        ("none", "none") => t.get("conn.mac_aead").to_owned(),
        (client, server) if client == server => client.to_owned(),
        (client, server) => t.format(
            "conn.mac_split",
            &[("client", client.into()), ("server", server.into())],
        ),
    };
    let strict = match (negotiated, test.strict_kex) {
        (false, _) => "—".to_owned(),
        (true, true) => t.get("conn.strict_yes").to_owned(),
        (true, false) => t.get("conn.strict_no").to_owned(),
    };
    let auth = match test.auth_method.as_str() {
        "password" => t.get("conn.auth_password").to_owned(),
        "publickey" => format!(
            "{}\n{}",
            t.format(
                "conn.auth_key",
                &[("algorithm", test.client_key_algorithm.clone())]
            ),
            test.client_key_fingerprint
        ),
        _ => "—".to_owned(),
    };
    let system = match (test.distro.as_str(), test.kernel.as_str()) {
        ("", kernel) => or_dash(kernel),
        (distro, "") => distro.to_owned(),
        (distro, kernel) => format!("{distro}  ·  {kernel}"),
    };
    let managers = if test.package_managers.is_empty() {
        "—".to_owned()
    } else {
        test.package_managers
            .iter()
            .map(|id| manager_label(t, id))
            .collect::<Vec<_>>()
            .join("、")
    };
    vec![
        (t.get("conn.server"), or_dash(&test.server_version)),
        (
            t.get("conn.host_key"),
            with_fingerprint(&test.host_key_algorithm, &test.host_key_fingerprint),
        ),
        (t.get("conn.kex"), kex),
        (t.get("conn.cipher"), or_dash(&test.cipher)),
        (t.get("conn.mac"), mac),
        (t.get("conn.strict"), strict),
        (t.get("conn.auth"), auth),
        (t.get("conn.system"), system),
        (t.get("conn.managers"), managers),
    ]
}

/// The connection details tab: how the last test connected and what it found.
fn connection_panel(ui: &mut egui::Ui, t: Translator, test: Option<&ConnectionTest>) {
    let palette = Palette::for_context(ui.ctx());
    let Some(test) = test else {
        ui.label(RichText::new(t.get("conn.not_run")).color(palette.muted));
        return;
    };
    let succeeded = test.status == "success";
    let time = [("time", format_time(&test.tested_at))];
    ui.label(
        RichText::new(if succeeded {
            t.format("conn.succeeded", &time)
        } else {
            t.format("conn.failed", &time)
        })
        .color(if succeeded {
            palette.muted
        } else {
            palette.error
        }),
    );
    if !succeeded && !test.error.is_empty() {
        ui.add(egui::Label::new(RichText::new(&test.error).color(palette.error)).selectable(true));
    }
    ui.add_space(8.0);
    egui::Grid::new("connection_rows")
        .num_columns(2)
        .striped(true)
        .min_col_width(130.0)
        .spacing([24.0, 12.0])
        .show(ui, |ui| {
            for (label, value) in connection_rows(t, test) {
                ui.label(RichText::new(label).strong().color(palette.muted));
                ui.add(egui::Label::new(RichText::new(value).monospace()).selectable(true));
                ui.end_row();
            }
        });
}

fn manager_label(t: Translator, id: &str) -> &'static str {
    package_manager::Manager::from_id(id)
        .map(package_manager::Manager::label)
        .unwrap_or(t.get("manager.none"))
}
/// "Ubuntu 26.04 · APT" once detected, otherwise the OS family chosen in the editor.
fn system_label(t: Translator, host: &Host) -> String {
    if host.distro.is_empty() {
        host.os.clone()
    } else {
        format!("{} · {}", host.distro, manager_label(t, host.manager()))
    }
}
/// Pending updates from the last successful run; hosts without one sort with zero.
fn pending_count(host: &Host) -> usize {
    host.last_check
        .as_ref()
        .filter(|r| r.status == "success")
        .map_or(0, |r| r.packages.len())
}
/// A short line of what needs restarting, or `None` when nothing does.
fn restart_summary(t: Translator, restart: &RestartStatus) -> Option<String> {
    let mut parts = vec![];
    if restart.reboot_required {
        parts.push(t.get("restart.reboot_short").to_owned());
    }
    let count = restart.services.len() + restart.containers.len();
    if count > 0 {
        parts.push(t.format("restart.services_short", &[("count", count.to_string())]));
    }
    (!parts.is_empty()).then(|| parts.join(t.get("restart.separator")))
}

/// The restart tab: why there is nothing to show, or what needs a reboot or restart.
fn restart_panel(ui: &mut egui::Ui, t: Translator, host: &Host) {
    let palette = Palette::for_context(ui.ctx());
    let muted = |ui: &mut egui::Ui, text: &str| {
        ui.label(RichText::new(text).color(palette.muted));
    };
    if host.manager() != "apt" {
        return muted(ui, t.get("restart.apt_only"));
    }
    if host.username != "root" {
        return muted(ui, t.get("restart.root_only"));
    }
    let Some((checked_at, restart)) = host
        .last_check
        .as_ref()
        .and_then(|r| r.restart.as_ref().map(|s| (&r.checked_at, s)))
    else {
        return muted(ui, t.get("restart.not_read"));
    };
    ui.label(
        RichText::new(t.format("restart.read_at", &[("time", format_time(checked_at))]))
            .size(11.0)
            .color(palette.muted),
    );
    ui.add_space(6.0);
    if restart.reboot_required {
        ui.colored_label(palette.warning, t.get("restart.reboot_required"));
    } else {
        ui.label(t.get("restart.no_reboot"));
    }
    egui::Grid::new("restart_kernel")
        .num_columns(2)
        .spacing([20.0, 8.0])
        .show(ui, |ui| {
            if !restart.running_kernel.is_empty() {
                ui.label(RichText::new(t.get("restart.running_kernel")).color(palette.muted));
                ui.label(&restart.running_kernel);
                ui.end_row();
            }
            if !restart.expected_kernel.is_empty() {
                ui.label(RichText::new(t.get("restart.expected_kernel")).color(palette.muted));
                ui.label(&restart.expected_kernel);
                ui.end_row();
            }
        });
    let list = |ui: &mut egui::Ui, title: String, names: &[String]| {
        if names.is_empty() {
            return;
        }
        ui.add_space(8.0);
        ui.label(RichText::new(title).strong());
        for name in names {
            ui.add(egui::Label::new(format!("•  {name}")).selectable(true));
        }
    };
    let count = |names: &[String]| [("count", names.len().to_string())];
    list(
        ui,
        t.format("restart.packages", &count(&restart.reboot_packages)),
        &restart.reboot_packages,
    );
    if !restart.needrestart {
        ui.add_space(8.0);
        return muted(ui, t.get("restart.no_needrestart"));
    }
    list(
        ui,
        t.format("restart.services", &count(&restart.services)),
        &restart.services,
    );
    list(
        ui,
        t.format("restart.containers", &count(&restart.containers)),
        &restart.containers,
    );
    list(
        ui,
        t.format("restart.sessions", &count(&restart.sessions)),
        &restart.sessions,
    );
    ui.add_space(8.0);
    if restart.services.is_empty() && restart.containers.is_empty() && restart.sessions.is_empty() {
        ui.label(t.get("restart.no_services"));
    } else {
        muted(ui, t.get("restart.manual"));
    }
}

fn record_result(host: &mut Host, result: CheckResult) {
    if !result.distro.is_empty() {
        host.distro = result.distro.clone();
        host.package_manager = result.package_manager.clone();
    }
    host.last_check = Some(result);
}
fn status_label(t: Translator, host: &Host) -> &'static str {
    match host.last_check.as_ref() {
        Some(r) => match (r.status.as_str(), r.operation == "upgrade") {
            ("success", false) => t.get("apt.complete"),
            ("success", true) => t.get("apt.upgrade_complete"),
            ("unsupported", _) => t.get("apt.unsupported"),
            (_, false) => t.get("apt.failed"),
            (_, true) => t.get("apt.upgrade_failed"),
        },
        None => t.get("apt.not_checked"),
    }
}
/// After the style's tooltip delay, explains what a sensitive control runs and risks.
fn hint(
    response: egui::Response,
    t: Translator,
    title: &str,
    commands: &[&str],
    effect: &str,
    risk: &str,
) -> egui::Response {
    let warning = Palette::for_context(&response.ctx).warning;
    response.on_hover_ui(|ui| {
        ui.set_max_width(560.0);
        ui.label(RichText::new(title).strong());
        if !commands.is_empty() {
            ui.label(t.get("hint.commands"));
            for command in commands {
                ui.add(egui::Label::new(RichText::new(*command).monospace().size(11.0)).wrap());
            }
        }
        ui.label(effect);
        ui.colored_label(warning, risk);
    })
}
fn stat(ui: &mut egui::Ui, title: &str, value: &str) {
    let palette = Palette::for_context(ui.ctx());
    card(ui).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new(title).size(12.0).color(palette.muted));
        ui.add_space(6.0);
        ui.label(RichText::new(value).size(22.0).strong());
    });
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, secret: bool) -> bool {
    ui.label(RichText::new(label).size(12.0).strong());
    ui.add(
        egui::TextEdit::singleline(value)
            .password(secret)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    )
    .changed()
}
/// One host's row in the batch table, as display text only.
struct BatchRow {
    id: String,
    /// Why the host cannot be selected, when it cannot.
    reason: Option<String>,
    /// Indexed like [`BatchColumn::ALL`].
    cells: [String; 11],
    failed: bool,
    /// The phased package names, for the phased count's tooltip.
    phased: Option<String>,
}

/// A left-to-right ui inside one table cell, clipped to it.
fn table_cell(ui: &mut egui::Ui, rect: egui::Rect, salt: impl egui::AsIdSalt) -> egui::Ui {
    let mut cell = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect.shrink2(egui::vec2(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    cell.set_clip_rect(rect.intersect(ui.clip_rect()));
    cell
}

fn format_time(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Local).format("%m/%d %H:%M").to_string())
        .unwrap_or_else(|_| value.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn connection_rows_explain_aead_post_quantum_and_unreached_steps() {
        let t = Translator(Language::ZhTw);
        let value = |rows: &[(&str, String)], key: &str| {
            rows.iter().find(|(k, _)| *k == key).unwrap().1.clone()
        };
        let test = ConnectionTest {
            status: "success".into(),
            kex: "mlkem768x25519-sha256".into(),
            cipher: "chacha20-poly1305@openssh.com".into(),
            client_mac: "none".into(),
            server_mac: "none".into(),
            strict_kex: true,
            auth_method: "publickey".into(),
            client_key_algorithm: "ecdsa-sha2-nistp521".into(),
            client_key_fingerprint: "SHA256:abc".into(),
            package_managers: vec!["dnf".into(), "yum".into()],
            ..Default::default()
        };
        let rows = connection_rows(t, &test);
        assert!(value(&rows, t.get("conn.kex")).contains(t.get("conn.post_quantum")));
        assert_eq!(value(&rows, t.get("conn.mac")), t.get("conn.mac_aead"));
        assert_eq!(value(&rows, t.get("conn.strict")), t.get("conn.strict_yes"));
        assert!(value(&rows, t.get("conn.auth")).contains("ecdsa-sha2-nistp521"));
        assert_eq!(value(&rows, t.get("conn.managers")), "DNF、YUM");
        // A test that failed before key exchange shows dashes, not "not strict".
        let failed = ConnectionTest {
            status: "failed".into(),
            ..Default::default()
        };
        let rows = connection_rows(t, &failed);
        assert!(rows.iter().all(|(_, value)| value == "—"), "{rows:?}");
    }

    #[test]
    fn plaintext_prompt_reappears_after_unlock_and_renders_full_path_in_both_languages() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: directory.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
        };
        let ctx = egui::Context::default();
        let audit = AuditLog::new(directory.path()).unwrap();
        let (requests, commands) = mpsc::channel();
        let (_events, results) = mpsc::channel();
        // No background runtime, hardware authenticator, or user profile in this UI test.
        let mut app = ManagementApp::initialize(
            &ctx,
            paths,
            audit,
            Worker {
                requests,
                events: results,
                decisions: tokio::sync::mpsc::unbounded_channel().0,
            },
        );
        assert_eq!(commands.recv().unwrap()["action"], "storage_status");
        let full_path = directory.path().join("hosts.db").display().to_string();
        let mut status = json!({"encrypted":false, "locked":true, "plaintext_exists":true, "plaintext_path":full_path});
        // No vault yet: setup opens at once, and no host is ever read unencrypted.
        app.received("storage_status", status.clone());
        assert!(!app.plaintext_delete_prompt);
        assert!(
            app.vault_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.mode == VaultMode::Create)
        );
        assert!(commands.try_recv().is_err());
        app.vault_dialog = None;
        status["encrypted"] = json!(true);
        status["locked"] = json!(true);
        app.received("storage_status", status.clone());
        assert!(!app.plaintext_delete_prompt);
        assert!(
            app.vault_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.mode == VaultMode::Unlock)
        );
        status["locked"] = json!(false);
        for action in ["vault_create", "vault_unlock", "vault_unlock_passkey"] {
            app.received(action, status.clone());
            assert!(app.plaintext_delete_prompt);
            assert!(app.vault_dialog.is_none());
            assert_eq!(commands.recv().unwrap()["action"], "list");
            app.busy = false;
            for language in [Language::ZhTw, Language::EnUs] {
                app.preferences.language = language;
                let mut text = String::new();
                // egui may perform a sizing pass before painting a newly opened modal.
                for _ in 0..2 {
                    let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
                        app.plaintext_delete_dialog(ui.ctx())
                    });
                    fn collect(shape: &egui::epaint::Shape, text: &mut String) {
                        match shape {
                            egui::epaint::Shape::Text(shape) => {
                                text.push_str(&shape.galley.job.text)
                            }
                            egui::epaint::Shape::Vec(shapes) => {
                                for shape in shapes {
                                    collect(shape, text);
                                }
                            }
                            _ => {}
                        }
                    }
                    frame.textures_delta.clear();
                    for shape in frame.shapes {
                        collect(&shape.shape, &mut text);
                    }
                }
                assert!(
                    text.contains(&full_path),
                    "Full path missing from {} modal",
                    language.code()
                );
                assert!(text.contains(Translator(language).get("plaintext.confirm_delete")));
            }
            app.respond_plaintext_delete(false);
            assert!(!app.plaintext_delete_prompt);
            assert!(commands.try_recv().is_err());
        }
        app.check_plaintext_copy();
        app.respond_plaintext_delete(true);
        let request = commands.recv().unwrap();
        assert_eq!(
            request,
            json!({"action":"vault_delete_plaintext", "path":full_path, "confirmed":true})
        );
        status["plaintext_exists"] = json!(false);
        app.received("vault_delete_plaintext", status.clone());
        assert!(!app.plaintext_delete_prompt);
        app.received("vault_unlock", status.clone());
        assert!(!app.plaintext_delete_prompt);
        assert_eq!(commands.recv().unwrap()["action"], "list");
        // An older vault database: no hosts are requested until the upgrade is accepted.
        let mut pending = status.clone();
        pending["locked"] = json!(true);
        pending["pending_upgrade"] = json!(1);
        app.received("vault_unlock", pending.clone());
        assert_eq!(app.schema_upgrade_prompt, Some(1));
        assert!(commands.try_recv().is_err());
        app.received("vault_upgrade_cancel", status.clone());
        assert_eq!(app.schema_upgrade_prompt, None);
        assert!(
            app.vault_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.mode == VaultMode::Unlock)
        );
        app.received("vault_unlock_passkey", pending);
        app.received("vault_upgrade", status);
        assert_eq!(app.schema_upgrade_prompt, None);
        assert_eq!(commands.recv().unwrap()["action"], "list");
    }

    #[test]
    fn preferred_passkey_starts_once_when_the_vault_is_locked_at_launch() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: directory.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: true,
        };
        let ctx = egui::Context::default();
        let audit = AuditLog::new(directory.path()).unwrap();
        let (requests, commands) = mpsc::channel();
        let (_events, results) = mpsc::channel();
        let mut app = ManagementApp::initialize(
            &ctx,
            paths,
            audit,
            Worker {
                requests,
                events: results,
                decisions: tokio::sync::mpsc::unbounded_channel().0,
            },
        );
        assert_eq!(commands.recv().unwrap()["action"], "storage_status");
        app.passkey_allowed = true;
        app.preferences.unlock_method = UnlockMethod::SecurityKey;
        let locked = json!({"encrypted":true, "locked":true});
        app.received("storage_status", locked.clone());
        let dialog = app.vault_dialog.as_ref().unwrap();
        assert_eq!(dialog.mode, VaultMode::Passkey);
        assert!(!dialog.hello);
        if cfg!(target_os = "windows") {
            let request = commands.recv().unwrap();
            assert_eq!(request["action"], "vault_unlock_passkey");
            assert_eq!(request["hello"], false);
        }
        // Locking again later shows the plain dialog instead of prompting by surprise.
        app.busy = false;
        app.received("storage_status", locked);
        assert_eq!(app.vault_dialog.as_ref().unwrap().mode, VaultMode::Unlock);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn session_log_records_the_run_without_any_secret() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: directory.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
        };
        let ctx = egui::Context::default();
        let audit = AuditLog::new(directory.path()).unwrap();
        let (requests, _commands) = mpsc::channel();
        let (events, results) = mpsc::channel();
        let mut app = ManagementApp::initialize(
            &ctx,
            paths,
            audit,
            Worker {
                requests,
                events: results,
                decisions: tokio::sync::mpsc::unbounded_channel().0,
            },
        );
        app.send(json!({"action":"vault_unlock","passphrase":"unlock-secret"}));
        let list = json!({"hosts":[{"id":"h","name":"web","password":"ssh-secret",
            "sudo_password":"sudo-secret","private_key":"key-body"}],"keys":[],"groups":[]});
        events
            .send(Event::Progress("正在解鎖 LUKSbox…".into()))
            .unwrap();
        events
            .send(Event::Done {
                action: "list".into(),
                result: Ok(list),
            })
            .unwrap();
        events
            .send(Event::Done {
                action: "check".into(),
                result: Err("無法連線".into()),
            })
            .unwrap();
        app.process_events();
        let mut frame = ctx.run_ui(Default::default(), |ui| app.dialogs(&ui.ctx().clone()));
        frame.textures_delta.clear();

        // A batch opens a titled section, and each host gets its own framed block.
        app.hosts = vec![Host {
            id: "u1".into(),
            name: "UBUNTU1".into(),
            address: "10.0.0.5".into(),
            port: 22,
            username: "deploy".into(),
            password: "ssh-secret".into(),
            package_manager: "apt".into(),
            ..Default::default()
        }];
        app.batch_manager = "apt".into();
        app.send(json!({"action":"batch","operation":"upgrade","ids":["u1"]}));
        events
            .send(Event::BatchHost {
                id: "u1".into(),
                result: CheckResult {
                    operation: "upgrade".into(),
                    status: "success".into(),
                    output: "Setting up openssl (3.0.17) ...\n".into(),
                    ..Default::default()
                },
            })
            .unwrap();
        app.process_events();

        let text = app.session_log.render();
        for secret in ["unlock-secret", "ssh-secret", "sudo-secret", "key-body"] {
            assert!(!text.contains(secret), "{secret}");
        }
        for expected in [
            r#"[REQUEST] vault_unlock {"action":"vault_unlock","passphrase":"[已遮蔽]"}"#,
            "[PROGRESS] 正在解鎖 LUKSbox…",
            r#""name":"web""#,
            "[RESULT] check failed",
            "[ERROR] 無法連線",
            "\n********\n使用者 進行批次 APT 升級（1 台）  (",
            r#"[REQUEST] batch {"action":"batch","ids":["u1"],"operation":"upgrade"}"#,
            "\n********\nUBUNTU1\n********\n主機 ID: u1\n連線: deploy@10.0.0.5:22\n",
            "遠端原始輸出:\nSetting up openssl (3.0.17) ...\n",
            "\n********\nEND OF UBUNTU1\n********\n",
        ] {
            assert!(text.contains(expected), "{expected}\n{}", text.as_str());
        }
    }

    fn rendered_text(ctx: &egui::Context, mut draw: impl FnMut(&mut egui::Ui)) -> String {
        fn collect(shape: &egui::epaint::Shape, text: &mut String) {
            match shape {
                egui::epaint::Shape::Text(shape) => text.push_str(&shape.galley.job.text),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        // A newly opened modal may need a sizing pass before it paints.
        for _ in 0..2 {
            let mut frame = ctx.run_ui(egui::RawInput::default(), &mut draw);
            frame.textures_delta.clear();
            for shape in frame.shapes {
                collect(&shape.shape, &mut text);
            }
        }
        text
    }

    #[test]
    fn batch_selection_keeps_only_eligible_hosts_and_upgrade_waits_for_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: directory.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
        };
        let ctx = egui::Context::default();
        let (requests, commands) = mpsc::channel();
        let (_events, results) = mpsc::channel();
        let mut app = ManagementApp::initialize(
            &ctx,
            paths,
            AuditLog::new(directory.path()).unwrap(),
            Worker {
                requests,
                events: results,
                decisions: tokio::sync::mpsc::unbounded_channel().0,
            },
        );
        assert_eq!(commands.recv().unwrap()["action"], "storage_status");
        let host = |id: &str, manager: &str, fingerprint: &str| Host {
            id: id.into(),
            name: format!("host-{id}"),
            address: "127.0.0.1".into(),
            username: "operator".into(),
            os: "Linux".into(),
            distro: if manager.is_empty() { "" } else { "Debian 13" }.into(),
            package_manager: manager.into(),
            fingerprint: fingerprint.into(),
            ..Default::default()
        };
        let mut busy = host("busy", "apt", "SHA256:busy-fingerprint");
        busy.last_check = Some(CheckResult {
            status: "success".into(),
            packages: vec![Default::default(); 3],
            ..Default::default()
        });
        app.batch_selected = ["trusted", "untrusted", "rocky", "undetected", "gone"]
            .map(String::from)
            .into();
        app.received(
            "list",
            json!({"hosts":[
                host("trusted", "apt", "SHA256:trusted-fingerprint"),
                host("untrusted", "apt", ""),
                host("rocky", "dnf", "SHA256:rocky-fingerprint"),
                host("undetected", "", "SHA256:undetected-fingerprint"),
                busy,
            ]}),
        );
        // APT has the most ready hosts, so only same-manager hosts stay selected.
        assert_eq!(app.batch_manager, "apt");
        assert_eq!(app.batch_selected, ["trusted".to_owned()].into());
        app.received(
            "batch",
            json!({"results":[{"id":"trusted","result":{
                "operation":"upgrade","status":"success",
                "upgraded":[{"name":"openssl","current_version":"3.0.16","candidate_version":"3.0.17","architecture":"amd64","repository":"stable"}]
            }}]}),
        );
        let saved = app.hosts[0].last_check.as_ref().unwrap();
        assert_eq!(saved.upgraded[0].name, "openssl");
        assert_eq!(
            app.status,
            Translator(app.preferences.language).format(
                "status.batch_done",
                &[("succeeded", "1".into()), ("failed", "0".into())]
            )
        );
        app.view = View::Batch;
        app.selected = "trusted".into();
        app.tab = 1;
        app.busy = false;
        for language in [Language::ZhTw, Language::EnUs] {
            app.preferences.language = language;
            let text = rendered_text(&ctx, |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.workspace(ui, 900.0));
            });
            assert!(text.contains(Translator(language).get("batch.title")));
            assert!(text.contains("openssl"));
            assert!(text.contains("Debian 13 · APT"));
            // Hosts with more pending updates come first.
            assert!(text.find("host-busy").unwrap() < text.find("host-trusted").unwrap());
            app.prompt_upgrade(vec!["trusted".into()], true);
            let text = rendered_text(&ctx, |ui| app.upgrade_dialog(ui.ctx()));
            assert!(text.contains("host-trusted"));
            assert!(text.contains(package_manager::apt::SUDO_UPGRADE));
            assert!(app.upgrade_confirm.is_some());
        }
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn dragging_a_host_or_group_in_the_sidebar_saves_the_new_arrangement() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: directory.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
        };
        let ctx = egui::Context::default();
        let (requests, commands) = mpsc::channel();
        let (_events, results) = mpsc::channel();
        let mut app = ManagementApp::initialize(
            &ctx,
            paths,
            AuditLog::new(directory.path()).unwrap(),
            Worker {
                requests,
                events: results,
                decisions: tokio::sync::mpsc::unbounded_channel().0,
            },
        );
        assert_eq!(commands.recv().unwrap()["action"], "storage_status");
        let host = |id: &str, group: &str| {
            json!({"id": id, "name": id, "address": "127.0.0.1", "port": 22,
                "username": "ops", "os": "Linux", "group_id": group})
        };
        let list = json!({
            "hosts": [host("alpha", ""), host("bravo", "g1"), host("charlie", "g1")],
            "groups": [{"id": "g1", "name": "First"}, {"id": "g2", "name": "Second"}],
        });
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(280.0, 900.0));
        let frame = |app: &mut ManagementApp, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.sidebar(ui));
            output.textures_delta.clear();
            // Where each label was drawn, to aim the pointer at it.
            let mut labels = Vec::new();
            for clipped in output.shapes {
                if let egui::epaint::Shape::Text(text) = clipped.shape {
                    labels.push((text.galley.job.text.clone(), text.visual_bounding_rect()));
                }
            }
            labels
        };
        let find = |labels: &[(String, egui::Rect)], prefix: &str| {
            labels
                .iter()
                .find(|(text, _)| text.starts_with(prefix))
                .unwrap_or_else(|| panic!("{prefix} not drawn"))
                .1
        };
        let drag = |app: &mut ManagementApp, from: egui::Pos2, to: egui::Pos2| {
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(
                app,
                vec![egui::Event::PointerMoved(from), button(from, true)],
            );
            for step in 1..=10 {
                let pos = from + (to - from) * (step as f32 / 10.0);
                frame(app, vec![egui::Event::PointerMoved(pos)]);
            }
            frame(app, vec![button(to, false)]);
            frame(app, vec![])
        };
        let placement = |request: &Value| -> Vec<(String, String)> {
            request["hosts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| {
                    (
                        h["id"].as_str().unwrap().into(),
                        h["group_id"].as_str().unwrap().into(),
                    )
                })
                .collect()
        };

        // alpha onto the lower half of charlie: last in the first group.
        app.received("list", list.clone());
        let labels = frame(&mut app, vec![]);
        let (alpha, charlie) = (find(&labels, "alpha"), find(&labels, "charlie"));
        drag(
            &mut app,
            alpha.center(),
            charlie.center() + egui::vec2(0.0, 24.0),
        );
        let request = commands.try_recv().expect("no arrange request");
        assert_eq!(request["action"], "arrange");
        assert_eq!(
            placement(&request),
            [("bravo", "g1"), ("charlie", "g1"), ("alpha", "g1")]
                .map(|(h, g)| (h.into(), g.into()))
        );
        assert_eq!(request["groups"], json!(["g1", "g2"]));
        assert!(app.hosts.iter().all(|h| h.group_id == "g1"));

        // The second group's heading above the first one's: the groups swap.
        app.busy = false;
        app.received("list", list.clone());
        let labels = frame(&mut app, vec![]);
        let (first, second) = (find(&labels, "▾ First"), find(&labels, "▾ Second"));
        drag(
            &mut app,
            second.center(),
            first.center() - egui::vec2(0.0, 4.0),
        );
        let request = commands.try_recv().expect("no arrange request");
        assert_eq!(request["groups"], json!(["g2", "g1"]));
        assert_eq!(placement(&request)[0], ("alpha".into(), String::new()));

        // Dropping a host back where it was saves nothing; a click still selects it.
        app.busy = false;
        app.received("list", list);
        let labels = frame(&mut app, vec![]);
        let bravo = find(&labels, "bravo");
        drag(
            &mut app,
            bravo.center(),
            bravo.center() + egui::vec2(0.0, 3.0),
        );
        assert!(commands.try_recv().is_err());
        assert_eq!(app.selected, "bravo");

        // The ungrouped heading above the first group: only the preference changes.
        let ungrouped = &format!(
            "▾ {}",
            Translator(app.preferences.language).get("group.ungrouped")
        );
        let labels = frame(&mut app, vec![]);
        let (heading, first) = (find(&labels, ungrouped), find(&labels, "▾ First"));
        drag(
            &mut app,
            heading.center(),
            first.center() - egui::vec2(0.0, 4.0),
        );
        assert!(commands.try_recv().is_err());
        assert_eq!(app.preferences.ungrouped_at, Some(0));
        let labels = frame(&mut app, vec![]);
        assert!(find(&labels, ungrouped).top() < find(&labels, "▾ First").top());

        // A click on the ungrouped heading collapses it like a group.
        let heading = find(&labels, ungrouped);
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(heading.center()),
                egui::Event::PointerButton {
                    pos: heading.center(),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: heading.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert!(app.preferences.ungrouped_collapsed);
        let labels = frame(&mut app, vec![]);
        assert!(labels.iter().all(|(text, _)| !text.starts_with("alpha")));
        app.preferences.ungrouped_collapsed = false;

        // Compact rows are drawn too, with grouped hosts indented past ungrouped ones.
        app.preferences.density = Density::Compact;
        let labels = frame(&mut app, vec![]);
        assert!(find(&labels, "bravo").left() > find(&labels, "alpha").left());
        assert!(find(&labels, "charlie").height() < 20.0);
    }

    #[test]
    fn batch_table_follows_the_chosen_order() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: directory.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
        };
        let ctx = egui::Context::default();
        let (requests, _commands) = mpsc::channel();
        let (_events, results) = mpsc::channel();
        let mut app = ManagementApp::initialize(
            &ctx,
            paths,
            AuditLog::new(directory.path()).unwrap(),
            Worker {
                requests,
                events: results,
                decisions: tokio::sync::mpsc::unbounded_channel().0,
            },
        );
        let host = |id: &str, group: &str, pending: usize, at: &str| {
            json!({"id": id, "name": id, "address": "127.0.0.1", "port": 22,
                "username": "ops", "os": "Linux", "group_id": group,
                "last_check": {"checked_at": at, "operation": "check", "status": "success",
                    "packages": vec![json!({"name": "pkg", "current_version": "1", "candidate_version": "2", "architecture": "amd64", "repository": "main"}); pending]}})
        };
        app.received(
            "list",
            json!({
                "hosts": [
                    host("delta", "", 1, "2026-11-01T08:00:00+08:00"),
                    host("bravo", "g1", 3, "2026-11-01T01:00:00Z"),
                    host("alpha", "g1", 0, "2026-10-30T00:00:00Z"),
                    host("charlie", "g2", 2, "2026-11-02T00:00:00Z"),
                ],
                "groups": [{"id": "g1", "name": "First"}, {"id": "g2", "name": "Second"}],
            }),
        );
        let t = Translator(Language::EnUs);
        let names = |app: &ManagementApp| -> Vec<String> {
            let (order, _) = app.batch_order(t);
            order.iter().map(|&i| app.hosts[i].name.clone()).collect()
        };
        assert_eq!(names(&app), ["bravo", "charlie", "delta", "alpha"]);
        app.preferences.batch_order = BatchOrder::Recent;
        // 08:00+08:00 is 00:00Z, before bravo's 01:00Z.
        assert_eq!(names(&app), ["charlie", "bravo", "delta", "alpha"]);
        app.preferences.batch_order = BatchOrder::Name;
        assert_eq!(names(&app), ["alpha", "bravo", "charlie", "delta"]);
        app.preferences.batch_order = BatchOrder::Sidebar;
        assert_eq!(names(&app), ["bravo", "alpha", "charlie", "delta"]);
        let (_, headings) = app.batch_order(t);
        let headings: Vec<_> = headings
            .iter()
            .map(|(at, name, _)| (*at, name.as_str()))
            .collect();
        assert_eq!(headings, [(0, "First"), (2, "Second"), (3, "Ungrouped")]);
        app.preferences.ungrouped_at = Some(0);
        assert_eq!(names(&app), ["delta", "bravo", "alpha", "charlie"]);

        // The table fills the width, shows apt's phased count, and a dragged border is saved.
        app.preferences.language = Language::EnUs;
        app.hosts[0].last_check = Some(misf_core::model::CheckResult {
            checked_at: "2026-11-02T00:00:00Z".into(),
            operation: "upgrade".into(),
            status: "success".into(),
            phased: vec!["libfoo".into(), "zed".into()],
            ..Default::default()
        });
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let frame = |app: &mut ManagementApp, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.batch_view(ui, 800.0));
            output.textures_delta.clear();
            let mut labels = Vec::new();
            for clipped in output.shapes {
                if let egui::epaint::Shape::Text(text) = clipped.shape {
                    labels.push((text.galley.job.text.clone(), text.visual_bounding_rect()));
                }
            }
            labels
        };
        let find = |labels: &[(String, egui::Rect)], text: &str| {
            labels
                .iter()
                .find(|(drawn, _)| drawn == text)
                .unwrap_or_else(|| panic!("{text} not drawn"))
                .1
        };
        frame(&mut app, vec![]);
        let labels = frame(&mut app, vec![]);
        let t = Translator(Language::EnUs);
        let checked = find(&labels, t.get("apt.last_check"));
        assert!(checked.left() > 900.0, "last column at {checked:?}");
        let phased = find(&labels, t.get("batch.phased"));
        assert!(
            labels
                .iter()
                .any(|(text, rect)| text == "2"
                    && (rect.center().x - phased.center().x).abs() < 80.0)
        );
        let system = find(&labels, t.get("batch.system"));
        let edge = egui::pos2(system.left() - 6.0, system.center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let to = edge + egui::vec2(120.0, 0.0);
        frame(&mut app, vec![egui::Event::PointerMoved(edge)]);
        frame(&mut app, vec![button(edge, true)]);
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(edge + egui::vec2(60.0, 0.0))],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(to)]);
        frame(&mut app, vec![button(to, false)]);
        let labels = frame(&mut app, vec![]);
        assert!(find(&labels, t.get("batch.system")).left() > system.left() + 100.0);
        let saved = crate::preferences::Preferences::load(&app.settings_path).unwrap();
        assert!(saved.batch_columns.weights[0] > 2.5);

        // In popup mode the packages wait for a double-click and Escape closes them.
        let title = |name: &str| t.format("batch.detail", &[("name", name.into())]);
        let drawn = |labels: &[(String, egui::Rect)], text: &str| {
            labels.iter().any(|(drawn, _)| drawn == text)
        };
        let labels = frame(&mut app, vec![]);
        assert!(drawn(&labels, &title(&app.current().unwrap().name)));
        app.preferences.batch_detail = BatchDetail::Popup;
        let labels = frame(&mut app, vec![]);
        assert!(
            !labels
                .iter()
                .any(|(text, _)| text.starts_with("Packages on"))
        );
        let host = find(&labels, "bravo").center();
        frame(&mut app, vec![egui::Event::PointerMoved(host)]);
        for pressed in [true, false, true, false] {
            frame(&mut app, vec![button(host, pressed)]);
        }
        frame(&mut app, vec![]);
        let labels = frame(&mut app, vec![]);
        assert_eq!(app.selected, "bravo");
        assert!(drawn(&labels, &title("bravo")));
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(&mut app, vec![escape]);
        let labels = frame(&mut app, vec![]);
        assert!(!app.batch_popup);
        assert!(!drawn(&labels, &title("bravo")));
    }
}
