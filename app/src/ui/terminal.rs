//! The terminal panel under the pages: each host's remote commands and output as they
//! run, one tab per host. Read-only; the text is already redacted by the core.

use super::{
    App, AppWindow, Clip, Ctx, TerminalLine, TerminalState, TerminalTab, ToolWindow, app::Tool,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

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
    pub current: usize,
    /// The request whose hosts the tabs show; its `Done` settles them.
    pub action: String,
    /// Bumped whenever shown lines change other than at the end, so the view rebuilds.
    generation: u64,
    /// The tab and generation the view's lines were built from.
    synced: Option<(usize, u64)>,
}

impl Terminal {
    /// Starts a new run on `hosts` (ID and name), replacing the previous one.
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

    /// The current tab's text, for copying.
    pub fn text(&self) -> String {
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
    let terminal = &mut app.terminal;
    state.set_shown(!terminal.tabs.is_empty());
    state.set_current(terminal.current as i32);
    let tabs: Vec<TerminalTab> = (terminal.tabs.iter())
        .map(|tab| TerminalTab {
            name: tab.name.as_str().into(),
            state: tab.state.index(),
        })
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

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<TerminalState>();
    let c = ctx.clone();
    state.on_select(move |index| {
        c.update(|app| {
            if let Ok(index) = usize::try_from(index)
                && index < app.terminal.tabs.len()
            {
                app.terminal.current = index;
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
