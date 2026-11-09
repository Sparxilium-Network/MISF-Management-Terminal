//! The top bar, the status line and the host/batch tabs.

use super::{
    App, AppWindow, Ctx, Shell,
    app::{VaultMode, View},
};
use serde_json::json;
use slint::ComponentHandle;

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    let shell = w.global::<Shell>();
    shell.set_busy(app.busy);
    shell.set_status(app.status.clone().into());
    shell.set_locked(app.locked);
    shell.set_encrypted(app.encrypted);
    shell.set_path(app.path.clone().into());
    shell.set_storage_warning(app.storage_warning.clone().into());
    shell.set_plaintext_remains(app.plaintext_exists);
    shell.set_debug_notice(
        match (app.debug, app.passkey_allowed) {
            (false, _) => "",
            (true, true) => t.get("debug.hardware_notice"),
            (true, false) => t.get("debug.notice"),
        }
        .into(),
    );
    shell.set_view(match app.view {
        View::Host => 0,
        View::Batch => 1,
    });
    shell.set_batch_tab(
        t.format(
            "view.batch",
            &[("count", app.batch_selected.len().to_string())],
        )
        .into(),
    );
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let shell = w.global::<Shell>();
    let c = ctx.clone();
    shell.on_settings(move || c.update(App::toggle_settings));
    let c = ctx.clone();
    shell.on_lock(move || c.update(|app| app.send(json!({"action":"vault_lock"}))));
    let c = ctx.clone();
    shell.on_unlock(move || c.update(|app| app.open_vault_dialog(VaultMode::Unlock)));
    let c = ctx.clone();
    shell.on_enable_vault(move || c.update(|app| app.open_vault_dialog(VaultMode::Create)));
    let c = ctx.clone();
    shell.on_manage_keys(move || c.update(App::open_key_dialog));
    let c = ctx.clone();
    shell.on_review_plaintext(move || c.update(App::check_plaintext_copy));
    let c = ctx.clone();
    shell.on_view_chosen(move |view| {
        c.update(|app| {
            app.set_view(if view == 1 { View::Batch } else { View::Host });
        })
    });
    // Closing the main window ends the app, even with the settings window open.
    let c = ctx.clone();
    w.window().on_close_requested(move || {
        if let Some(settings) = c.settings.upgrade() {
            let _ = settings.hide();
        }
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
}
