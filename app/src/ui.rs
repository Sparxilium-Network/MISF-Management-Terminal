use crate::{
    i18n::{Language, Translator},
    preferences::Preferences,
    theme::{self, Appearance, Palette},
    worker::{Event, Worker},
};
use chrono::{DateTime, Local};
use eframe::egui::{self, Color32, RichText, Stroke};
use misf_core::model::{CheckResult, Host};
use misf_core::{
    audit::{Action, AuditLog, Outcome},
    runtime::RuntimePaths,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use zeroize::Zeroize;

const NAVY: Color32 = Color32::from_rgb(19, 36, 59);
const BLUE: Color32 = Color32::from_rgb(33, 117, 195);

struct Editor {
    host: Host,
    port: String,
    show_secrets: bool,
    error: String,
}
struct TrustDialog {
    host_id: String,
    address: String,
    old: String,
    actual: String,
    next: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum VaultMode {
    Create,
    Unlock,
    Manage,
    AddKey,
    Passkey,
    Revoke(usize),
}
struct VaultDialog {
    mode: VaultMode,
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
    debug: bool,
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
    encrypted: bool,
    locked: bool,
    plaintext_exists: bool,
    slots: Vec<(usize, String)>,
    storage_warning: String,
    vault_dialog: Option<VaultDialog>,
    #[cfg(feature = "capture")]
    capture_started: std::time::Instant,
    #[cfg(feature = "capture")]
    capture_requested: bool,
}

impl ManagementApp {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: RuntimePaths, audit: AuditLog) -> Self {
        let settings_path = paths.store.with_file_name("ui-settings.json");
        let loaded = Preferences::load(&settings_path);
        let preferences = loaded.clone().unwrap_or_default();
        theme::install(&cc.egui_ctx);
        preferences.appearance.apply(&cc.egui_ctx);
        let worker = Worker::new(cc.egui_ctx.clone(), paths.clone(), audit.clone());
        let mut app = Self {
            audit,
            settings_path,
            debug: paths.debug,
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
            encrypted: false,
            locked: false,
            plaintext_exists: false,
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
        app
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
                ui.label(t.get("settings.plaintext_notice"));
                ui.add_space(12.0);
                close = ui.button(t.get("common.close")).clicked();
            });
        self.settings_open = open && !close;
        if !self.settings_open {
            self.log(Action::SettingsClose, Outcome::Succeeded);
        }
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
                    self.send(json!({"action":"list"}));
                }
            }
            "vault_create" | "vault_unlock" | "vault_unlock_passkey" => {
                self.apply_storage(&data);
                self.vault_dialog = None;
                self.send(json!({"action":"list"}));
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
                self.status = t.format(
                    "status.connected",
                    &[("os", data["os"].as_str().unwrap_or_default().into())],
                )
            }
            "check" => match serde_json::from_value::<CheckResult>(data) {
                Ok(result) => {
                    self.status = if result.status == "success" {
                        t.get("status.apt_done")
                    } else {
                        t.get("status.apt_failed")
                    }
                    .into();
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == self.selected) {
                        host.last_check = Some(result);
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            },
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
                if ui
                    .add_enabled(
                        host.is_some(),
                        egui::Button::new(
                            RichText::new(t.get("common.delete")).color(palette.error),
                        ),
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
                host.os,
                if host.auth_type == "key" {
                    "SSH Key"
                } else {
                    t.get("auth.password_login")
                }
            ))
            .color(palette.muted),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button(t.get("host.test")).clicked() {
                self.start_connection_action("test");
            }
            if ui.button(t.get("host.fingerprint")).clicked() {
                self.after_trust = None;
                self.send(json!({"action":"probe","id":self.selected}));
            }
            if ui
                .add_enabled(host.os != "BSD", primary(t.get("apt.check")))
                .clicked()
            {
                self.start_connection_action("check");
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
            stat(
                &mut columns[1],
                t.get("apt.status"),
                result
                    .map(|r| match r.status.as_str() {
                        "success" => t.get("apt.complete"),
                        "unsupported" => t.get("apt.unsupported"),
                        _ => t.get("apt.failed"),
                    })
                    .unwrap_or(if host.os == "BSD" {
                        t.get("apt.unsupported")
                    } else {
                        t.get("apt.not_checked")
                    }),
            );
            stat(
                &mut columns[2],
                t.get("apt.last_check"),
                &result
                    .map(|r| format_time(&r.checked_at))
                    .unwrap_or("—".into()),
            );
        });
        ui.add_space(16.0);
        card(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let before = self.tab;
                ui.selectable_value(&mut self.tab, 0, t.get("apt.packages"));
                ui.selectable_value(&mut self.tab, 1, t.get("apt.output"));
                if before != self.tab {
                    self.log(Action::ResultTab, Outcome::Succeeded);
                }
            });
            ui.separator();
            let available = (viewport_height - 500.0).max(100.0);
            if self.tab == 1 {
                egui::ScrollArea::both()
                    .max_height(available)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let text = result
                            .map(|r| format!("{}\n{}", r.output, r.error))
                            .unwrap_or(t.get("apt.not_run").into());
                        ui.label(RichText::new(text).monospace().size(12.0));
                    });
            } else {
                let message = match result {
                    Some(r) if r.status == "success" => {
                        if r.packages.is_empty() {
                            t.get("apt.up_to_date")
                        } else {
                            t.get("apt.available_description")
                        }
                    }
                    Some(r) => &r.error,
                    None if host.os == "BSD" => t.get("apt.bsd_description"),
                    None => t.get("apt.check_hint"),
                };
                ui.label(RichText::new(message).color(
                    if result.is_some_and(|r| r.status == "failed") {
                        palette.error
                    } else {
                        palette.muted
                    },
                ));
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
                egui::ScrollArea::both()
                    .max_height((available - 75.0).max(70.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("packages")
                            .striped(true)
                            .min_col_width(130.0)
                            .spacing([20.0, 14.0])
                            .show(ui, |ui| {
                                for caption in [
                                    t.get("apt.package"),
                                    t.get("apt.current"),
                                    t.get("apt.candidate"),
                                    t.get("apt.architecture"),
                                ] {
                                    ui.label(RichText::new(caption).strong().color(palette.muted));
                                }
                                ui.end_row();
                                if let Some(result) = result {
                                    for package in result.packages.iter().filter(|p| {
                                        p.name
                                            .to_lowercase()
                                            .contains(&self.package_search.to_lowercase())
                                    }) {
                                        ui.label(&package.name);
                                        ui.label(&package.current_version);
                                        ui.label(&package.candidate_version);
                                        ui.label(&package.architecture);
                                        ui.end_row();
                                    }
                                }
                            });
                        if result.is_none_or(|r| r.packages.is_empty()) {
                            ui.add_space(28.0);
                            ui.label(RichText::new(t.get("apt.no_data")).color(palette.muted));
                        }
                    });
            }
        });
        self.storage_notice(ui);
    }

    fn storage_notice(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let t = Translator(self.preferences.language);
        ui.add_space(15.0);
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
        self.path = data["path"].as_str().unwrap_or_default().into();
        self.storage_warning = data["warning"].as_str().unwrap_or_default().into();
        self.slots = data["slots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|slot| {
                slot["index"].as_u64().map(|index| {
                    (
                        index as usize,
                        slot["kind"].as_str().unwrap_or_default().to_owned(),
                    )
                })
            })
            .collect();
    }

    fn clear_host_cache(&mut self) {
        for host in &mut self.hosts {
            wipe_host(host);
        }
        self.hosts.clear();
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
    }

    fn open_vault_dialog(&mut self, mode: VaultMode) {
        self.log(Action::VaultDialogOpen, Outcome::Succeeded);
        self.vault_dialog = Some(VaultDialog {
            mode,
            password: String::new(),
            confirmation: String::new(),
            error: String::new(),
            hello: cfg!(target_os = "windows"),
            acknowledged: false,
        });
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
        egui::Modal::new(egui::Id::unique("vault_dialog")).show(ctx, |ui| {
            ui.set_width(570.0);
            ui.heading(match dialog.mode {
                VaultMode::Create => t.get("vault.enable"),
                VaultMode::Unlock => t.get("vault.unlock_host"),
                VaultMode::Manage => t.get("vault.manage"),
                VaultMode::AddKey => t.get("vault.add_password"),
                VaultMode::Passkey => t.get("passkey.title"),
                VaultMode::Revoke(_) => t.get("vault.revoke"),
            });
            ui.add_space(12.0);
            ui.add_enabled_ui(!self.busy, |ui| {
                if dialog.mode == VaultMode::Manage {
                    ui.label(t.format(
                        "vault.slot_count",
                        &[("count", self.slots.len().to_string())],
                    ));
                    for (slot, kind) in &self.slots {
                        ui.horizontal(|ui| {
                            ui.label(t.format("vault.slot", &[("slot", slot.to_string())]));
                            ui.label(if kind == "Fido2HmacSecret" {
                                t.get("passkey.title")
                            } else {
                                t.get("auth.password")
                            });
                            if ui
                                .add_enabled(
                                    self.slots.len() > 1,
                                    egui::Button::new(t.get("vault.revoke")),
                                )
                                .clicked()
                            {
                                revoke = Some(*slot);
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
                                !self.debug && self.slots.len() < 8,
                                egui::Button::new(t.get("passkey.add")),
                            )
                            .clicked();
                        backup = ui.button(t.get("vault.export")).clicked();
                        cancel = ui.button(t.get("common.close")).clicked();
                    });
                } else if let VaultMode::Revoke(slot) = dialog.mode {
                    ui.label(t.format("vault.revoke_prompt", &[("slot", slot.to_string())]));
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
                                !self.debug,
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
                            .add_enabled(!self.debug, egui::Button::new(t.get("passkey.unlock")))
                            .clicked();
                    }
                }
                if self.debug {
                    ui.colored_label(palette.warning, t.get("debug.notice"));
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
            } else if dialog.mode == VaultMode::Passkey {
                let action = if self.locked {
                    "vault_unlock_passkey"
                } else {
                    "vault_add_passkey"
                };
                self.send(json!({"action":action,"hello":dialog.hello,"pin":std::mem::take(&mut dialog.password)}));
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
        self.vault_dialog(ctx);
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
                    ui.colored_label(Palette::for_context(&ctx).warning, t.get("debug.notice"));
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
