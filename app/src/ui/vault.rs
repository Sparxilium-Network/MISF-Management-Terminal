//! The LUKSbox vault: storage status, the vault dialog, the leftover plaintext copy and
//! the schema upgrade question.

use super::{
    App, AppWindow, Ctx, PlaintextState, SchemaState, SlotView, VaultState,
    app::{SlotRow, VaultDialog, VaultMode},
};
use crate::preferences::UnlockMethod;
use misf_core::{
    audit::{Action, Outcome},
    vault::{MIN_PASSPHRASE_CHARS, PassphraseProblem, passphrase_problem},
};
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use zeroize::Zeroize;

impl App {
    pub fn apply_storage(&mut self, data: &Value) {
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

    pub fn check_plaintext_copy(&mut self) {
        if self.encrypted
            && !self.locked
            && self.plaintext_exists
            && !self.plaintext_path.is_empty()
        {
            self.plaintext_delete_prompt = true;
            self.log(Action::PlaintextDeletePrompt, Outcome::Succeeded);
        }
    }

    pub fn respond_plaintext_delete(&mut self, delete: bool) {
        if delete {
            self.send(json!({"action":"vault_delete_plaintext", "path":self.plaintext_path, "confirmed":true}));
        } else {
            self.plaintext_delete_prompt = false;
            self.log(Action::PlaintextDeleteCancel, Outcome::Cancelled);
        }
    }

    /// Starts the unlock method chosen in preferences, so a passkey prompt appears without
    /// first choosing it in the dialog. A PIN is still typed where the platform asks for one.
    pub fn start_default_unlock(&mut self) {
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

    pub fn open_vault_dialog(&mut self, mode: VaultMode) {
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
        self.reload.vault = true;
    }

    /// Moves the open vault dialog to another mode with empty fields.
    fn switch_vault(&mut self, mode: VaultMode, name: String) {
        if let Some(dialog) = &mut self.vault_dialog {
            dialog.mode = mode;
            dialog.name = name;
            dialog.password.zeroize();
            dialog.confirmation.zeroize();
            dialog.error.clear();
            dialog.acknowledged = false;
            self.reload.vault = true;
        }
    }

    /// "#1（Google Titan）", or "#1" for an unnamed slot.
    pub fn slot_title(&self, index: usize) -> String {
        let t = self.t();
        match self.slots.iter().find(|slot| slot.index == index) {
            Some(slot) if !slot.name.is_empty() => t.format(
                "vault.slot_named",
                &[("slot", index.to_string()), ("name", slot.name.clone())],
            ),
            _ => format!("#{index}"),
        }
    }

    /// Validates the vault dialog and sends its request.
    pub fn submit_vault(&mut self) {
        let t = self.t();
        let locked = self.locked;
        let Some(dialog) = &mut self.vault_dialog else {
            return;
        };
        let request = if let VaultMode::Revoke(slot) = dialog.mode {
            if !dialog.acknowledged {
                return;
            }
            json!({"action":"vault_revoke_key","slot":slot})
        } else if let VaultMode::Rename(slot) = dialog.mode {
            json!({"action":"vault_rename_key","slot":slot,"name":dialog.name})
        } else if dialog.mode == VaultMode::Passkey {
            if !self.passkey_allowed {
                return;
            }
            let action = if locked {
                "vault_unlock_passkey"
            } else {
                "vault_add_passkey"
            };
            json!({"action":action,"hello":dialog.hello,"name":dialog.name,"pin":std::mem::take(&mut dialog.password)})
        } else if dialog.mode == VaultMode::Manage {
            return;
        } else {
            dialog.error.clear();
            if dialog.mode != VaultMode::Unlock && dialog.password != dialog.confirmation {
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
            }
            if !dialog.error.is_empty() {
                self.log(Action::ValidationRejected, Outcome::Failed);
                return;
            }
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
            // The typed secrets leave the window as well.
            self.reload.vault = true;
            request
        };
        self.send(request);
    }

    pub fn cancel_vault(&mut self) {
        let Some(dialog) = self.vault_dialog.take() else {
            return;
        };
        if let VaultMode::Revoke(slot) = dialog.mode {
            self.log_at_slot(Action::VaultRevokeCancel, Outcome::Cancelled, Some(slot));
        } else {
            self.log(Action::VaultDialogClose, Outcome::Succeeded);
        }
    }

    pub fn vault_add_password(&mut self) {
        self.switch_vault(VaultMode::AddKey, String::new());
        self.log(Action::VaultDialogOpen, Outcome::Succeeded);
    }

    pub fn vault_passkey(&mut self) {
        self.switch_vault(VaultMode::Passkey, String::new());
        self.log(Action::VaultDialogOpen, Outcome::Succeeded);
    }

    pub fn vault_use_password(&mut self) {
        self.switch_vault(VaultMode::Unlock, String::new());
        self.log(Action::VaultDialogOpen, Outcome::Succeeded);
    }

    pub fn vault_rename(&mut self, slot: usize) {
        let name = (self.slots.iter().find(|s| s.index == slot))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        self.switch_vault(VaultMode::Rename(slot), name);
        self.log_at_slot(Action::VaultDialogOpen, Outcome::Succeeded, Some(slot));
    }

    pub fn vault_revoke(&mut self, slot: usize) {
        self.switch_vault(VaultMode::Revoke(slot), String::new());
        self.log_at_slot(Action::VaultRevokePrompt, Outcome::Succeeded, Some(slot));
    }
}

/// Asks where to save an encrypted backup, outside [`Ctx::update`] because the native
/// dialog blocks.
pub fn choose_backup(ctx: &Ctx) {
    ctx.update(|app| app.log(Action::BackupDialog, Outcome::Started));
    let t = ctx.t();
    let path = rfd::FileDialog::new()
        .set_title(t.get("vault.export_title"))
        .set_file_name("misf-backup.lbx")
        .add_filter("LUKSbox", &["lbx"])
        .save_file();
    ctx.update(|app| match path {
        Some(path) => app.send(json!({"action":"vault_backup","path":path})),
        None => app.log(Action::BackupDialog, Outcome::Cancelled),
    });
}

fn pull(app: &mut App, w: &AppWindow) {
    let state = w.global::<VaultState>();
    let Some(dialog) = &mut app.vault_dialog else {
        return;
    };
    dialog.name = state.get_name().into();
    dialog.password = state.get_password().into();
    dialog.confirmation = state.get_confirmation().into();
    dialog.acknowledged = state.get_acknowledged();
}

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    let state = w.global::<VaultState>();
    match &app.vault_dialog {
        None => {
            if state.get_shown() {
                state.set_password(SharedString::new());
                state.set_confirmation(SharedString::new());
                state.set_name(SharedString::new());
                state.set_shown(false);
            }
        }
        Some(dialog) => {
            if std::mem::take(&mut app.reload.vault) {
                state.set_name(dialog.name.clone().into());
                state.set_password(dialog.password.clone().into());
                state.set_confirmation(dialog.confirmation.clone().into());
                state.set_acknowledged(dialog.acknowledged);
            }
            state.set_shown(true);
            state.set_mode(match dialog.mode {
                VaultMode::Create => 0,
                VaultMode::Unlock => 1,
                VaultMode::Manage => 2,
                VaultMode::AddKey => 3,
                VaultMode::Passkey => 4,
                VaultMode::Revoke(_) => 5,
                VaultMode::Rename(_) => 6,
            });
            state.set_slots(ModelRc::new(VecModel::from(
                app.slots
                    .iter()
                    .map(|slot| SlotView {
                        index: slot.index as i32,
                        title: t
                            .format("vault.slot", &[("slot", slot.index.to_string())])
                            .into(),
                        kind: if slot.kind == "Fido2HmacSecret" {
                            t.get("passkey.title")
                        } else {
                            t.get("auth.password")
                        }
                        .into(),
                        name: slot.name.clone().into(),
                    })
                    .collect::<Vec<_>>(),
            )));
            state.set_slot_count(
                t.format(
                    "vault.slot_count",
                    &[("count", app.slots.len().to_string())],
                )
                .into(),
            );
            state.set_can_add(app.slots.len() < 8);
            state.set_passkey_allowed(app.passkey_allowed);
            state.set_locked(app.locked);
            state.set_plaintext_exists(app.plaintext_exists);
            state.set_windows(cfg!(target_os = "windows"));
            state.set_hello(dialog.hello);
            let slot = match dialog.mode {
                VaultMode::Revoke(slot) | VaultMode::Rename(slot) => Some(slot),
                _ => None,
            };
            state.set_slot_label(
                slot.map(|slot| t.format("vault.slot", &[("slot", slot.to_string())]))
                    .unwrap_or_default()
                    .into(),
            );
            state.set_revoke_prompt(
                slot.map(|slot| t.format("vault.revoke_prompt", &[("slot", app.slot_title(slot))]))
                    .unwrap_or_default()
                    .into(),
            );
            state.set_error(dialog.error.clone().into());
        }
    }
    let plaintext = w.global::<PlaintextState>();
    plaintext.set_shown(app.plaintext_delete_prompt && !app.locked);
    plaintext.set_path(app.plaintext_path.clone().into());
    let schema = w.global::<SchemaState>();
    schema.set_shown(app.schema_upgrade_prompt.is_some());
    if let Some(from) = app.schema_upgrade_prompt {
        let versions = [
            ("from", from.to_string()),
            ("to", misf_core::sqlite::SCHEMA_VERSION.to_string()),
        ];
        schema.set_description(t.format("schema_upgrade.description", &versions).into());
        schema.set_confirm_label(t.format("schema_upgrade.confirm", &versions).into());
    }
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<VaultState>();
    let c = ctx.clone();
    state.on_submit(move || {
        c.update_with(|app, w| {
            pull(app, w);
            app.submit_vault();
        })
    });
    let c = ctx.clone();
    state.on_cancel(move || c.update(App::cancel_vault));
    let c = ctx.clone();
    state.on_add_password(move || c.update(App::vault_add_password));
    let c = ctx.clone();
    state.on_passkey(move || c.update(App::vault_passkey));
    let c = ctx.clone();
    state.on_use_password(move || c.update(App::vault_use_password));
    let c = ctx.clone();
    state.on_rename(move |slot| c.update(|app| app.vault_rename(slot.max(0) as usize)));
    let c = ctx.clone();
    state.on_revoke(move |slot| c.update(|app| app.vault_revoke(slot.max(0) as usize)));
    let c = ctx.clone();
    state.on_backup(move || choose_backup(&c));
    let c = ctx.clone();
    state.on_hello_chosen(move |hello| {
        c.update_with(|app, w| {
            pull(app, w);
            if let Some(dialog) = &mut app.vault_dialog
                && dialog.hello != hello
            {
                dialog.hello = hello;
                app.log(Action::PasskeyMethod, Outcome::Succeeded);
            }
        })
    });
    let plaintext = w.global::<PlaintextState>();
    let c = ctx.clone();
    plaintext.on_keep(move || c.update(|app| app.respond_plaintext_delete(false)));
    let c = ctx.clone();
    plaintext.on_delete(move || c.update(|app| app.respond_plaintext_delete(true)));
    let schema = w.global::<SchemaState>();
    let c = ctx.clone();
    schema.on_upgrade(move || c.update(|app| app.send(json!({"action":"vault_upgrade"}))));
    let c = ctx.clone();
    schema.on_cancel(move || c.update(|app| app.send(json!({"action":"vault_upgrade_cancel"}))));
}
