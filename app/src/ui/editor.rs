//! The host editor and the stored SSH key dialog.

use super::{
    App, AppWindow, Ctx, EditorState, KeyView, KeysState,
    app::{Editor, KeyDialog, KeyRow, wipe_host},
    format::manager_label,
    texts,
};
use crate::i18n::Translator;
use misf_core::{
    audit::{Action, Outcome},
    model::Host,
};
use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use zeroize::Zeroize;

/// The editor's authentication choices, in the order the Slint list shows them.
const AUTH: [(&str, bool); 3] = [("password", false), ("key", false), ("key", true)];

impl App {
    pub fn open_editor(&mut self, host: Option<Host>) {
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
        self.reload.editor = true;
    }

    pub fn close_editor(&mut self) {
        if let Some(editor) = &mut self.editor {
            wipe_host(&mut editor.host);
            editor.port.zeroize();
        }
        self.editor = None;
    }

    pub fn open_key_dialog(&mut self) {
        if self.locked {
            return;
        }
        self.log(Action::SshKeyDialog, Outcome::Succeeded);
        self.key_dialog = Some(KeyDialog::default());
        self.reload.keys = true;
    }

    /// Validates the editor and sends its host to be saved.
    pub fn save_editor(&mut self) {
        let t = self.t();
        let Some(editor) = &mut self.editor else {
            return;
        };
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
                editor.error.clear();
                let request = json!({"action":"save","host":editor.host});
                self.send(request);
            }
            _ => {
                editor.error = t.get("host.port_invalid").into();
                self.log(Action::ValidationRejected, Outcome::Failed);
            }
        }
    }

    /// Picks the editor's OS family; a package manager the new family lacks is dropped.
    pub fn editor_os(&mut self, os: &str) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.host.os = os.to_owned();
        let managers = misf_core::model::manager_choices(&editor.host.os);
        if !managers.contains(&editor.host.manager_choice.as_str()) {
            editor.host.manager_choice.clear();
        }
        self.log(Action::FormEdit, Outcome::Succeeded);
    }

    pub fn add_key(&mut self) {
        let t = self.t();
        let Some(dialog) = &mut self.key_dialog else {
            return;
        };
        if dialog.name.trim().is_empty() || dialog.private_key.trim().is_empty() {
            dialog.error = t.get("keys.incomplete").into();
            self.log(Action::ValidationRejected, Outcome::Failed);
            return;
        }
        dialog.error.clear();
        let request = json!({
            "action": "key_add",
            "name": dialog.name,
            "private_key": dialog.private_key,
            "passphrase": dialog.passphrase,
        });
        self.send(request);
    }
}

/// Asks for a private key file and reads it; `None` when the user cancels. Runs outside
/// [`Ctx::update`] because the native dialog blocks.
fn pick_key_file(t: Translator) -> Option<Result<String, String>> {
    let path = rfd::FileDialog::new()
        .set_title(t.get("auth.import_title"))
        .pick_file()?;
    let result = std::fs::metadata(&path).and_then(|m| {
        if m.len() <= 1024 * 1024 {
            std::fs::read_to_string(&path)
        } else {
            Err(std::io::Error::other(t.get("auth.key_too_large")))
        }
    });
    Some(result.map_err(|error| t.format("auth.key_read_error", &[("error", error.to_string())])))
}

/// Logs and runs the key file picker, then hands the result to `apply`.
fn import_key(ctx: &Ctx, apply: impl FnOnce(&mut App, Result<String, String>)) {
    ctx.update(|app| app.log(Action::ImportKey, Outcome::Started));
    let picked = pick_key_file(ctx.t());
    ctx.update(|app| match picked {
        None => app.log(Action::ImportKey, Outcome::Cancelled),
        Some(result) => {
            app.log(
                Action::ImportKey,
                if result.is_ok() {
                    Outcome::Succeeded
                } else {
                    Outcome::Failed
                },
            );
            apply(app, result);
        }
    });
}

/// Copies what was typed in the editor into its state.
fn pull_editor(app: &mut App, w: &AppWindow) {
    let state = w.global::<EditorState>();
    let Some(editor) = &mut app.editor else {
        return;
    };
    let host = &mut editor.host;
    host.name = state.get_name().into();
    host.address = state.get_address().into();
    editor.port = state.get_port().into();
    host.username = state.get_username().into();
    host.password = state.get_password().into();
    host.private_key = state.get_private_key().into();
    host.key_passphrase = state.get_key_passphrase().into();
    host.sudo_password = state.get_sudo_password().into();
    host.notes = state.get_notes().into();
}

fn pull_keys(app: &mut App, w: &AppWindow) {
    let state = w.global::<KeysState>();
    let Some(dialog) = &mut app.key_dialog else {
        return;
    };
    dialog.name = state.get_name().into();
    dialog.private_key = state.get_private_key().into();
    dialog.passphrase = state.get_passphrase().into();
}

fn manager_choices(t: Translator, editor: &Editor) -> (Vec<String>, i32) {
    let managers = misf_core::model::manager_choices(&editor.host.os);
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
    let labels = managers
        .iter()
        .map(|id| {
            if id.is_empty() {
                auto.clone()
            } else {
                manager_label(t, id).to_owned()
            }
        })
        .collect();
    let index = managers
        .iter()
        .position(|id| *id == editor.host.manager_choice)
        .unwrap_or(0);
    (labels, index as i32)
}

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    push_keys(app, w, t);
    let state = w.global::<EditorState>();
    let Some(editor) = &app.editor else {
        if state.get_shown() {
            // Nothing typed stays in the window once the editor closes.
            let empty = SharedString::new;
            state.set_password(empty());
            state.set_private_key(empty());
            state.set_key_passphrase(empty());
            state.set_sudo_password(empty());
            state.set_name(empty());
            state.set_address(empty());
            state.set_username(empty());
            state.set_notes(empty());
            state.set_shown(false);
        }
        return;
    };
    if std::mem::take(&mut app.reload.editor) {
        let host = &editor.host;
        state.set_name(host.name.clone().into());
        state.set_address(host.address.clone().into());
        state.set_port(editor.port.clone().into());
        state.set_username(host.username.clone().into());
        state.set_password(host.password.clone().into());
        state.set_private_key(host.private_key.clone().into());
        state.set_key_passphrase(host.key_passphrase.clone().into());
        state.set_sudo_password(host.sudo_password.clone().into());
        state.set_notes(host.notes.clone().into());
    }
    state.set_shown(true);
    state.set_adding(editor.host.id.is_empty());
    state.set_os(if editor.host.os == "BSD" { 1 } else { 0 });
    let saved = editor.host.auth_type == "key" && editor.saved_key;
    state.set_auth(match (editor.host.auth_type.as_str(), saved) {
        ("key", true) => 2,
        ("key", false) => 1,
        _ => 0,
    });
    let (managers, manager) = manager_choices(t, editor);
    state.set_managers(texts(managers));
    state.set_manager(manager);
    state.set_show_secrets(editor.show_secrets);
    state.set_keys(texts(app.ssh_keys.iter().map(KeyRow::label)));
    let chosen = app.ssh_keys.iter().position(|k| k.id == editor.host.key_id);
    state.set_key(chosen.map_or(-1, |i| i as i32));
    state.set_key_fingerprint(
        chosen
            .map(|i| app.ssh_keys[i].fingerprint.clone())
            .unwrap_or_default()
            .into(),
    );
    state.set_pty(editor.host.use_pty);
    state.set_error(editor.error.clone().into());
}

fn push_keys(app: &mut App, w: &AppWindow, t: Translator) {
    let state = w.global::<KeysState>();
    let Some(dialog) = app.key_dialog.as_ref().filter(|_| !app.locked) else {
        if state.get_shown() {
            state.set_private_key(SharedString::new());
            state.set_passphrase(SharedString::new());
            state.set_name(SharedString::new());
            state.set_shown(false);
        }
        return;
    };
    if std::mem::take(&mut app.reload.keys) {
        state.set_name(dialog.name.clone().into());
        state.set_private_key(dialog.private_key.clone().into());
        state.set_passphrase(dialog.passphrase.clone().into());
    }
    state.set_shown(true);
    state.set_keys(ModelRc::new(VecModel::from(
        app.ssh_keys
            .iter()
            .map(|key| {
                let users = app.hosts.iter().filter(|h| h.key_id == key.id).count();
                KeyView {
                    id: key.id.clone().into(),
                    name: key.name.clone().into(),
                    algorithm: key.algorithm.clone().into(),
                    used: t
                        .format("keys.used_by", &[("count", users.to_string())])
                        .into(),
                    deletable: users == 0,
                    fingerprint: key.fingerprint.clone().into(),
                }
            })
            .collect::<Vec<_>>(),
    )));
    state.set_delete_prompt(
        dialog
            .delete
            .as_ref()
            .map(|id| {
                let name = app
                    .ssh_keys
                    .iter()
                    .find(|key| key.id == *id)
                    .map(|key| key.name.clone())
                    .unwrap_or_default();
                t.format("keys.delete_prompt", &[("name", name)])
            })
            .unwrap_or_default()
            .into(),
    );
    state.set_show_secrets(dialog.show_secrets);
    state.set_error(dialog.error.clone().into());
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<EditorState>();
    let c = ctx.clone();
    state.on_edited(move || c.update(|app| app.log(Action::FormEdit, Outcome::Succeeded)));
    let c = ctx.clone();
    state.on_os_chosen(move |index| {
        c.update_with(|app, w| {
            pull_editor(app, w);
            app.editor_os(if index == 1 { "BSD" } else { "Linux" });
        })
    });
    let c = ctx.clone();
    state.on_auth_chosen(move |index| {
        c.update(|app| {
            let Some((auth, stored)) = usize::try_from(index).ok().and_then(|i| AUTH.get(i)) else {
                return;
            };
            if let Some(editor) = &mut app.editor {
                editor.host.auth_type = (*auth).into();
                editor.saved_key = *stored;
            }
            app.log(Action::FormEdit, Outcome::Succeeded);
        })
    });
    let c = ctx.clone();
    state.on_manager_chosen(move |index| {
        c.update(|app| {
            if let Some(editor) = &mut app.editor {
                let managers = misf_core::model::manager_choices(&editor.host.os);
                if let Some(id) = usize::try_from(index).ok().and_then(|i| managers.get(i)) {
                    editor.host.manager_choice = (*id).into();
                }
            }
            app.log(Action::FormEdit, Outcome::Succeeded);
        })
    });
    let c = ctx.clone();
    state.on_key_chosen(move |index| {
        c.update(|app| {
            let key = usize::try_from(index)
                .ok()
                .and_then(|i| app.ssh_keys.get(i))
                .map(|k| k.id.clone());
            if let (Some(editor), Some(key)) = (&mut app.editor, key) {
                editor.host.key_id = key;
            }
            app.log(Action::FormEdit, Outcome::Succeeded);
        })
    });
    let c = ctx.clone();
    state.on_show_secrets_toggled(move |shown| {
        c.update(|app| {
            if let Some(editor) = &mut app.editor {
                editor.show_secrets = shown;
            }
            app.log(Action::RevealSecrets, Outcome::Succeeded);
        })
    });
    let c = ctx.clone();
    state.on_pty_toggled(move |on| {
        c.update(|app| {
            if let Some(editor) = &mut app.editor {
                editor.host.use_pty = on;
            }
            app.log(Action::FormEdit, Outcome::Succeeded);
        })
    });
    let c = ctx.clone();
    state.on_import_key(move || {
        c.update_with(pull_editor);
        import_key(&c, |app, result| {
            if let Some(editor) = &mut app.editor {
                match result {
                    Ok(key) => {
                        editor.host.private_key = key;
                        editor.error.clear();
                        app.reload.editor = true;
                    }
                    Err(error) => editor.error = error,
                }
            }
        });
    });
    let c = ctx.clone();
    state.on_manage_keys(move || {
        c.update_with(|app, w| {
            pull_editor(app, w);
            app.open_key_dialog();
        })
    });
    let c = ctx.clone();
    state.on_save(move || {
        c.update_with(|app, w| {
            pull_editor(app, w);
            app.save_editor();
        })
    });
    let c = ctx.clone();
    state.on_cancel(move || {
        c.update(|app| {
            app.close_editor();
            app.log(Action::EditorClose, Outcome::Succeeded);
        })
    });

    let keys = w.global::<KeysState>();
    let c = ctx.clone();
    keys.on_add(move || {
        c.update_with(|app, w| {
            pull_keys(app, w);
            app.add_key();
        })
    });
    let c = ctx.clone();
    keys.on_close(move || c.update(|app| app.key_dialog = None));
    let c = ctx.clone();
    keys.on_delete(move |id| {
        c.update_with(|app, w| {
            pull_keys(app, w);
            if let Some(dialog) = &mut app.key_dialog {
                dialog.delete = Some(id.into());
            }
        })
    });
    let c = ctx.clone();
    keys.on_confirm_delete(move || {
        c.update(|app| {
            let id = app.key_dialog.as_ref().and_then(|d| d.delete.clone());
            if let Some(id) = id {
                app.send(json!({"action":"key_delete","id":id}));
            }
        })
    });
    let c = ctx.clone();
    keys.on_cancel_delete(move || {
        c.update(|app| {
            if let Some(dialog) = &mut app.key_dialog {
                dialog.delete = None;
            }
        })
    });
    let c = ctx.clone();
    keys.on_import_key(move || {
        c.update_with(pull_keys);
        import_key(&c, |app, result| {
            if let Some(dialog) = &mut app.key_dialog {
                match result {
                    Ok(key) => {
                        dialog.private_key = key;
                        dialog.error.clear();
                        app.reload.keys = true;
                    }
                    Err(error) => dialog.error = error,
                }
            }
        });
    });
    let c = ctx.clone();
    keys.on_show_secrets_toggled(move |shown| {
        c.update_with(|app, w| {
            pull_keys(app, w);
            if let Some(dialog) = &mut app.key_dialog {
                dialog.show_secrets = shown;
            }
            app.log(Action::RevealSecrets, Outcome::Succeeded);
        })
    });
}
