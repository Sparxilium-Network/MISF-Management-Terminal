//! The Slint window. [`App`] holds the state; each page's module turns it into the
//! properties of that page's Slint global and handles that page's callbacks.

mod app;
mod batch;
mod dialogs;
mod editor;
mod format;
mod host_page;
mod logging;
mod settings;
mod shell;
mod sidebar;
mod vault;

#[cfg(test)]
mod tests;

slint::include_modules!();

pub use app::App;

use crate::{i18n::Translator, worker::Worker};
use misf_core::{audit::AuditLog, runtime::RuntimePaths};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

/// Shared by every callback: the state and both windows.
#[derive(Clone)]
pub struct Ctx {
    app: Rc<RefCell<App>>,
    main: slint::Weak<AppWindow>,
    settings: slint::Weak<SettingsWindow>,
}

impl Ctx {
    /// Changes the state, then shows the result in both windows.
    pub fn update(&self, change: impl FnOnce(&mut App)) {
        self.update_with(|app, _| change(app));
    }

    /// Like [`Self::update`], with the main window to read typed text from.
    pub fn update_with(&self, change: impl FnOnce(&mut App, &AppWindow)) {
        // A nested call, for example from a native dialog's message loop, waits its turn.
        let Ok(mut app) = self.app.try_borrow_mut() else {
            return;
        };
        let (Some(main), Some(settings)) = (self.main.upgrade(), self.settings.upgrade()) else {
            return;
        };
        change(&mut app, &main);
        refresh(&mut app, &main, &settings);
    }

    /// The display language, for work done outside [`Self::update`].
    pub fn t(&self) -> Translator {
        self.app.borrow().t()
    }
}

thread_local! {
    /// The callbacks' context, for the worker's wake-ups on the UI thread.
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// Shows every part of the state in both windows.
pub fn refresh(app: &mut App, main: &AppWindow, settings: &SettingsWindow) {
    app.note_error();
    for language in [main.global::<T>(), settings.global::<T>()] {
        language.set_language(app.preferences.language.index());
    }
    let font = crate::preferences::InterfaceFont::ALL
        .iter()
        .position(|f| *f == app.preferences.font)
        .unwrap_or(0) as i32;
    for theme in [main.global::<Theme>(), settings.global::<Theme>()] {
        theme.set_appearance(app.preferences.appearance.index());
        theme.set_font(font);
    }
    shell::push(app, main);
    sidebar::push(app, main);
    host_page::push(app, main);
    batch::push(app, main);
    editor::push(app, main);
    vault::push(app, main);
    dialogs::push(app, main);
    settings::push(app, settings);
}

/// Creates both windows for `app`, connects their callbacks and shows the state.
pub fn build(
    app: App,
    debug: bool,
) -> Result<(Ctx, AppWindow, SettingsWindow), slint::PlatformError> {
    let main = AppWindow::new()?;
    let settings = SettingsWindow::new()?;
    main.set_debug(debug);
    let ctx = Ctx {
        app: Rc::new(RefCell::new(app)),
        main: main.as_weak(),
        settings: settings.as_weak(),
    };
    wire(&ctx, &main, &settings);
    ctx.update(|_| {});
    Ok((ctx, main, settings))
}

/// Opens the window and runs until it closes.
pub fn run(paths: RuntimePaths, audit: AuditLog) -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        // Skia draws text more sharply than femtovg's grayscale-only antialiasing.
        .renderer_name("skia".into())
        .select()?;
    let wake: crate::worker::Wake = Arc::new(|| {
        let _ = slint::invoke_from_event_loop(|| {
            CTX.with(|ctx| {
                if let Some(ctx) = ctx.borrow().as_ref() {
                    ctx.update(App::process_events);
                }
            })
        });
    });
    let worker = Worker::new(wake, paths.clone(), audit.clone());
    let debug = paths.debug;
    let (ctx, main, _settings) = build(App::new(paths, audit, worker), debug)?;
    let size = ctx.app.borrow().preferences.window;
    let [width, height] = size.map_or(crate::preferences::WindowSize::DEFAULT, |w| w.inner_size());
    main.window()
        .set_size(slint::LogicalSize::new(width, height));
    let maximized = size.is_some_and(|w| w.maximized);
    CTX.with(|cell| *cell.borrow_mut() = Some(ctx.clone()));
    main.show()?;
    if maximized {
        main.window().set_maximized(true);
    }
    #[cfg(feature = "capture")]
    capture(&ctx);
    slint::run_event_loop()?;
    CTX.with(|cell| cell.borrow_mut().take());
    ctx.app.borrow_mut().shutdown();
    Ok(())
}

fn wire(ctx: &Ctx, main: &AppWindow, settings: &SettingsWindow) {
    let lookup = |key: SharedString, language: i32| -> SharedString {
        Translator(crate::i18n::Language::from_index(language))
            .text(&key)
            .into()
    };
    main.global::<T>().on_lookup(lookup);
    settings.global::<T>().on_lookup(lookup);
    shell::wire(ctx, main);
    sidebar::wire(ctx, main);
    host_page::wire(ctx, main);
    batch::wire(ctx, main);
    editor::wire(ctx, main);
    vault::wire(ctx, main);
    dialogs::wire(ctx, main);
    settings::wire(ctx, settings);
    watch_input(ctx, main, settings);
}

/// Keeps `PopMenu.right-down` true only while the right button is held. The hook runs
/// before Slint sees the event, so the menu areas take the right press and no other.
/// Any other press, the pointer leaving or the window losing focus clears it, so a lost
/// release can never leave the menu areas swallowing left-clicks.
fn track_right_button(main: &AppWindow, event: &slint::winit_030::winit::event::WindowEvent) {
    use slint::winit_030::winit::event::{MouseButton, WindowEvent};
    let down = match event {
        WindowEvent::MouseInput { state, button, .. } => {
            *button == MouseButton::Right && state.is_pressed()
        }
        WindowEvent::CursorLeft { .. } | WindowEvent::Focused(false) => false,
        _ => return,
    };
    let menu = main.global::<PopMenu>();
    if menu.get_right_down() != down {
        menu.set_right_down(down);
    }
}

/// Restarts the auto-lock count on input to either window, tracks the main window's size,
/// and checks once a second whether the idle time ran out.
fn watch_input(ctx: &Ctx, main: &AppWindow, settings: &SettingsWindow) {
    use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
    let input = |event: &WindowEvent| {
        matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::CursorMoved { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::Touch(_)
                | WindowEvent::PinchGesture { .. }
                | WindowEvent::Ime(_)
        )
    };
    let c = ctx.clone();
    let weak = main.as_weak();
    main.window().on_winit_window_event(move |window, event| {
        if input(event) {
            activity(&c);
        }
        if let Some(main) = weak.upgrade() {
            track_right_button(&main, event);
        }
        if let WindowEvent::Resized(_) = event {
            let size = window.size().to_logical(window.scale_factor());
            let (maximized, minimized) = (window.is_maximized(), window.is_minimized());
            if let Ok(mut app) = c.app.try_borrow_mut() {
                app.track_window([size.width, size.height], maximized, minimized);
            }
        }
        EventResult::Propagate
    });
    let c = ctx.clone();
    settings
        .window()
        .on_winit_window_event(move |window, event| {
            if input(event) {
                activity(&c);
            }
            if let WindowEvent::Resized(_) = event
                && window.is_visible()
            {
                let size = window.size().to_logical(window.scale_factor());
                if let Ok(mut app) = c.app.try_borrow_mut() {
                    app.preferences.settings_window =
                        Some([size.width.round(), size.height.round()]);
                }
            }
            EventResult::Propagate
        });
    let c = ctx.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(1),
        move || idle_check(&c, false),
    );
    // The timer lives as long as the window.
    std::mem::forget(timer);
}

fn activity(ctx: &Ctx) {
    idle_check(ctx, true);
}

/// Feeds input or a tick to [`App::auto_lock`]; redraws only when it locked.
fn idle_check(ctx: &Ctx, active: bool) {
    let locked = {
        let Ok(mut app) = ctx.app.try_borrow_mut() else {
            return;
        };
        let busy = app.busy;
        app.auto_lock(Instant::now(), active);
        app.busy != busy
    };
    if locked {
        ctx.update(|_| {});
    }
}

/// Saves a screenshot of the main window and closes it, for documentation captures.
#[cfg(feature = "capture")]
fn capture(ctx: &Ctx) {
    let Some(path) = std::env::var_os("MISF_SCREENSHOT_TO") else {
        return;
    };
    let fixture = std::env::var("MISF_FIXTURE").as_deref() == Ok("1");
    if !fixture {
        ctx.update(open_requested);
    }
    let started = Instant::now();
    let mut injected = false;
    let c = ctx.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(100),
        move || {
            if fixture && !injected && !c.app.borrow().busy {
                injected = true;
                c.update(|app| {
                    fixture_hosts(app);
                    open_requested(app);
                });
                if std::env::var("MISF_OPEN_MENU").as_deref() == Ok("1")
                    && let Some(main) = c.main.upgrade()
                {
                    open_sample_menu(&c, &main);
                }
            }
            if started.elapsed().as_secs_f32() < 2.0 || c.app.borrow().busy {
                return;
            }
            let settings_open = c.app.borrow().settings_open;
            let snapshot = if settings_open {
                c.settings.upgrade().map(|w| w.window().take_snapshot())
            } else {
                c.main.upgrade().map(|w| w.window().take_snapshot())
            };
            if let Some(Ok(image)) = snapshot {
                image::save_buffer(
                    std::path::PathBuf::from(&path),
                    image.as_bytes(),
                    image.width(),
                    image.height(),
                    image::ColorType::Rgba8,
                )
                .expect("Failed to save UI screenshot");
            }
            let _ = slint::quit_event_loop();
        },
    );
    std::mem::forget(timer);
}

/// Opens the advanced menu and, over its first item, the copy menu a right-click shows.
#[cfg(feature = "capture")]
fn open_sample_menu(ctx: &Ctx, main: &AppWindow) {
    ctx.update(|app| app.advanced_open = true);
    let host = main.global::<HostState>();
    // The button sets these when clicked; here, its place in the 1280×820 capture.
    host.set_menu_x(616.0);
    host.set_menu_y(283.0);
    let line = MenuLine {
        title: ctx.t().get("hint.copy_commands").into(),
        enabled: true,
        action: "copy".into(),
        arg: host.get_upgrade_hint().copy,
    };
    main.global::<PopMenu>().invoke_open(
        ModelRc::new(VecModel::from(vec![line])),
        host.get_menu_x() + 60.0,
        host.get_menu_y() + 22.0,
    );
}

/// Opens what the capture's environment asks for.
#[cfg(feature = "capture")]
fn open_requested(app: &mut App) {
    match std::env::var("MISF_OPEN_SETTINGS").as_deref() {
        Ok(page @ ("1" | "about" | "preferences" | "security" | "batch")) => {
            app.settings_page = match page {
                "about" => app::SettingsPage::About,
                "preferences" => app::SettingsPage::Appearance,
                "security" => app::SettingsPage::Security,
                "batch" => app::SettingsPage::Batch,
                _ => app::SettingsPage::Vault,
            };
            app.toggle_settings();
        }
        _ => {
            if std::env::var("MISF_OPEN_EDITOR").as_deref() == Ok("1") {
                app.open_editor(None);
            }
            if std::env::var("MISF_OPEN_VAULT_SETUP").as_deref() == Ok("1") {
                app.open_vault_dialog(app::VaultMode::Create);
            }
            if std::env::var("MISF_OPEN_PASSKEY").as_deref() == Ok("1") {
                app.open_vault_dialog(app::VaultMode::Passkey);
            }
            if std::env::var("MISF_OPEN_KEYS").as_deref() == Ok("1") {
                app.open_key_dialog();
            }
            if std::env::var("MISF_OPEN_BATCH").as_deref() == Ok("1") {
                app.set_view(app::View::Batch);
            }
        }
    }
}

/// Sample hosts shown instead of a vault, so captures can show the host pages.
#[cfg(feature = "capture")]
fn fixture_hosts(app: &mut App) {
    use misf_core::model::{CheckResult, Host, HostGroup, Package};
    app.vault_dialog = None;
    app.encrypted = true;
    app.locked = false;
    let package = |name: &str, from: &str, to: &str| Package {
        name: name.into(),
        current_version: from.into(),
        candidate_version: to.into(),
        architecture: "amd64".into(),
        repository: "stable".into(),
    };
    let host = |id: &str, name: &str, group: &str, manager: &str| Host {
        id: id.into(),
        name: name.into(),
        address: format!("10.0.0.{}", id.len() + 10),
        port: 22,
        username: "deploy".into(),
        os: "Linux".into(),
        auth_type: "password".into(),
        group_id: group.into(),
        fingerprint: "SHA256:fixture".into(),
        distro: if manager == "apt" {
            "Ubuntu 26.04"
        } else {
            "Rocky Linux 10"
        }
        .into(),
        package_manager: manager.into(),
        notes: "Sample host".into(),
        ..Default::default()
    };
    let mut web = host("web", "web-01", "g1", "apt");
    web.last_check = Some(CheckResult {
        checked_at: "2026-11-09T10:00:00+08:00".into(),
        operation: "check".into(),
        status: "success".into(),
        packages: vec![
            package("openssl", "3.0.16", "3.0.17"),
            package("curl", "8.5.0-2", "8.5.0-3"),
        ],
        output: "Reading package lists...".into(),
        ..Default::default()
    });
    app.hosts = vec![
        web,
        host("db", "db-01", "g1", "apt"),
        host("rocky", "rocky-01", "g2", "dnf"),
        host("misc", "lab", "", "apt"),
    ];
    app.groups = vec![
        HostGroup {
            id: "g1".into(),
            name: "Production".into(),
            collapsed: false,
        },
        HostGroup {
            id: "g2".into(),
            name: "Staging".into(),
            collapsed: false,
        },
    ];
    app.selected = "web".into();
    app.prune_batch();
    app.batch_selected.insert("web".into());
}

/// A list model for Slint from any strings.
pub fn texts<S: Into<SharedString>>(items: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
}

/// Puts `rows` into a list property, changing rows in place when the count is the same,
/// so a row being dragged or hovered is not rebuilt under the pointer.
pub fn sync_rows<R: Clone + PartialEq + 'static>(
    current: ModelRc<R>,
    rows: Vec<R>,
    set: impl FnOnce(ModelRc<R>),
) {
    if let Some(model) = current.as_any().downcast_ref::<VecModel<R>>() {
        if model.row_count() == rows.len() {
            for (i, row) in rows.into_iter().enumerate() {
                if model.row_data(i).as_ref() != Some(&row) {
                    model.set_row_data(i, row);
                }
            }
        } else {
            model.set_vec(rows);
        }
        return;
    }
    set(ModelRc::new(VecModel::from(rows)));
}

/// Before [`sync_rows`]: rows that already exist keep their inner list model, refilled in
/// place, so the elements drawn from it are not rebuilt either.
pub fn keep_inner<R, I>(
    current: &ModelRc<R>,
    rows: &mut [R],
    inner: impl Fn(&mut R) -> &mut ModelRc<I>,
) where
    R: Clone + 'static,
    I: Clone + PartialEq + 'static,
{
    for (i, row) in rows.iter_mut().enumerate() {
        let Some(mut old) = current.row_data(i) else {
            break;
        };
        let old_inner = inner(&mut old).clone();
        let fresh: Vec<I> = inner(row).iter().collect();
        sync_rows(old_inner.clone(), fresh, |_| {});
        if old_inner.as_any().is::<VecModel<I>>() {
            *inner(row) = old_inner;
        }
    }
}

/// A hover hint: what a control runs, what it does and what could go wrong.
pub fn hint(title: &str, commands: &[&str], effect: &str, risk: &str) -> HintData {
    HintData {
        title: title.into(),
        commands: texts(commands.iter().copied()),
        effect: effect.into(),
        risk: risk.into(),
        copy: if commands.is_empty() {
            SharedString::new()
        } else {
            format::copied_commands(commands).into()
        },
    }
}

/// Text panel lines for Slint.
pub fn info_lines(lines: Vec<format::Line>) -> ModelRc<InfoLine> {
    ModelRc::new(VecModel::from(
        lines
            .into_iter()
            .map(|line| InfoLine {
                kind: line.kind as i32,
                text: line.text.into(),
                value: line.value.into(),
            })
            .collect::<Vec<_>>(),
    ))
}
