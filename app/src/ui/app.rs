//! The window's state and the requests it sends, apart from how Slint draws it.

use super::format::{manager_label, record_result};
use crate::{
    i18n::Translator,
    preferences::{Preferences, WindowSize},
    session_log::SessionLog,
    worker::{Event, Interrupted, Worker},
};
use misf_core::{
    audit::{Action, AuditLog, Outcome},
    model::{CheckResult, ConnectionTest, Host, HostGroup, UpgradePlan},
    package_manager,
    runtime::RuntimePaths,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
    time::{Duration, Instant},
};
use zeroize::Zeroize;

/// How many past results the status line keeps.
pub const HISTORY_LIMIT: usize = 200;

/// A past result or error on the status line, at the local time it happened.
pub struct HistoryEntry {
    pub time: String,
    pub text: String,
    pub error: bool,
}

/// The group name dialog: `id` is `None` to add a group, or the group being renamed.
#[derive(Default)]
pub struct GroupDialog {
    pub id: Option<String>,
    pub name: String,
}

pub struct Editor {
    pub host: Host,
    pub port: String,
    /// Key authentication through a stored key (`host.key_id`) rather than an inline one.
    pub saved_key: bool,
    pub show_secrets: bool,
    pub error: String,
}

/// A stored SSH key as the worker describes it: never the key material.
pub struct KeyRow {
    pub id: String,
    pub name: String,
    pub algorithm: String,
    pub fingerprint: String,
}

impl KeyRow {
    pub fn label(&self) -> String {
        format!("{} · {}", self.name, self.algorithm)
    }
}

#[derive(Default)]
pub struct KeyDialog {
    pub name: String,
    pub private_key: String,
    pub passphrase: String,
    pub show_secrets: bool,
    pub error: String,
    /// The key awaiting delete confirmation.
    pub delete: Option<String>,
}

impl Drop for KeyDialog {
    fn drop(&mut self) {
        self.private_key.zeroize();
        self.passphrase.zeroize();
    }
}

/// A page of the settings window, picked from its side tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsPage {
    Vault,
    /// Preferences > Appearance and host list.
    Appearance,
    /// Preferences > Security: unlock method and auto-lock.
    Security,
    /// Preferences > Batch operations: order, details and columns.
    Batch,
    Language,
    Log,
    About,
}

impl SettingsPage {
    /// In tree order; the index is the page number Slint uses.
    pub const ALL: [Self; 7] = [
        Self::Vault,
        Self::Appearance,
        Self::Security,
        Self::Batch,
        Self::Language,
        Self::Log,
        Self::About,
    ];
    /// The pages under the Preferences branch of the settings tree.
    pub const PREFERENCES: [Self; 3] = [Self::Appearance, Self::Security, Self::Batch];

    pub fn index(self) -> i32 {
        Self::ALL.iter().position(|page| *page == self).unwrap_or(0) as i32
    }

    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Host,
    Batch,
}

/// An upgrade awaiting explicit confirmation; `batch` sends one request for all hosts.
pub struct UpgradeConfirm {
    pub ids: Vec<String>,
    pub batch: bool,
    /// [`package_manager::Mode::Upgrade`] or [`package_manager::Mode::DistUpgrade`].
    pub mode: package_manager::Mode,
}

/// A host whose upgrade kept packages back, for the report shown when an operation ends.
pub struct KeptBack {
    pub id: String,
    pub name: String,
    pub packages: Vec<String>,
    pub plan: Option<UpgradePlan>,
    /// The `apt-get -s dist-upgrade` output, for the details window.
    pub simulation: String,
    /// Ticked for the dist-upgrade the report offers.
    pub chosen: bool,
}

impl KeptBack {
    /// Whether the simulation would change anything; without that a dist-upgrade does
    /// nothing for these packages either, as with a phased rollout.
    pub fn has_plan(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|p| !(p.install.is_empty() && p.upgrade.is_empty() && p.remove.is_empty()))
    }
}

/// A reboot awaiting two confirmations: the risks first, then a final yes.
pub struct RebootConfirm {
    pub id: String,
    pub name: String,
    pub address: String,
    pub command: &'static str,
    pub acknowledged: bool,
    pub final_step: bool,
}

/// A running request the user may stop.
pub struct Stoppable {
    /// It upgrades: stopping may leave the remote apt/dpkg half-way, so ask first.
    pub upgrade: bool,
}

/// What the stop dialog asks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopPrompt {
    /// Stop the running upgrade?
    Stop,
    /// Lock now by stopping the running request, or once it ends?
    Lock,
}

/// A failed batch host the worker is waiting on: stop the batch or start the rest.
pub struct BatchFailure {
    pub name: String,
    pub upgrade: bool,
    pub error: String,
}

/// A tool window under the pages, like an IDE's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Terminal,
    History,
    /// The chosen host's packages, upgrades, restart needs, output and connection.
    Packages,
}

impl Tool {
    /// Its index in the Slint `ToolWindow.open`; 0 is none.
    pub fn index(tool: Option<Self>) -> i32 {
        match tool {
            None => 0,
            Some(Self::Terminal) => 1,
            Some(Self::History) => 2,
            Some(Self::Packages) => 3,
        }
    }

    pub fn from_index(index: i32) -> Option<Self> {
        match index {
            1 => Some(Self::Terminal),
            2 => Some(Self::History),
            3 => Some(Self::Packages),
            _ => None,
        }
    }
}

pub struct TrustDialog {
    pub host_id: String,
    pub address: String,
    pub old: String,
    pub actual: String,
    pub next: Option<String>,
}

pub struct SlotRow {
    pub index: usize,
    pub kind: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VaultMode {
    Create,
    Unlock,
    Manage,
    AddKey,
    Passkey,
    Revoke(usize),
    Rename(usize),
}

pub struct VaultDialog {
    pub mode: VaultMode,
    /// Slot name for a new or renamed slot; not a secret.
    pub name: String,
    pub password: String,
    pub confirmation: String,
    pub error: String,
    pub acknowledged: bool,
}

impl Drop for VaultDialog {
    fn drop(&mut self) {
        self.password.zeroize();
        self.confirmation.zeroize();
    }
}

/// Text inputs whose content the app changed, so the window must be given it again.
/// Otherwise Slint keeps what the user typed and the app reads it when it is needed.
#[derive(Default)]
pub struct Reload {
    pub editor: bool,
    pub keys: bool,
    pub vault: bool,
    pub group: bool,
    pub search: bool,
    pub package_search: bool,
}

/// Events [`App::process_events`] handles before letting the window draw.
const EVENT_BUDGET: usize = 512;
/// Shell output counted as one more event against [`EVENT_BUDGET`].
const SHELL_BYTES_PER_EVENT: usize = 4096;

pub struct App {
    pub audit: AuditLog,
    pub settings_path: PathBuf,
    /// False when ui-settings.json could not be read, so closing never overwrites it.
    pub window_loaded: bool,
    /// Main and settings window sizes as loaded, so closing saves only a change.
    pub saved_window: (Option<WindowSize>, Option<[f32; 2]>),
    pub debug: bool,
    pub passkey_allowed: bool,
    /// The preferred unlock method starts by itself only once per launch.
    pub default_unlock_started: bool,
    pub preferences: Preferences,
    pub settings_open: bool,
    pub settings_page: SettingsPage,
    /// Whether the Preferences branch of the settings tree is expanded.
    pub preferences_expanded: bool,
    pub worker: Worker,
    pub hosts: Vec<Host>,
    pub groups: Vec<HostGroup>,
    pub group_dialog: Option<GroupDialog>,
    pub selected: String,
    pub search: String,
    pub package_search: String,
    pub path: String,
    pub busy: bool,
    /// The status line. Progress goes straight here; results go through [`Self::set_status`]
    /// so they are also kept in [`Self::history`].
    pub status: String,
    /// Past results and errors, newest last; memory only and cleared when the vault locks,
    /// since they name hosts.
    pub history: VecDeque<HistoryEntry>,
    /// Whether the status line is expanded to show [`Self::history`].
    /// The tool window open under the pages, chosen from the stripe on the left.
    pub tool: Option<Tool>,
    pub error: Option<String>,
    pub editor: Option<Editor>,
    pub delete_id: Option<String>,
    pub trust: Option<TrustDialog>,
    pub after_trust: Option<String>,
    /// The result tab: 0 packages, 1 upgraded, 2 output, 3 connection, 4 restart.
    pub tab: usize,
    pub view: View,
    pub batch_selected: BTreeSet<String>,
    /// Whether the chosen host's packages are open in a window over the batch table.
    pub batch_popup: bool,
    /// Whether the host page's advanced menu is open.
    pub advanced_open: bool,
    /// Whether the host page's more menu (fingerprint, reload, edit, delete) is open.
    pub more_open: bool,
    /// The last user input, or the last moment auto-lock could not apply; idle time counts
    /// from here.
    pub last_activity: Instant,
    /// The one package manager the batch selection may contain.
    pub batch_manager: String,
    pub upgrade_confirm: Option<UpgradeConfirm>,
    pub batch_failure: Option<BatchFailure>,
    pub reboot_confirm: Option<RebootConfirm>,
    /// Hosts that need a dist-upgrade, reported once the operation that found them ends.
    pub kept_back_report: Vec<KeptBack>,
    /// This run's full log; memory only, gone when the app closes.
    pub session_log: SessionLog,
    /// The error last copied into [`Self::session_log`], so each is logged once.
    pub logged_error: Option<String>,
    pub encrypted: bool,
    /// "sqlite" or "json" from the last host list; empty while locked.
    pub storage_format: String,
    pub schema_version: Option<u64>,
    pub locked: bool,
    pub plaintext_exists: bool,
    pub plaintext_path: String,
    pub plaintext_delete_prompt: bool,
    /// The window's close button was pressed while an operation was running or a shell
    /// was open.
    pub close_prompt: bool,
    /// The running request, when the user may stop it.
    pub stoppable: Option<Stoppable>,
    /// A stop was asked for and the worker has not answered yet.
    pub stopping: bool,
    /// The stop question being asked, if any.
    pub stop_prompt: Option<StopPrompt>,
    /// Lock the vault as soon as the running request ends.
    pub lock_after: bool,
    /// [`Self::process_events`] stopped with events still waiting, to let the window
    /// draw and take input; the caller runs it again soon.
    pub events_pending: bool,
    /// Live remote output of the last check, upgrade or reboot, one tab per host.
    pub terminal: super::terminal::Terminal,
    /// The older schema version of a just-decrypted vault, waiting for upgrade consent.
    pub schema_upgrade_prompt: Option<u64>,
    pub slots: Vec<SlotRow>,
    pub storage_warning: String,
    pub vault_dialog: Option<VaultDialog>,
    pub ssh_keys: Vec<KeyRow>,
    pub key_dialog: Option<KeyDialog>,
    /// A sidebar row being dragged.
    pub drag: Option<super::sidebar::Drag>,
    pub reload: Reload,
}

impl App {
    pub fn new(paths: RuntimePaths, audit: AuditLog, worker: Worker) -> Self {
        let settings_path = paths.store.with_file_name("ui-settings.json");
        let loaded = Preferences::load(&settings_path);
        let preferences = loaded.clone().unwrap_or_default();
        let window_loaded = loaded.is_ok();
        let mut app = Self {
            audit,
            settings_path,
            window_loaded,
            saved_window: (preferences.window, preferences.settings_window),
            debug: paths.debug,
            passkey_allowed: paths.passkey_allowed(),
            default_unlock_started: false,
            preferences,
            settings_open: false,
            settings_page: SettingsPage::Vault,
            preferences_expanded: true,
            worker,
            hosts: vec![],
            groups: vec![],
            group_dialog: None,
            selected: String::new(),
            search: String::new(),
            package_search: String::new(),
            path: String::new(),
            busy: false,
            status: String::new(),
            history: VecDeque::new(),
            tool: None,
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
            batch_popup: false,
            advanced_open: false,
            more_open: false,
            last_activity: Instant::now(),
            batch_manager: String::new(),
            upgrade_confirm: None,
            batch_failure: None,
            reboot_confirm: None,
            kept_back_report: Vec::new(),
            session_log: SessionLog::new(),
            logged_error: None,
            encrypted: false,
            storage_format: String::new(),
            schema_version: None,
            locked: false,
            plaintext_exists: false,
            plaintext_path: String::new(),
            plaintext_delete_prompt: false,
            close_prompt: false,
            stoppable: None,
            stopping: false,
            stop_prompt: None,
            lock_after: false,
            events_pending: false,
            terminal: Default::default(),
            schema_upgrade_prompt: None,
            slots: vec![],
            storage_warning: String::new(),
            vault_dialog: None,
            ssh_keys: vec![],
            key_dialog: None,
            drag: None,
            reload: Reload::default(),
        };
        app.send(json!({"action":"storage_status"}));
        app
    }

    pub fn t(&self) -> Translator {
        Translator(self.preferences.language)
    }

    pub fn current(&self) -> Option<&Host> {
        self.hosts.iter().find(|host| host.id == self.selected)
    }

    pub fn host_name(&self, id: &str) -> String {
        self.hosts
            .iter()
            .find(|h| h.id == id)
            .map_or_else(|| id.to_owned(), |h| h.name.clone())
    }

    pub fn log(&mut self, action: Action, outcome: Outcome) {
        self.log_at_slot(action, outcome, None);
    }

    pub fn log_at_slot(&mut self, action: Action, outcome: Outcome, slot: Option<usize>) {
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
        let name = |value: Value| value.as_str().unwrap_or("?").to_owned();
        let mut entry = format!(
            "{} {}",
            name(serde_json::to_value(action).unwrap_or_default()),
            name(serde_json::to_value(outcome).unwrap_or_default())
        );
        if let Some(host) = host {
            entry.push_str(&format!(" host={:?}", host.name));
        }
        if let Some(slot) = slot {
            entry.push_str(&format!(" slot={slot}"));
        }
        if self.audit.record(action, outcome, host, slot).is_err() {
            self.error = Some(self.t().get("audit.failed").into());
        }
        self.session_log.add("UI", &entry, "");
    }

    pub fn send(&mut self, request: Value) {
        self.log_operation(&request);
        self.session_log.request(&request);
        let t = self.t();
        self.busy = true;
        self.status = t.get("status.working").into();
        let action = request["action"].as_str().unwrap_or_default();
        self.stoppable = misf_core::cancel::CANCELLABLE
            .contains(&action)
            .then(|| Stoppable {
                upgrade: matches!(action, "upgrade" | "dist_upgrade")
                    || (action == "batch"
                        && matches!(
                            request["operation"].as_str(),
                            Some("upgrade" | "dist_upgrade")
                        )),
            });
        self.stopping = false;
        let hosts = super::terminal::hosts(&request);
        if !hosts.is_empty() {
            let hosts = (hosts.into_iter())
                .map(|id| {
                    let name = self.host_name(&id);
                    (id, name)
                })
                .collect();
            let action = request["action"].as_str().unwrap_or_default();
            self.terminal.start(action, hosts);
            self.tool = Some(Tool::Terminal);
        }
        if self.worker.requests.send(request).is_err() {
            self.busy = false;
            self.error = Some(t.get("status.worker_ended").into());
        }
    }

    /// The status line's stop button: an upgrade asks first, since stopping it may leave
    /// the remote apt/dpkg half-way.
    pub fn ask_stop(&mut self) {
        match &self.stoppable {
            Some(running) if running.upgrade => self.stop_prompt = Some(StopPrompt::Stop),
            Some(_) => self.stop(),
            None => {}
        }
    }

    /// Asks the worker to stop the running request. It drops the request's SSH
    /// connections; the answer arrives as the request's usual `Done`.
    pub fn stop(&mut self) {
        self.stop_prompt = None;
        if self.stoppable.is_none() || self.stopping {
            return;
        }
        self.stopping = true;
        self.worker.cancel.request();
        self.session_log.add("STOP", "stop requested", "");
        self.status = self.t().get("stop.stopping").into();
    }

    /// The title bar's lock. Idle, it locks at once. While a request the user may stop
    /// runs, ask whether to stop it first; any other request is short, so the lock
    /// waits for it.
    pub fn lock(&mut self) {
        if !self.busy {
            self.send(json!({"action":"vault_lock"}));
        } else if self.stoppable.is_some() && !self.stopping {
            self.stop_prompt = Some(StopPrompt::Lock);
        } else {
            self.lock_after();
        }
    }

    /// Locks once the running request ends.
    pub fn lock_after(&mut self) {
        self.stop_prompt = None;
        self.lock_after = true;
        self.status = self.t().get("stop.lock_after").into();
    }

    /// Opens `tool`, or closes it when it is already open.
    pub fn toggle_tool(&mut self, tool: Tool) {
        self.tool = if self.tool == Some(tool) {
            None
        } else {
            Some(tool)
        };
    }

    /// The worker panicked or ended mid-request. Nothing more will answer, so stop
    /// waiting. An interrupted upgrade may have partly run on the remote host: the worker
    /// already saved its hosts as unknown, and the error names them.
    pub fn worker_crashed(&mut self, detail: String, interrupted: Option<Interrupted>) {
        let t = self.t();
        self.busy = false;
        self.batch_failure = None;
        self.stoppable = None;
        self.stopping = false;
        self.stop_prompt = None;
        self.lock_after = false;
        self.status = t.get("status.failed").into();
        self.terminal
            .end(false, &format!("[{}]", t.get("terminal.interrupted")));
        // The worker's runtime ran the shells; they went with it.
        self.terminal.fail_shells(t.get("terminal.interrupted"));
        let detail = if detail.is_empty() {
            "-".to_owned()
        } else {
            detail
        };
        let mut error = t.format("worker.crashed", &[("detail", detail)]);
        if let Some(interrupted) = interrupted.filter(|i| !i.ids.is_empty()) {
            let names: Vec<String> = (interrupted.ids.iter())
                .map(|id| format!("- {}", self.host_name(id)))
                .collect();
            let mut result = interrupted.result;
            result.error = t.get("worker.upgrade_unknown").into();
            for host in (self.hosts.iter_mut()).filter(|h| interrupted.ids.contains(&h.id)) {
                record_result(host, result.clone());
            }
            error.push_str(
                "

",
            );
            error.push_str(&t.format(
                "worker.unknown_hosts",
                &[(
                    "hosts",
                    names.join(
                        "
",
                    ),
                )],
            ));
            if let Some(reason) = interrupted.unsaved {
                error.push_str(
                    "

",
                );
                error.push_str(&t.format("worker.unknown_unsaved", &[("error", reason)]));
            }
        }
        self.error = Some(error);
    }

    /// Saves the preferences, reporting a failure as an error.
    pub fn save_preferences(&mut self) -> Result<(), ()> {
        self.preferences.save(&self.settings_path).map_err(|error| {
            self.error = Some(self.t().format("settings.save_error", &[("error", error)]));
        })
    }

    /// Copies a new error into the session log and the history once.
    pub fn note_error(&mut self) {
        if self.error != self.logged_error {
            if let Some(error) = self.error.clone() {
                self.session_log.add("ERROR", &error, "");
                self.remember(error, true);
            }
            self.logged_error = self.error.clone();
        }
    }

    /// Shows a result on the status line and keeps it in the history.
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.remember(self.status.clone(), false);
    }

    /// Like [`Self::set_status`], naming the chosen host in the history.
    pub fn set_host_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        let line = match self.current() {
            Some(host) => self.t().format(
                "status.host_line",
                &[("name", host.name.clone()), ("text", self.status.clone())],
            ),
            None => self.status.clone(),
        };
        self.remember(line, false);
    }

    fn remember(&mut self, text: String, error: bool) {
        if self.history.len() == HISTORY_LIMIT {
            self.history.pop_front();
        }
        self.history.push_back(HistoryEntry {
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            text,
            error,
        });
    }

    pub fn process_events(&mut self) {
        let t = self.t();
        // Before reading: terminal lines sent after this wake the UI again.
        (self.worker.terminal).store(false, std::sync::atomic::Ordering::Release);
        // A flood of output (e.g. `yes` in a shell) is read a slice at a time, so the
        // window still draws and Ctrl+C still reaches the shell.
        let mut budget = EVENT_BUDGET;
        self.events_pending = false;
        loop {
            if budget == 0 {
                self.events_pending = true;
                break;
            }
            budget -= 1;
            let event = match self.worker.events.try_recv() {
                Ok(event) => event,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // The thread is gone without a word, mid-request: stop waiting for it.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if self.busy {
                        self.worker_crashed(String::new(), None);
                    }
                    break;
                }
            };
            match event {
                Event::Crashed {
                    detail,
                    interrupted,
                } => self.worker_crashed(detail, interrupted),
                Event::Progress(message) => {
                    self.session_log.add("PROGRESS", &message, "");
                    self.status = t.progress(&message);
                }
                Event::Terminal {
                    host,
                    text,
                    command,
                } => self.terminal.write(&host, &text, command),
                Event::ShellOutput { shell, data } => {
                    budget = budget.saturating_sub(data.len() / SHELL_BYTES_PER_EVENT);
                    // Output keeps the vault from locking under a running program.
                    self.last_activity = Instant::now();
                    if self.terminal.shell_output(shell, &data) {
                        // The emulator failed on this output: stop the shell.
                        self.worker
                            .shells
                            .send(shell, misf_core::shell::Input::Close);
                    }
                    self.worker.shells.drawn(shell, data.len());
                }
                Event::ShellEnded { shell, error } => self.shell_ended(shell, error),
                Event::BatchHost { id, result } => {
                    let (succeeded, note) = terminal_note(t, &result.status, &result.error);
                    self.terminal.finish(&id, succeeded, note);
                    let name = self.host_name(&id);
                    let host = self.hosts.iter().find(|h| h.id == id);
                    self.session_log.host(t, &name, host, &result);
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == id) {
                        record_result(host, result);
                    }
                }
                Event::BatchFailed { id, result } => {
                    let name = self
                        .hosts
                        .iter()
                        .find(|h| h.id == id)
                        .map_or(id, |h| h.name.clone());
                    self.session_log.add(
                        "BATCH",
                        &format!("{name}: {}, waiting for a decision", result.status),
                        "",
                    );
                    self.batch_failure = Some(BatchFailure {
                        name,
                        upgrade: result.is_upgrade(),
                        error: result.error,
                    });
                }
                Event::Done { action, result } => {
                    self.busy = false;
                    self.batch_failure = None;
                    let stopped = std::mem::take(&mut self.stopping);
                    self.stoppable = None;
                    self.stop_prompt = None;
                    if action == self.terminal.action {
                        let (succeeded, note) = match &result {
                            // Hosts still running when stopped; finished ones keep theirs.
                            _ if stopped => (false, format!("[{}]", t.get("stop.terminal"))),
                            // A check or upgrade answers with its result, a reboot with
                            // the host's reply.
                            Ok(data) => terminal_note(
                                t,
                                data["status"].as_str().unwrap_or("success"),
                                data["error"].as_str().unwrap_or_default(),
                            ),
                            Err(error) => terminal_note(t, "failed", error),
                        };
                        self.terminal.end(succeeded, &note);
                    }
                    self.log_done(&action, &result);
                    if action == "shell_open" {
                        self.shell_opened(&result);
                    }
                    match result {
                        Ok(data) => {
                            self.received(&action, data);
                            if stopped && matches!(action.as_str(), "upgrade" | "dist_upgrade") {
                                self.set_host_status(t.get("stop.upgrade_stopped"));
                            }
                        }
                        // The user stopped it: say so on the status line, not as an error.
                        Err(error) if stopped => self.set_host_status(error),
                        Err(error) => {
                            self.status = t.get("status.failed").into();
                            self.error = Some(error);
                            // The sidebar showed this change before it was saved.
                            if action == "arrange" || action == "group_collapse" || action == "test"
                            {
                                self.send(json!({"action":"list"}));
                            }
                            if action == "vault_create" {
                                self.vault_dialog = None;
                                self.send(json!({"action":"storage_status"}));
                            }
                        }
                    }
                    // Waits for any request the answer itself sent, such as a reload.
                    if self.lock_after && !self.busy {
                        self.lock_after = false;
                        if self.encrypted && !self.locked {
                            self.send(json!({"action":"vault_lock"}));
                        }
                    }
                }
            }
        }
        self.note_error();
    }

    pub fn received(&mut self, action: &str, data: Value) {
        let t = self.t();
        match action {
            "storage_status" => {
                self.apply_storage(&data);
                if !self.encrypted {
                    // There is no unencrypted mode: a vault must exist before any host.
                    self.clear_host_cache();
                    if self.vault_dialog.is_none() {
                        self.open_vault_dialog(VaultMode::Create);
                    }
                    self.set_status(t.get("vault.setup_required"));
                } else if self.locked {
                    self.clear_host_cache();
                    self.open_vault_dialog(VaultMode::Unlock);
                    self.set_status(t.get("vault.locked"));
                    self.start_default_unlock();
                } else {
                    self.check_plaintext_copy();
                    self.send(json!({"action":"list"}));
                }
            }
            "vault_create" | "vault_unlock" | "vault_unlock_passkey" | "vault_upgrade" => {
                self.apply_storage(&data);
                self.vault_dialog = None;
                self.schema_upgrade_prompt = data["pending_upgrade"].as_u64();
                if self.schema_upgrade_prompt.is_some() {
                    return;
                }
                if action == "vault_upgrade" {
                    self.set_status(t.format(
                        "settings.converted",
                        &[("version", misf_core::sqlite::SCHEMA_VERSION.to_string())],
                    ));
                }
                self.check_plaintext_copy();
                self.send(json!({"action":"list"}));
            }
            "vault_delete_plaintext" => {
                self.apply_storage(&data);
                self.plaintext_delete_prompt = false;
                self.set_status(t.get("plaintext.deleted"));
                // A legacy hosts.json may remain after the plaintext database is gone.
                self.check_plaintext_copy();
            }
            "vault_upgrade_cancel" => {
                self.apply_storage(&data);
                self.schema_upgrade_prompt = None;
                self.open_vault_dialog(VaultMode::Unlock);
                self.set_status(t.get("vault.locked"));
            }
            "vault_lock" => {
                self.apply_storage(&data);
                self.schema_upgrade_prompt = None;
                self.clear_host_cache();
                self.vault_dialog = None;
                self.set_status(t.get("status.locked"));
            }
            "vault_add_key" | "vault_add_passkey" => {
                self.apply_storage(&data["storage"]);
                self.open_vault_dialog(VaultMode::Manage);
                self.set_status(
                    t.format("status.slot_added", &[("slot", data["slot"].to_string())]),
                );
            }
            "vault_revoke_key" => {
                self.apply_storage(&data["storage"]);
                self.open_vault_dialog(VaultMode::Manage);
                self.set_status(
                    t.format("status.slot_revoked", &[("slot", data["slot"].to_string())]),
                );
            }
            "vault_rename_key" => {
                self.apply_storage(&data["storage"]);
                self.open_vault_dialog(VaultMode::Manage);
                self.set_status(
                    t.format("status.slot_renamed", &[("slot", data["slot"].to_string())]),
                );
            }
            "convert_storage" => {
                self.set_status(t.format(
                    "settings.converted",
                    &[("version", data["schema_version"].to_string())],
                ));
                self.send(json!({"action":"list"}));
            }
            "vault_backup" => {
                self.set_status(t.format(
                    "status.backup_exported",
                    &[("path", data["path"].as_str().unwrap_or_default().into())],
                ));
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
                self.groups = serde_json::from_value(data["groups"].clone()).unwrap_or_default();
                self.ssh_keys = data["keys"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|key| {
                        let text = |field: &str| key[field].as_str().unwrap_or_default().to_owned();
                        KeyRow {
                            id: text("id"),
                            name: text("name"),
                            algorithm: text("algorithm"),
                            fingerprint: text("fingerprint"),
                        }
                    })
                    .collect();
                self.path = data["path"].as_str().unwrap_or_default().into();
                self.encrypted = data["encrypted"].as_bool().unwrap_or(false);
                self.storage_format = data["format"].as_str().unwrap_or_default().into();
                self.schema_version = data["schema_version"].as_u64();
                self.prune_batch();
                if !self.hosts.iter().any(|h| h.id == self.selected) {
                    self.selected = self.hosts.first().map(|h| h.id.clone()).unwrap_or_default();
                }
                self.set_status(t.format(
                    "status.hosts_loaded",
                    &[("count", self.hosts.len().to_string())],
                ));
            }
            "save" => {
                self.selected = data["id"].as_str().unwrap_or_default().into();
                self.close_editor();
                self.send(json!({"action":"list"}));
            }
            "delete" => {
                self.delete_id = None;
                self.selected.clear();
                self.send(json!({"action":"list"}));
            }
            "group_add" | "group_rename" | "group_delete" => {
                self.group_dialog = None;
                self.send(json!({"action":"list"}));
            }
            "arrange" | "group_collapse" => self.set_status(t.get("group.saved")),
            "key_add" => {
                if let Some(dialog) = &mut self.key_dialog {
                    dialog.name.clear();
                    dialog.private_key.zeroize();
                    dialog.passphrase.zeroize();
                    dialog.error.clear();
                    self.reload.keys = true;
                }
                let name = data["key"]["name"].as_str().unwrap_or_default().to_owned();
                self.send(json!({"action":"list"}));
                self.set_status(t.format("status.key_added", &[("name", name)]));
            }
            "key_delete" => {
                if let Some(dialog) = &mut self.key_dialog {
                    dialog.delete = None;
                }
                self.send(json!({"action":"list"}));
                self.set_status(t.get("status.key_deleted"));
            }
            "probe" => {
                if let Some(host) = self.current() {
                    let actual = data["fingerprint"].as_str().unwrap_or_default().to_owned();
                    if host.fingerprint == actual {
                        self.set_host_status(t.get("status.fingerprint_matches"));
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
                        self.set_host_status(t.get("status.check_fingerprint"));
                    }
                }
            }
            "trust" => {
                if let Some(trust) = self.trust.take() {
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == trust.host_id) {
                        host.fingerprint = trust.actual;
                    }
                    self.set_host_status(t.get("status.fingerprint_saved"));
                    if let Some(action) = trust.next {
                        self.send(json!({"action":action,"id":self.selected}));
                    }
                }
            }
            "test" => {
                let distro = data["distro"].as_str().unwrap_or_default();
                let manager = data["package_manager"].as_str().unwrap_or_default();
                let test = serde_json::from_value::<ConnectionTest>(data["test"].clone()).ok();
                if let Some(host) = self.hosts.iter_mut().find(|h| h.id == self.selected) {
                    host.distro = distro.into();
                    host.package_manager = manager.into();
                    host.last_test = test;
                }
                self.prune_batch();
                let os = if distro.is_empty() {
                    data["os"].as_str().unwrap_or_default()
                } else {
                    distro
                };
                self.set_host_status(t.format(
                    "status.connected",
                    &[
                        ("os", os.into()),
                        ("manager", manager_label(t, manager).into()),
                    ],
                ));
            }
            "reboot" => {
                let name = self.current().map(|h| h.name.clone()).unwrap_or_default();
                self.set_status(t.format("reboot.scheduled", &[("name", name)]));
            }
            "check" | "upgrade" | "dist_upgrade" => {
                match serde_json::from_value::<CheckResult>(data) {
                    Ok(result) => {
                        self.set_host_status(match (action, result.status == "success") {
                            ("check", true) => t.get("status.apt_done"),
                            ("check", false) => t.get("status.apt_failed"),
                            (_, true) => t.get("status.upgrade_done"),
                            (_, false) => t.get("status.upgrade_failed"),
                        });
                        if let Some(host) = self.hosts.iter_mut().find(|h| h.id == self.selected) {
                            record_result(host, result);
                        }
                        self.prune_batch();
                        self.report_kept_back(vec![self.selected.clone()]);
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            "batch" => {
                let (mut succeeded, mut failed) = (0, 0);
                let mut finished = vec![];
                for entry in data["results"].as_array().into_iter().flatten() {
                    finished.extend(entry["id"].as_str().map(str::to_owned));
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
                let skipped = data["skipped"].as_array().map_or(0, Vec::len);
                let cancelled = data["cancelled"].as_array().map_or(0, Vec::len);
                // A stopped upgrade is also among the results, saved as unknown.
                let stopped_upgrades = (data["results"].as_array().into_iter().flatten())
                    .filter(|entry| {
                        (data["cancelled"].as_array().into_iter().flatten())
                            .any(|id| *id == entry["id"])
                    })
                    .count();
                failed -= stopped_upgrades;
                let mut counts = vec![
                    ("succeeded", succeeded.to_string()),
                    ("failed", failed.to_string()),
                ];
                counts.push(("skipped", skipped.to_string()));
                self.set_status(if cancelled > 0 {
                    counts.push(("cancelled", cancelled.to_string()));
                    t.format("status.batch_stopped", &counts)
                } else if skipped == 0 {
                    t.format("status.batch_done", &counts)
                } else {
                    t.format("status.batch_aborted", &counts)
                });
                self.report_kept_back(finished);
            }
            _ => {}
        }
    }

    /// Runs `action` on the chosen host, first confirming its key if none is stored.
    pub fn start_connection_action(&mut self, action: &str) {
        if let Some(host) = self.current() {
            if host.fingerprint.is_empty() {
                self.after_trust = Some(action.into());
                self.send(json!({"action":"probe","id":self.selected}));
            } else {
                self.send(json!({"action":action,"id":self.selected}));
            }
        }
    }

    pub fn select_host(&mut self, id: &str) {
        self.selected = id.to_owned();
        self.log(Action::HostSelect, Outcome::Succeeded);
    }

    pub fn set_view(&mut self, view: View) {
        if self.view != view {
            self.view = view;
            self.log(Action::MainTab, Outcome::Succeeded);
        }
    }

    pub fn toggle_settings(&mut self) {
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

    pub fn clear_host_cache(&mut self) {
        // The terminal names hosts and holds what they printed, shells included.
        self.end_shells();
        self.terminal.clear();
        for host in &mut self.hosts {
            wipe_host(host);
        }
        self.hosts.clear();
        self.groups.clear();
        self.group_dialog = None;
        self.storage_format.clear();
        self.schema_version = None;
        self.selected.clear();
        self.search.zeroize();
        self.package_search.zeroize();
        self.reload.search = true;
        self.reload.package_search = true;
        self.close_editor();
        self.trust = None;
        self.delete_id = None;
        self.after_trust = None;
        self.batch_selected.clear();
        self.upgrade_confirm = None;
        self.reboot_confirm = None;
        self.kept_back_report.clear();
        self.batch_popup = false;
        self.advanced_open = false;
        self.more_open = false;
        self.drag = None;
        self.ssh_keys.clear();
        self.history.clear();
        self.key_dialog = None;
    }

    /// Locks an unlocked vault once nothing was typed or clicked for the chosen minutes.
    /// Running work restarts the count. Returns how long until the next check is due.
    pub fn auto_lock(&mut self, now: Instant, active: bool) -> Option<Duration> {
        let minutes = self.preferences.auto_lock_minutes?;
        if self.busy || self.locked || !self.encrypted {
            self.last_activity = now;
            return None;
        }
        let limit = Duration::from_secs(u64::from(minutes) * 60);
        // Checked before this input, so the first move after a long idle while minimized
        // still locks instead of restarting the count.
        let idle = now.saturating_duration_since(self.last_activity);
        if idle < limit {
            if active {
                self.last_activity = now;
                return Some(limit);
            }
            return Some(limit - idle);
        }
        self.last_activity = now;
        self.log(Action::VaultAutoLock, Outcome::Started);
        // Drop the hosts now, not when the worker answers, so nothing stays on screen.
        self.clear_host_cache();
        self.send(json!({"action":"vault_lock"}));
        None
    }

    /// Remembers the restored size and the maximized state, saved when the app closes.
    pub fn track_window(&mut self, size: [f32; 2], maximized: bool, minimized: bool) {
        if minimized {
            return;
        }
        let [width, height] = match (maximized, self.preferences.window) {
            // A maximized window keeps the size it returns to when restored.
            (true, Some(previous)) => previous.inner_size(),
            _ => size,
        };
        self.preferences.window = Some(WindowSize {
            width: width.max(WindowSize::MIN[0]),
            height: height.max(WindowSize::MIN[1]),
            maximized,
        });
    }

    /// Saves changed window sizes and records the exit; called once as the app closes.
    pub fn shutdown(&mut self) {
        // Settings that failed to load are left as they are rather than overwritten.
        let windows = (self.preferences.window, self.preferences.settings_window);
        if self.window_loaded && windows != self.saved_window {
            let _ = self.preferences.save(&self.settings_path);
        }
        let _ = self
            .audit
            .record(Action::AppExit, Outcome::Succeeded, None, None);
        self.clear_host_cache();
    }
}

pub fn wipe_host(host: &mut Host) {
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

/// Whether a host's run succeeded, and the line closing its terminal tab.
fn terminal_note(t: Translator, status: &str, error: &str) -> (bool, String) {
    match status {
        "success" => (true, format!("[{}]", t.get("terminal.succeeded"))),
        _ if error.is_empty() => (false, format!("[{}]", t.get("terminal.failed"))),
        _ => (
            false,
            format!(
                "[{}] {}",
                t.get("terminal.failed"),
                error.lines().next().unwrap_or("")
            ),
        ),
    }
}
