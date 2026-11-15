//! The terminal tool window: each host's remote commands and output as an operation
//! runs, one read-only tab per host (the text is already redacted by the core), then
//! any interactive shells the user opened, see [`super::remote_shell`].

use super::{
    App, AppWindow, Clip, Ctx, ShellRow, TerminalLine, TerminalState, TerminalTab, ToolWindow,
    app::Tool,
    remote_shell::{self, ShellState, ShellTab},
};
use misf_core::{
    audit::{Action, Outcome},
    shell::Input,
};
use serde_json::json;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

/// A shell's size before the panel has reported one.
const SHELL_SIZE: (u16, u16) = (100, 24);

/// Lines kept per host; older ones are dropped in steps of [`TRIM`].
pub const MAX_LINES: usize = 10_000;
const TRIM: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabState {
    Waiting,
    Running,
    Succeeded,
    Failed,
    /// A batch stopped before this host started.
    Skipped,
}

impl TabState {
    fn index(self) -> i32 {
        match self {
            Self::Waiting => 0,
            Self::Running => 1,
            Self::Succeeded => 2,
            Self::Failed => 3,
            Self::Skipped => 4,
        }
    }
}

pub struct Line {
    pub text: String,
    pub command: bool,
}

pub struct Tab {
    pub host: String,
    pub name: String,
    pub state: TabState,
    pub lines: Vec<Line>,
}

#[derive(Default)]
pub struct Terminal {
    pub tabs: Vec<Tab>,
    /// Interactive shells, shown after the run's tabs.
    pub shells: Vec<ShellTab>,
    /// Into the run's tabs, then the shells.
    pub current: usize,
    /// The shell area's size in cells, as the panel last reported it.
    pub size: Option<(u16, u16)>,
    /// A shell's screen changed since the view last drew it.
    screen_dirty: bool,
    /// The shell tab the view's screen was drawn from.
    screen_synced: Option<usize>,
    /// The shell whose `shell_open` request is out.
    pending_open: Option<u64>,
    /// The request whose hosts the tabs show; its `Done` settles them.
    pub action: String,
    /// Bumped whenever shown lines change other than at the end, so the view rebuilds.
    generation: u64,
    /// The tab and generation the view's lines were built from.
    synced: Option<(usize, u64)>,
}

impl Terminal {
    /// Starts a new run on `hosts` (ID and name), replacing the previous one.
    /// Shells stay, after the new run's tabs.
    pub fn start(&mut self, action: &str, hosts: Vec<(String, String)>) {
        self.tabs = (hosts.into_iter())
            .map(|(host, name)| Tab {
                host,
                name,
                state: TabState::Waiting,
                lines: Vec::new(),
            })
            .collect();
        self.action = action.into();
        self.current = 0;
        self.generation += 1;
        self.screen_dirty = true;
    }

    fn tab(&mut self, host: &str) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.host == host)
    }

    /// Adds a command, or lines of output, to its host's tab.
    pub fn write(&mut self, host: &str, text: &str, command: bool) {
        let Some(tab) = self.tab(host) else {
            return;
        };
        if tab.state == TabState::Waiting {
            tab.state = TabState::Running;
        }
        for (index, line) in text.lines().enumerate() {
            // A command spanning lines continues like a shell's `>` prompt.
            let text = match (command, index) {
                (false, _) => line.to_owned(),
                (true, 0) => format!("$ {line}"),
                (true, _) => format!("> {line}"),
            };
            tab.lines.push(Line { text, command });
        }
        if tab.lines.len() > MAX_LINES {
            tab.lines.drain(..TRIM);
            self.generation += 1;
        }
    }

    /// Marks a host finished, with a closing note.
    pub fn finish(&mut self, host: &str, succeeded: bool, note: String) {
        if let Some(tab) = self.tab(host) {
            tab.state = if succeeded {
                TabState::Succeeded
            } else {
                TabState::Failed
            };
            tab.lines.push(Line {
                text: note,
                command: false,
            });
        }
    }

    /// Settles the tabs when the request ends: hosts never started were skipped, and a
    /// host still running ended with `note`, e.g. after a single-host result.
    pub fn end(&mut self, succeeded: bool, note: &str) {
        for tab in &mut self.tabs {
            match tab.state {
                TabState::Waiting if self.action == "batch" => tab.state = TabState::Skipped,
                TabState::Waiting | TabState::Running => {
                    tab.state = if succeeded {
                        TabState::Succeeded
                    } else {
                        TabState::Failed
                    };
                    tab.lines.push(Line {
                        text: note.into(),
                        command: false,
                    });
                }
                _ => {}
            }
        }
        self.action.clear();
    }

    /// The shell the current tab shows, if it is one.
    pub fn shell(&self) -> Option<&ShellTab> {
        self.shells.get(self.current.checked_sub(self.tabs.len())?)
    }

    fn shell_mut(&mut self) -> Option<&mut ShellTab> {
        let index = self.current.checked_sub(self.tabs.len())?;
        self.shells.get_mut(index)
    }

    fn shell_by_id(&mut self, id: u64) -> Option<&mut ShellTab> {
        self.shells.iter_mut().find(|shell| shell.id == id)
    }

    /// The shells' size in cells.
    pub fn size(&self) -> (u16, u16) {
        self.size.unwrap_or(SHELL_SIZE)
    }

    /// Adds a shell tab and shows it.
    pub fn add_shell(&mut self, shell: ShellTab) {
        self.shells.push(shell);
        self.current = self.tabs.len() + self.shells.len() - 1;
        self.screen_dirty = true;
    }

    /// Output for a shell's screen; true when the emulator failed on it, which leaves
    /// the shell failed with a blank screen.
    pub fn shell_output(&mut self, id: u64, data: &[u8]) -> bool {
        let Some(shell) = self.shell_by_id(id) else {
            return false;
        };
        if shell.state == ShellState::Failed {
            return false;
        }
        let parser = &mut shell.parser;
        let failed = catch_unwind(AssertUnwindSafe(|| parser.process(data))).is_err();
        if failed {
            shell.break_down();
        }
        self.screen_dirty = true;
        failed
    }

    /// Forgets every run and shell, e.g. when the vault locks.
    pub fn clear(&mut self) {
        self.tabs.clear();
        self.shells.clear();
        self.action.clear();
        self.current = 0;
        self.pending_open = None;
        self.generation += 1;
        self.screen_dirty = true;
    }

    /// Marks every shell not yet ended as failed with `note`, e.g. when the worker that
    /// ran them is gone.
    pub fn fail_shells(&mut self, note: &str) {
        for shell in &mut self.shells {
            if matches!(shell.state, ShellState::Opening | ShellState::Open) {
                shell.state = ShellState::Failed;
                shell.note = note.into();
            }
            shell.connected = false;
        }
        self.pending_open = None;
        self.screen_dirty = true;
    }

    /// A shell ended; returns its host when it was open, for the audit log.
    pub fn shell_ended(&mut self, id: u64, failed: bool, note: String) -> Option<String> {
        let shell = self.shell_by_id(id)?;
        let was_open = std::mem::replace(&mut shell.connected, false);
        shell.state = if failed {
            ShellState::Failed
        } else {
            ShellState::Closed
        };
        shell.note = note;
        let host = shell.host.clone();
        self.screen_dirty = true;
        was_open.then_some(host)
    }

    /// Drops an ended shell's tab.
    pub fn remove_shell(&mut self, index: usize) {
        if index < self.shells.len() {
            self.shells.remove(index);
            let last = (self.tabs.len() + self.shells.len()).saturating_sub(1);
            self.current = self.current.min(last);
            self.screen_dirty = true;
        }
    }

    /// The current tab's text, for copying.
    pub fn text(&self) -> String {
        if let Some(shell) = self.shell() {
            return shell.text();
        }
        (self.tabs.get(self.current).into_iter())
            .flat_map(|tab| &tab.lines)
            .map(|line| format!("{}\n", line.text))
            .collect()
    }
}

/// The hosts whose remote work a request shows in the terminal, if any.
pub fn hosts(request: &serde_json::Value) -> Vec<String> {
    match request["action"].as_str().unwrap_or_default() {
        "check" | "upgrade" | "dist_upgrade" | "reboot" => request["id"]
            .as_str()
            .map(str::to_owned)
            .into_iter()
            .collect(),
        "batch" => {
            let mut ids: Vec<String> = Vec::new();
            for id in (request["ids"].as_array().into_iter().flatten()).filter_map(|v| v.as_str()) {
                if !ids.iter().any(|known| known == id) {
                    ids.push(id.to_owned());
                }
            }
            ids
        }
        _ => vec![],
    }
}

fn view(line: &Line) -> TerminalLine {
    TerminalLine {
        text: line.text.as_str().into(),
        command: line.command,
    }
}

pub fn push(app: &mut App, w: &AppWindow) {
    w.global::<ToolWindow>().set_open(Tool::index(app.tool));
    let state = w.global::<TerminalState>();
    let t = app.t();
    let typing_allowed = !app.busy && !app.locked;
    let opener = app.current().map(|host| host.name.clone());
    let terminal = &mut app.terminal;
    state.set_shown(!terminal.tabs.is_empty() || !terminal.shells.is_empty());
    state.set_current(terminal.current as i32);
    state.set_can_open_shell(typing_allowed && opener.is_some());
    state.set_open_shell_hint(
        opener
            .map(|name| t.format("shell.open_hint", &[("name", name)]))
            .unwrap_or_else(|| t.get("shell.open_none").into())
            .into(),
    );
    let tabs: Vec<TerminalTab> = (terminal.tabs.iter())
        .map(|tab| TerminalTab {
            name: tab.name.as_str().into(),
            state: tab.state.index(),
            shell: false,
        })
        .chain(terminal.shells.iter().map(|shell| TerminalTab {
            name: shell.name.as_str().into(),
            state: match shell.state {
                ShellState::Opening => 0,
                ShellState::Open => 1,
                ShellState::Failed => 3,
                ShellState::Closed => 4,
            },
            shell: true,
        }))
        .collect();
    let shown = state.get_tabs();
    if shown.row_count() != tabs.len() || shown.iter().zip(&tabs).any(|(a, b)| a != *b) {
        state.set_tabs(ModelRc::new(VecModel::from(tabs)));
    }
    // One model for the panel's life: lines are appended, never the whole list replaced,
    // so the view keeps its scroll position.
    let model = state.get_lines();
    let lines = match model.as_any().downcast_ref::<VecModel<TerminalLine>>() {
        Some(_) => model,
        None => {
            let model = ModelRc::from(Rc::new(VecModel::<TerminalLine>::default()));
            state.set_lines(model.clone());
            model
        }
    };
    let lines = lines
        .as_any()
        .downcast_ref::<VecModel<TerminalLine>>()
        .expect("terminal lines are a VecModel");
    push_shell(terminal, &state, typing_allowed, t);
    let current = terminal.tabs.get(terminal.current);
    let key = (terminal.current, terminal.generation);
    if terminal.synced != Some(key) {
        lines.set_vec(current.map_or_else(Vec::new, |tab| tab.lines.iter().map(view).collect()));
        terminal.synced = Some(key);
    } else if let Some(tab) = current {
        for line in tab.lines.iter().skip(lines.row_count()) {
            lines.push(view(line));
        }
    }
}

/// The current shell's screen, cursor and whether keys reach it.
fn push_shell(
    terminal: &mut Terminal,
    state: &TerminalState,
    typing_allowed: bool,
    t: crate::i18n::Translator,
) {
    let current = terminal.current;
    let Some(shell) = terminal.shell() else {
        state.set_shell_view(false);
        terminal.screen_synced = None;
        return;
    };
    state.set_shell_view(true);
    let open = shell.state == ShellState::Open;
    state.set_shell_typing(open && typing_allowed);
    state.set_shell_notice(
        match shell.state {
            ShellState::Opening => t.get("shell.opening").into(),
            ShellState::Open if !typing_allowed => t.get("shell.paused").into(),
            ShellState::Open => String::new(),
            ShellState::Failed if shell.note.is_empty() => t.get("shell.emulator_failed").into(),
            ShellState::Closed | ShellState::Failed => shell.note.clone(),
        }
        .into(),
    );
    if terminal.screen_dirty || terminal.screen_synced != Some(current) {
        let screen = shell.parser.screen();
        let (row, col) = screen.cursor_position();
        // The emulator's state came from the remote side; a panic drawing it fails
        // this shell, not the app.
        let drawn = catch_unwind(AssertUnwindSafe(|| remote_shell::rows(screen)));
        let rows: Vec<ShellRow> = match drawn {
            Ok(rows) => {
                state.set_cursor_row(i32::from(row));
                state.set_cursor_col(i32::from(col));
                state.set_cursor_shown(open && !screen.hide_cursor() && screen.scrollback() == 0);
                rows
            }
            Err(_) => {
                let id = terminal.shell().map(|shell| shell.id);
                if let Some(shell) = id.and_then(|id| terminal.shell_by_id(id)) {
                    shell.break_down();
                }
                state.set_cursor_shown(false);
                Vec::new()
            }
        };
        state.set_screen(ModelRc::new(VecModel::from(rows)));
        terminal.screen_dirty = false;
        terminal.screen_synced = Some(current);
    }
}

impl App {
    /// Opens an interactive shell on the chosen host in a new terminal tab.
    pub fn open_shell(&mut self) {
        if self.busy || self.locked {
            return;
        }
        let Some(host) = self.current() else {
            return;
        };
        let (id, name) = (host.id.clone(), host.name.clone());
        let shell = self.worker.shells.reserve();
        let size = self.terminal.size();
        self.terminal
            .add_shell(ShellTab::new(shell, id.clone(), name, size));
        self.terminal.pending_open = Some(shell);
        self.tool = Some(Tool::Terminal);
        self.send(json!({
            "action": "shell_open",
            "id": id,
            "shell": shell,
            "cols": size.0,
            "rows": size.1,
        }));
    }

    /// The answer to a request to open a shell.
    pub fn shell_opened(&mut self, result: &Result<serde_json::Value, String>) {
        let t = self.t();
        let Some(id) = self.terminal.pending_open.take() else {
            return;
        };
        if let Some(shell) = self.terminal.shell_by_id(id)
            && shell.state == ShellState::Opening
        {
            match result {
                Ok(_) => {
                    shell.state = ShellState::Open;
                    shell.connected = true;
                }
                Err(error) => {
                    shell.state = ShellState::Failed;
                    shell.note = t.format("shell.failed", &[("error", error.clone())]);
                }
            }
            self.terminal.screen_dirty = true;
        }
    }

    /// A shell ended, by the user, the remote side or a lock; its tab stays to read.
    pub fn shell_ended(&mut self, id: u64, error: Option<String>) {
        let t = self.t();
        let note = match &error {
            Some(error) => t.format("shell.broken", &[("error", error.clone())]),
            None => t.get("shell.ended").into(),
        };
        if let Some(host) = self.terminal.shell_ended(id, error.is_some(), note) {
            let outcome = if error.is_some() {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            };
            let host = self.hosts.iter().find(|h| h.id == host);
            if self
                .audit
                .record(Action::ShellClose, outcome, host, None)
                .is_err()
            {
                self.error = Some(self.t().get("audit.failed").into());
            }
        }
    }

    /// Ends every open shell and logs each end now: the vault is locking or the app is
    /// closing, and their own end may never be heard.
    pub fn end_shells(&mut self) {
        let t = self.t();
        let ended: Vec<String> = (self.terminal.shells.iter_mut())
            .filter(|shell| shell.connected)
            .map(|shell| {
                shell.connected = false;
                if shell.state == ShellState::Open {
                    shell.state = ShellState::Closed;
                    shell.note = t.get("shell.ended").into();
                }
                shell.host.clone()
            })
            .collect();
        self.worker.shells.close_all();
        for id in ended {
            let host = self.hosts.iter().find(|h| h.id == id);
            if (self.audit)
                .record(Action::ShellClose, Outcome::Succeeded, host, None)
                .is_err()
            {
                self.error = Some(t.get("audit.failed").into());
            }
        }
    }

    /// Sends the current shell bytes typed into it.
    fn type_into_shell(&mut self, bytes: Vec<u8>) -> bool {
        if self.busy || self.locked {
            return false;
        }
        let Some(shell) = self.terminal.shell_mut() else {
            return false;
        };
        if shell.state != ShellState::Open {
            return false;
        }
        let id = shell.id;
        // Typing returns to the live screen.
        if shell.parser.screen().scrollback() > 0 {
            shell.parser.screen_mut().set_scrollback(0);
            self.terminal.screen_dirty = true;
        }
        self.worker.shells.send(id, Input::Data(bytes))
    }
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<TerminalState>();
    let c = ctx.clone();
    state.on_select(move |index| {
        c.update(|app| {
            if let Ok(index) = usize::try_from(index)
                && index < app.terminal.tabs.len() + app.terminal.shells.len()
            {
                app.terminal.current = index;
            }
        })
    });
    let c = ctx.clone();
    state.on_open_shell(move || c.update(App::open_shell));
    let c = ctx.clone();
    state.on_close_shell(move |index| {
        c.update(|app| {
            let Some(index) = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(app.terminal.tabs.len()))
            else {
                return;
            };
            let Some(shell) = app.terminal.shells.get(index) else {
                return;
            };
            match shell.state {
                // Ended in its own time: its tab goes.
                ShellState::Closed | ShellState::Failed => app.terminal.remove_shell(index),
                // Its end arrives as an event and marks the tab.
                ShellState::Open => {
                    app.worker.shells.send(shell.id, Input::Close);
                }
                // Still waiting for the worker; once nothing runs, the tab is stale.
                ShellState::Opening if !app.busy => app.terminal.remove_shell(index),
                ShellState::Opening => {}
            }
        })
    });
    let c = ctx.clone();
    state.on_shell_key(move |text, control, alt, shift| {
        let typed = {
            let Ok(mut app) = c.app.try_borrow_mut() else {
                return false;
            };
            let cursor_app = app
                .terminal
                .shell()
                .is_some_and(|shell| shell.parser.screen().application_cursor());
            let Some(bytes) = remote_shell::key_bytes(&text, control, alt, shift, cursor_app)
            else {
                return false;
            };
            app.type_into_shell(bytes)
        };
        c.update(|_| {});
        typed
    });
    let c = ctx.clone();
    state.on_shell_paste(move |text| {
        c.update(|app| {
            let bracketed = app
                .terminal
                .shell()
                .is_some_and(|shell| shell.parser.screen().bracketed_paste());
            app.type_into_shell(remote_shell::paste_bytes(&text, bracketed));
        })
    });
    let c = ctx.clone();
    state.on_shell_resized(move |cols, rows| {
        let (Ok(cols), Ok(rows)) = (u16::try_from(cols), u16::try_from(rows)) else {
            return;
        };
        let size = (cols.clamp(2, 1000), rows.clamp(2, 1000));
        c.update(|app| {
            if app.terminal.size == Some(size) {
                return;
            }
            app.terminal.size = Some(size);
            for shell in &mut app.terminal.shells {
                shell.resize(size);
                if shell.state == ShellState::Open {
                    app.worker.shells.send(
                        shell.id,
                        Input::Resize {
                            cols: u32::from(size.0),
                            rows: u32::from(size.1),
                        },
                    );
                }
            }
            app.terminal.screen_dirty = true;
        })
    });
    let c = ctx.clone();
    state.on_shell_scrolled(move |lines| {
        c.update(|app| {
            if let Some(shell) = app.terminal.shell_mut() {
                shell.scroll(lines);
                app.terminal.screen_dirty = true;
            }
        })
    });
    let c = ctx.clone();
    let main = w.as_weak();
    state.on_copy(move || {
        let text = c.app.borrow().terminal.text();
        if let Some(main) = main.upgrade() {
            let clip = main.global::<Clip>();
            clip.set_text(SharedString::from(text));
            clip.set_serial(clip.get_serial() + 1);
        }
    });

    // The stripe of tool buttons and the shared tool window height.
    let tools = w.global::<ToolWindow>();
    let height = ctx.app.borrow().preferences.tool_height;
    tools.set_body_height(height.unwrap_or(crate::preferences::TOOL_HEIGHT));
    let c = ctx.clone();
    tools.on_chosen(move |index| {
        c.update(|app| {
            if let Some(tool) = Tool::from_index(index) {
                app.toggle_tool(tool);
            } else {
                app.tool = None;
            }
        })
    });
    let c = ctx.clone();
    tools.on_resized(move |height| {
        c.update(|app| {
            app.preferences.tool_height = Some(height);
            let _ = app.save_preferences();
        })
    });
}
