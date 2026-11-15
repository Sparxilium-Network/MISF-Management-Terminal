//! The title bar, the status line and the host/batch tabs.

use super::{
    App, AppWindow, CloseState, Ctx, HistoryLine, Shell, WindowChrome,
    app::{VaultMode, View},
};
use serde_json::json;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    let shell = w.global::<Shell>();
    shell.set_busy(app.busy);
    shell.set_status(app.status.clone().into());
    // Nothing left to interrupt: the question no longer applies.
    if !app.busy {
        app.close_prompt = false;
    }
    w.global::<CloseState>().set_shown(app.close_prompt);
    // Only rebuilt when it changed, so an open, scrolled list stays where it is.
    let lines: Vec<HistoryLine> = app
        .history
        .iter()
        .rev()
        .map(|entry| HistoryLine {
            time: entry.time.clone().into(),
            text: entry.text.clone().into(),
            error: entry.error,
        })
        .collect();
    let shown = shell.get_history();
    if shown.row_count() != lines.len() || shown.iter().zip(&lines).any(|(a, b)| a != *b) {
        shell.set_history(ModelRc::new(VecModel::from(lines)));
    }
    shell.set_locked(app.locked);
    shell.set_encrypted(app.encrypted);
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
    // Closing the main window ends the app, even with the settings window open. While
    // an operation runs, ask first: quitting drops its SSH connection, which can leave a
    // remote apt/dpkg half-way.
    let c = ctx.clone();
    w.window().on_close_requested(move || {
        let busy = c.app.try_borrow().is_ok_and(|app| app.busy);
        if busy {
            c.update(|app| app.close_prompt = true);
            return slint::CloseRequestResponse::KeepWindowShown;
        }
        quit(&c);
        slint::CloseRequestResponse::HideWindow
    });
    let close = w.global::<CloseState>();
    let c = ctx.clone();
    close.on_keep_open(move || c.update(|app| app.close_prompt = false));
    let c = ctx.clone();
    close.on_quit(move || quit(&c));
    wire_chrome(w);
}

/// The window has no system frame; its own title bar moves, resizes and closes it.
fn wire_chrome(w: &AppWindow) {
    use slint::winit_030::{WinitWindowAccessor, winit::window::ResizeDirection};
    let chrome = w.global::<WindowChrome>();
    let main = w.as_weak();
    chrome.on_drag(move || {
        if let Some(main) = main.upgrade() {
            main.window().with_winit_window(|window| {
                let _ = window.drag_window();
            });
        }
    });
    let main = w.as_weak();
    chrome.on_resize(move |edge| {
        let direction = match edge {
            0 => ResizeDirection::North,
            1 => ResizeDirection::South,
            2 => ResizeDirection::West,
            3 => ResizeDirection::East,
            4 => ResizeDirection::NorthWest,
            5 => ResizeDirection::NorthEast,
            6 => ResizeDirection::SouthWest,
            _ => ResizeDirection::SouthEast,
        };
        if let Some(main) = main.upgrade() {
            main.window().with_winit_window(|window| {
                let _ = window.drag_resize_window(direction);
            });
        }
    });
    let main = w.as_weak();
    chrome.on_minimize(move || {
        if let Some(main) = main.upgrade() {
            main.window().set_minimized(true);
        }
    });
    let main = w.as_weak();
    chrome.on_toggle_maximize(move || {
        if let Some(main) = main.upgrade() {
            let window = main.window();
            window.set_maximized(!window.is_maximized());
        }
    });
    // The same request as the system's close, so a running operation still asks first.
    let main = w.as_weak();
    chrome.on_close(move || {
        if let Some(main) = main.upgrade() {
            main.window()
                .dispatch_event(slint::platform::WindowEvent::CloseRequested);
        }
    });
}

fn quit(ctx: &Ctx) {
    // Say goodbye to open shells rather than drop their links.
    if let Ok(app) = ctx.app.try_borrow() {
        app.worker.shells.close_all();
    }
    if let Some(settings) = ctx.settings.upgrade() {
        let _ = settings.hide();
    }
    let _ = slint::quit_event_loop();
}
