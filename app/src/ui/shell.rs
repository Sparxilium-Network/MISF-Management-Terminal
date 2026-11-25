//! The title bar, the status line and the host/batch tabs.

use super::{
    App, AppWindow, CloseState, Ctx, HistoryLine, Shell, StopState, WindowChrome,
    app::{StopPrompt, VaultMode, View},
    format::{batch_managers, manager_label},
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    let shell = w.global::<Shell>();
    shell.set_busy(app.busy);
    shell.set_status(app.status.clone().into());
    // Nothing left to interrupt: the question no longer applies.
    let shells = app.worker.shells.count() > 0;
    if !app.busy && !shells {
        app.close_prompt = false;
    }
    let close = w.global::<CloseState>();
    close.set_shown(app.close_prompt);
    close.set_shells_only(!app.busy);
    shell.set_stoppable(app.stoppable.is_some() && !app.stopping);
    let stop = w.global::<StopState>();
    stop.set_shown(app.stop_prompt.is_some());
    stop.set_lock(app.stop_prompt == Some(StopPrompt::Lock));
    stop.set_upgrade(
        app.stoppable
            .as_ref()
            .is_some_and(|running| running.upgrade),
    );
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
    // One batch tab per package manager; with none yet, one tab explains why.
    app.prune_batch();
    let managers = batch_managers(&app.hosts);
    let mut tabs = vec![t.get("view.host").to_owned()];
    if managers.is_empty() {
        tabs.push(t.get("view.batch_none").to_owned());
    }
    for (id, _) in &managers {
        // Switching tabs clears the selection, so only the open tab has one.
        let count = if *id == app.batch_manager {
            app.batch_selected.len()
        } else {
            0
        };
        tabs.push(t.format(
            "view.batch",
            &[
                ("manager", manager_label(t, id).to_owned()),
                ("count", count.to_string()),
            ],
        ));
    }
    let current = match app.view {
        View::Host => 0,
        View::Batch => managers
            .iter()
            .position(|(id, _)| *id == app.batch_manager)
            .map_or(1, |i| i + 1),
    };
    let ids: Vec<i32> = (0..tabs.len() as i32).collect();
    let labels: Vec<SharedString> = tabs.into_iter().map(Into::into).collect();
    if shell.get_tabs().iter().ne(labels.iter().cloned()) {
        shell.set_tabs(ModelRc::new(VecModel::from(labels)));
        shell.set_tab_ids(ModelRc::new(VecModel::from(ids)));
    }
    shell.set_current_tab(current as i32);
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let shell = w.global::<Shell>();
    let c = ctx.clone();
    shell.on_settings(move || c.update(App::toggle_settings));
    let c = ctx.clone();
    shell.on_lock(move || c.update(App::lock));
    let c = ctx.clone();
    shell.on_stop(move || c.update(App::ask_stop));
    let stop = w.global::<StopState>();
    let c = ctx.clone();
    stop.on_stop(move || {
        c.update(|app| {
            // Asked by the lock: lock as soon as the stopped request answers.
            if app.stop_prompt == Some(StopPrompt::Lock) {
                app.lock_after = true;
            }
            app.stop();
        })
    });
    let c = ctx.clone();
    stop.on_wait(move || c.update(App::lock_after));
    let c = ctx.clone();
    stop.on_back(move || c.update(|app| app.stop_prompt = None));
    let c = ctx.clone();
    shell.on_unlock(move || c.update(|app| app.open_vault_dialog(VaultMode::Unlock)));
    let c = ctx.clone();
    shell.on_enable_vault(move || c.update(|app| app.open_vault_dialog(VaultMode::Create)));
    let c = ctx.clone();
    shell.on_manage_keys(move || c.update(App::open_key_dialog));
    let c = ctx.clone();
    shell.on_review_plaintext(move || c.update(App::check_plaintext_copy));
    let c = ctx.clone();
    shell.on_view_chosen(move |tab| {
        c.update(|app| {
            let Some(index) = usize::try_from(tab).ok().and_then(|t| t.checked_sub(1)) else {
                app.set_view(View::Host);
                return;
            };
            if let Some((id, _)) = batch_managers(&app.hosts).get(index) {
                let id = id.clone();
                app.choose_batch_manager(&id);
            }
            app.set_view(View::Batch);
        })
    });
    // Closing the main window ends the app, even with the settings window open. While
    // an operation runs or a shell is open, ask first: quitting drops their SSH
    // connections, which can leave a remote apt/dpkg half-way.
    let c = ctx.clone();
    w.window().on_close_requested(move || {
        let busy = (c.app.try_borrow()).is_ok_and(|app| app.busy || app.worker.shells.count() > 0);
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
    // Say goodbye to open shells rather than drop their links, and log their end.
    if let Ok(mut app) = ctx.app.try_borrow_mut() {
        app.end_shells();
    }
    if let Some(settings) = ctx.settings.upgrade() {
        let _ = settings.hide();
    }
    let _ = slint::quit_event_loop();
}
