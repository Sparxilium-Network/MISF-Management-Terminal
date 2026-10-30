use crate::{
    i18n::{Language, Translator},
    preferences::{Preferences, WindowSize},
    theme::{self, Appearance, Palette},
    worker::{Event, Worker},
};
use chrono::{DateTime, Local};
use eframe::egui::{self, Color32, RichText, Stroke};
use misf_core::model::{CheckResult, Host};
use misf_core::{
    audit::{Action, AuditLog, Outcome},
    runtime::RuntimePaths,
    ssh,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
use zeroize::Zeroize;

const NAVY: Color32 = Color32::from_rgb(19, 36, 59);
const BLUE: Color32 = Color32::from_rgb(33, 117, 195);
const RED: Color32 = Color32::from_rgb(178, 58, 58);

struct Editor {
    host: Host,
    port: String,
    show_secrets: bool,
    error: String,
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

#[derive(Clone, Copy, PartialEq)]
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
    saved_window: Option<WindowSize>,
    debug: bool,
    passkey_allowed: bool,
    preferences: Preferences,
    settings_open: bool,
    worker: Worker,
    hosts: Vec<Host>,
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
    /// The one package manager the batch selection may contain.
    batch_manager: String,
    upgrade_confirm: Option<UpgradeConfirm>,
    encrypted: bool,
    /// "sqlite" or "json" from the last host list; empty while locked.
    storage_format: String,
    schema_version: Option<u64>,
    locked: bool,
    plaintext_exists: bool,
    plaintext_path: String,
    plaintext_delete_prompt: bool,
    slots: Vec<SlotRow>,
    storage_warning: String,
    vault_dialog: Option<VaultDialog>,
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
            saved_window: preferences.window,
            debug: paths.debug,
            passkey_allowed: paths.passkey_allowed(),
            preferences,
            settings_open: false,
            worker,
            hosts: vec![],
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
            batch_manager: String::new(),
            upgrade_confirm: None,
            encrypted: false,
            storage_format: String::new(),
            schema_version: None,
            locked: false,
            plaintext_exists: false,
            plaintext_path: String::new(),
            plaintext_delete_prompt: false,
            slots: vec![],
            storage_warning: String::new(),
            vault_dialog: None,
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
        if std::env::var("MISF_OPEN_SETTINGS").as_deref() == Ok("1") {
            app.settings_open = true;
            app.log(Action::SettingsOpen, Outcome::Succeeded);
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
        let mut close = false;
        egui::Window::new(t.get("settings.title"))
            .id(egui::Id::unique("settings_window"))
            .open(&mut open)
            .default_pos([16.0, 70.0])
            .default_width(420.0)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                self.preferences_form(ui);
                ui.add_space(12.0);
                ui.separator();
                if self.about_section(ui) {
                    self.send(json!({"action":"convert_storage"}));
                }
                ui.separator();
                ui.label(t.get("settings.plaintext_notice"));
                ui.add_space(12.0);
                close = ui.button(t.get("common.close")).clicked();
            });
        self.settings_open = open && !close;
        if !self.settings_open {
            self.log(Action::SettingsClose, Outcome::Succeeded);
        }
    }

    /// Returns true when the user asked to convert a legacy JSON store, or rewrite an
    /// encrypted database still stored at an older schema version (upgraded only in memory).
    fn about_section(&self, ui: &mut egui::Ui) -> bool {
        let t = Translator(self.preferences.language);
        ui.label(RichText::new(t.get("settings.about")).strong());
        ui.add_space(4.0);
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
        let current = misf_core::sqlite::SCHEMA_VERSION as u64;
        let action = match (self.storage_format.as_str(), self.schema_version) {
            ("json", _) => Some(t.get("settings.convert_db").to_owned()),
            ("sqlite", Some(version)) if version < current => {
                Some(t.format("settings.upgrade_db", &[("version", current.to_string())]))
            }
            _ => None,
        };
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
        ui.add_space(8.0);
        convert
    }

    fn preferences_form(&mut self, ui: &mut egui::Ui) {
        let before = self.preferences;
        let t = Translator(before.language);
        egui::Grid::new("preferences_form")
            .spacing([24.0, 16.0])
            .show(ui, |ui| {
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
            });
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
        if self.audit.record(action, outcome, host, slot).is_err() {
            self.error = Some(
                Translator(self.preferences.language)
                    .get("audit.failed")
                    .into(),
            );
        }
    }
    fn send(&mut self, request: Value) {
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
                Event::Progress(message) => self.status = t.progress(&message),
                Event::Done { action, result } => {
                    self.busy = false;
                    match result {
                        Ok(data) => self.received(&action, data),
                        Err(error) => {
                            self.status = t.get("status.failed").into();
                            self.error = Some(error);
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
                if self.locked {
                    self.clear_host_cache();
                    self.open_vault_dialog(VaultMode::Unlock);
                    self.status = t.get("vault.locked").into();
                } else {
                    self.check_plaintext_copy();
                    self.send(json!({"action":"list"}));
                }
            }
            "vault_create" | "vault_unlock" | "vault_unlock_passkey" => {
                self.apply_storage(&data);
                self.vault_dialog = None;
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
            "vault_lock" => {
                self.apply_storage(&data);
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
                if let Some(host) = self.hosts.iter_mut().find(|h| h.id == self.selected) {
                    host.distro = distro.into();
                    host.package_manager = manager.into();
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
                self.status = t.format(
                    "status.batch_done",
                    &[
                        ("succeeded", succeeded.to_string()),
                        ("failed", failed.to_string()),
                    ],
                );
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
            host,
            show_secrets: false,
            error: String::new(),
        });
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
        ui.label(
            RichText::new(t.format(
                "host.list_count",
                &[("count", self.hosts.len().to_string())],
            ))
            .size(12.0)
            .color(Color32::from_rgb(147, 174, 208)),
        );
        ui.add_space(6.0);
        let query = self.search.to_lowercase();
        let visible: Vec<_> = self
            .hosts
            .iter()
            .filter(|h| {
                format!("{} {} {}", h.name, h.address, h.notes)
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned()
            .collect();
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 98.0).max(80.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if visible.is_empty() {
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
                for host in visible {
                    let selected = self.selected == host.id;
                    let fill = if selected {
                        Color32::from_rgb(40, 66, 95)
                    } else {
                        NAVY
                    };
                    let card = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(7)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width() - 2.0);
                            let success = host
                                .last_check
                                .as_ref()
                                .is_some_and(|r| r.status == "success");
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").size(10.0).color(if success {
                                    Color32::from_rgb(78, 201, 162)
                                } else {
                                    palette.muted
                                }));
                                ui.label(
                                    RichText::new(&host.name)
                                        .strong()
                                        .size(15.0)
                                        .color(Color32::WHITE),
                                );
                            });
                            ui.label(
                                RichText::new(format!(
                                    "{}@{}:{}",
                                    host.username, host.address, host.port
                                ))
                                .size(11.0)
                                .color(Color32::from_rgb(166, 187, 213)),
                            );
                            if !host.distro.is_empty() {
                                ui.label(
                                    RichText::new(&host.distro)
                                        .size(11.0)
                                        .color(Color32::from_rgb(166, 187, 213)),
                                );
                            }
                        })
                        .response;
                    if ui
                        .interact(card.rect, card.id, egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.selected = host.id;
                        self.log(Action::HostSelect, Outcome::Succeeded);
                    }
                    ui.add_space(5.0);
                }
            });
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
            ui.heading(RichText::new(t.get("vault.locked")).size(29.0));
            ui.label(t.get("vault.unlock_description"));
            ui.add_space(24.0);
            card(ui).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add_space(15.0);
                ui.label(t.get("vault.algorithm"));
                ui.add_space(12.0);
                if ui.add(primary(t.get("vault.unlock_host"))).clicked() {
                    self.open_vault_dialog(VaultMode::Unlock);
                }
                ui.add_space(15.0);
            });
            self.storage_notice(ui);
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
            self.storage_notice(ui);
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
            self.storage_notice(ui);
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
                &[ssh::DETECT],
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
            let detected = ssh::Manager::from_id(host.manager());
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
                &ssh::package_commands(&host, false),
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
                &ssh::package_commands(&host, true),
                t.get("hint.upgrade_effect"),
                t.get("hint.upgrade_risk"),
            )
            .clicked()
            {
                self.prompt_upgrade(vec![host.id.clone()], false);
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
        ui.add_space(16.0);
        self.result_panel(ui, &host, (viewport_height - 500.0).max(100.0));
        self.storage_notice(ui);
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
                ui.selectable_value(&mut self.tab, 2, t.get("apt.output"));
                if before != self.tab {
                    self.log(Action::ResultTab, Outcome::Succeeded);
                }
            });
            ui.separator();
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
            ssh::Manager::from_id(&self.batch_manager)
                .map(|m| ssh::manager_commands(m, false, upgrade))
                .unwrap_or_default()
        };
        let (check_commands, upgrade_commands) = (commands(false), commands(true));
        // Most pending updates first; the stable sort keeps the saved order among equals.
        let mut order: Vec<usize> = (0..self.hosts.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(pending_count(&self.hosts[i])));
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
        let rows: Vec<_> = order
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
                (
                    h.id.clone(),
                    h.name.clone(),
                    reason,
                    if h.distro.is_empty() {
                        "—".to_owned()
                    } else {
                        system_label(t, h)
                    },
                    result.map(|r| {
                        if r.operation == "upgrade" {
                            t.get("batch.operation_upgrade")
                        } else {
                            t.get("batch.operation_check")
                        }
                    }),
                    status_label(t, h),
                    result.is_some_and(|r| r.status == "failed"),
                    success.map(|r| r.packages.len().to_string()),
                    success
                        .filter(|r| r.operation == "upgrade")
                        .map(|r| r.upgraded.len().to_string()),
                    result.map(|r| format_time(&r.checked_at)),
                )
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
                .show(ui, |ui| {
                    egui::Grid::new("batch_hosts_grid")
                        .striped(true)
                        .spacing([18.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("");
                            for caption in [
                                t.get("batch.host"),
                                t.get("batch.system"),
                                t.get("batch.last_operation"),
                                t.get("apt.status"),
                                t.get("apt.pending"),
                                t.get("batch.upgraded_count"),
                                t.get("apt.last_check"),
                            ] {
                                ui.label(RichText::new(caption).strong().color(palette.muted));
                            }
                            ui.end_row();
                            for (
                                id,
                                name,
                                reason,
                                system,
                                operation,
                                status,
                                failed,
                                pending,
                                upgraded,
                                time,
                            ) in rows
                            {
                                let mut checked = self.batch_selected.contains(&id);
                                let mut response = ui.add_enabled(
                                    reason.is_none(),
                                    egui::Checkbox::without_text(&mut checked),
                                );
                                if let Some(reason) = reason {
                                    response = response.on_disabled_hover_text(reason);
                                }
                                if response.changed() {
                                    if checked {
                                        self.batch_selected.insert(id.clone());
                                    } else {
                                        self.batch_selected.remove(&id);
                                    }
                                    self.log(Action::BatchSelect, Outcome::Succeeded);
                                }
                                if ui.selectable_label(self.selected == id, name).clicked() {
                                    self.selected = id;
                                    self.log(Action::HostSelect, Outcome::Succeeded);
                                }
                                ui.label(system);
                                ui.label(operation.unwrap_or("—"));
                                ui.label(RichText::new(status).color(if failed {
                                    palette.error
                                } else {
                                    palette.ink
                                }));
                                ui.label(pending.unwrap_or("—".into()));
                                ui.label(upgraded.unwrap_or("—".into()));
                                ui.label(time.unwrap_or("—".into()));
                                ui.end_row();
                            }
                        });
                });
        });
        ui.add_space(12.0);
        let Some(host) = self.current().cloned() else {
            return;
        };
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
        self.result_panel(ui, &host, (viewport_height - 560.0).max(160.0));
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
            Some(host) if !confirm.batch => ssh::package_commands(host, true),
            Some(host) => ssh::Manager::from_id(host.manager())
                .map(|m| ssh::manager_commands(m, false, true))
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

    fn storage_notice(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        ui.add_space(15.0);
        ui.label(
            RichText::new(t.get("app.development_warning"))
                .strong()
                .color(palette.warning),
        );
        ui.label(t.get("app.development_detail"));
        ui.label(
            RichText::new(if self.encrypted {
                t.get("storage.encrypted")
            } else {
                t.get("storage.plaintext")
            })
            .size(11.0)
            .color(palette.warning),
        );
        ui.label(RichText::new(&self.path).size(10.0).color(palette.muted));
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
        ui.horizontal_wrapped(|ui| {
            if self.encrypted {
                if self.locked {
                    if ui.button(t.get("vault.unlock_host")).clicked() {
                        self.open_vault_dialog(VaultMode::Unlock);
                    }
                } else {
                    if ui.button(t.get("vault.lock")).clicked() {
                        self.send(json!({"action":"vault_lock"}));
                    }
                    if ui.button(t.get("vault.manage")).clicked() {
                        self.open_vault_dialog(VaultMode::Manage);
                    }
                    if ui.button(t.get("vault.export")).clicked() {
                        self.choose_backup();
                    }
                }
            } else if ui.button(t.get("vault.enable_button")).clicked() {
                self.open_vault_dialog(VaultMode::Create);
            }
        });
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
                        cancel = ui.button(t.get("common.cancel")).clicked();
                    });
                } else {
                    if dialog.mode == VaultMode::Create {
                        ui.label(t.get("vault.create_description"));
                        ui.colored_label(palette.warning, t.get("vault.experimental"));
                        ui.label(t.get("vault.retain_plaintext"));
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
            } else if dialog.mode != VaultMode::Unlock && dialog.password.chars().count() < 12 {
                dialog.error = t.get("vault.password_short").into();
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
                            let choices_before =
                                (editor.host.os.clone(), editor.host.auth_type.clone());
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
                                egui::ComboBox::from_label(t.get("auth.method"))
                                    .selected_text(if editor.host.auth_type == "key" {
                                        t.get("auth.private_key")
                                    } else {
                                        t.get("auth.password")
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut editor.host.auth_type,
                                            "password".into(),
                                            t.get("auth.password"),
                                        );
                                        ui.selectable_value(
                                            &mut editor.host.auth_type,
                                            "key".into(),
                                            t.get("auth.private_key"),
                                        );
                                    });
                            });
                            editor_changed |= choices_before
                                != (editor.host.os.clone(), editor.host.auth_type.clone());
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
                                    self.log(Action::ImportKey, Outcome::Started);
                                    if let Some(path) = rfd::FileDialog::new()
                                        .set_title(t.get("auth.import_title"))
                                        .pick_file()
                                    {
                                        let result = std::fs::metadata(&path).and_then(|m| {
                                            if m.len() <= 1024 * 1024 {
                                                std::fs::read_to_string(&path)
                                            } else {
                                                Err(std::io::Error::other(
                                                    t.get("auth.key_too_large"),
                                                ))
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
                                        match result {
                                            Ok(key) => {
                                                editor.host.private_key = key;
                                                editor.error.clear();
                                            }
                                            Err(error) => {
                                                editor.error = t.format(
                                                    "auth.key_read_error",
                                                    &[("error", error.to_string())],
                                                )
                                            }
                                        }
                                    } else {
                                        self.log(Action::ImportKey, Outcome::Cancelled);
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
                                RichText::new(if self.encrypted {
                                    t.get("storage.credentials_encrypted")
                                } else {
                                    t.get("storage.credentials_plaintext")
                                })
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
                match editor.port.trim().parse::<u16>() {
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
        self.vault_dialog(ctx);
        self.plaintext_delete_dialog(ctx);
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
        if self.window_loaded && self.preferences.window != self.saved_window {
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
    !host.fingerprint.is_empty() && ssh::Manager::from_id(host.manager()).is_some()
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
fn manager_label(t: Translator, id: &str) -> &'static str {
    ssh::Manager::from_id(id)
        .map(ssh::Manager::label)
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
            },
        );
        assert_eq!(commands.recv().unwrap()["action"], "storage_status");
        let full_path = directory.path().join("hosts.db").display().to_string();
        let mut status = json!({"encrypted":false, "locked":false, "plaintext_exists":true, "plaintext_path":full_path});
        app.received("storage_status", status.clone());
        assert!(!app.plaintext_delete_prompt);
        assert_eq!(commands.recv().unwrap()["action"], "list");
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
        app.received("vault_unlock", status);
        assert!(!app.plaintext_delete_prompt);
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
            assert!(text.contains(ssh::SUDO_UPGRADE));
            assert!(app.upgrade_confirm.is_some());
        }
        assert!(commands.try_recv().is_err());
    }
}
