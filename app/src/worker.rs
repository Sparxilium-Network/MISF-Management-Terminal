use misf_core::{
    audit::AuditLog,
    batch::{self, Decision},
    cancel::Cancel,
    console::Console,
    model::CheckResult,
    runtime::RuntimePaths,
    session::Session,
    shell::{self, Shells},
};
use serde_json::Value;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};
use tokio::sync::{
    Mutex,
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};

pub enum Event {
    Progress(String),
    /// A remote command about to run on a host, or complete lines of its output; already
    /// redacted. These come in bursts, so they share one wake-up: see [`Worker::terminal`].
    Terminal {
        host: String,
        text: String,
        command: bool,
    },
    /// Raw output from an interactive shell, for its terminal emulator. Shares the
    /// terminal's wake-up.
    ShellOutput {
        shell: u64,
        data: Vec<u8>,
    },
    /// An interactive shell ended; `error` when the link broke rather than closed.
    ShellEnded {
        shell: u64,
        error: Option<String>,
    },
    /// One batch host finished; its result is already saved.
    BatchHost {
        id: String,
        result: CheckResult,
    },
    /// One batch host failed; the batch starts no new host until a [`Decision`] is sent.
    BatchFailed {
        id: String,
        result: CheckResult,
    },
    Done {
        action: String,
        result: Result<Value, String>,
    },
    /// The request panicked. The session's state may be inconsistent, so the worker
    /// stops after this; no [`Event::Done`] follows.
    Crashed {
        detail: String,
        /// Set when the request was an upgrade.
        interrupted: Option<Interrupted>,
    },
}

/// The hosts a crashed upgrade left without a reported outcome.
pub struct Interrupted {
    /// Each one is marked with `result`, in the vault when possible.
    pub ids: Vec<String>,
    pub result: CheckResult,
    /// Why the mark could not be saved, so a restart would lose it.
    pub unsaved: Option<String>,
}

/// What the running request reaches; kept without its secrets in case it panics.
struct Running {
    action: String,
    operation: String,
    id: String,
    ids: Vec<String>,
}

impl Running {
    fn of(request: &Value) -> Self {
        let text = |field: &str| request[field].as_str().unwrap_or_default().to_owned();
        Self {
            action: text("action"),
            operation: text("operation"),
            id: text("id"),
            ids: (request["ids"].as_array().into_iter().flatten())
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect(),
        }
    }

    /// The upgrade this request runs on remote hosts, if any.
    fn upgrade(&self) -> Option<&str> {
        let operation = if self.action == "batch" {
            self.operation.as_str()
        } else {
            self.action.as_str()
        };
        matches!(operation, "upgrade" | "dist_upgrade").then_some(operation)
    }

    /// Its hosts whose outcome was never reported: for a batch, those not yet saved
    /// one by one.
    fn unreported(&self, reported: &[String]) -> Vec<String> {
        if self.action == "batch" {
            (self.ids.iter())
                .filter(|id| !reported.contains(id))
                .cloned()
                .collect()
        } else {
            vec![self.id.clone()]
        }
    }
}

/// Asks the UI thread to read [`Worker::events`]; called after every event is sent.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Worker {
    pub requests: Sender<Value>,
    pub events: Receiver<Event>,
    /// Answers to [`Event::BatchFailed`], one per question.
    pub decisions: UnboundedSender<Decision>,
    /// Set when terminal output woke the UI; cleared by the UI before it reads
    /// [`Self::events`], so lines arriving later wake it again.
    pub terminal: Arc<AtomicBool>,
    /// Open interactive shells: typing goes straight to them, not through `requests`.
    pub shells: Shells,
    /// Stops the running request when it is one of [`misf_core::cancel::CANCELLABLE`];
    /// reset before each request.
    pub cancel: Cancel,
}

/// Forwards live remote output to the UI, waking it at most once per read.
struct Screen {
    events: Sender<Event>,
    wake: Wake,
    woken: Arc<AtomicBool>,
}

impl Screen {
    fn send(&self, host: &str, text: &str, command: bool) {
        let _ = self.events.send(Event::Terminal {
            host: host.into(),
            text: text.into(),
            command,
        });
        if !self.woken.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }
}

impl shell::Screen for Screen {
    fn output(&self, shell: u64, data: &[u8]) {
        let _ = self.events.send(Event::ShellOutput {
            shell,
            data: data.to_vec(),
        });
        if !self.woken.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }
    fn ended(&self, shell: u64, error: Option<String>) {
        let _ = self.events.send(Event::ShellEnded { shell, error });
        (self.wake)();
    }
}

impl Console for Screen {
    fn command(&self, host: &str, command: &str) {
        self.send(host, command, true);
    }
    fn output(&self, host: &str, text: &str) {
        self.send(host, text, false);
    }
}

/// Forwards batch progress to the UI and waits for its answer to a failure.
struct Relay {
    events: Sender<Event>,
    wake: Wake,
    decisions: Mutex<UnboundedReceiver<Decision>>,
    /// Batch hosts of the running request already saved and reported.
    reported: Reported,
}

type Reported = Arc<std::sync::Mutex<Vec<String>>>;

/// The list even if a panic poisoned its lock: it only ever grows by whole IDs.
fn reported(list: &Reported) -> std::sync::MutexGuard<'_, Vec<String>> {
    list.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Relay {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
        (self.wake)();
    }
}

impl batch::Observer for Relay {
    fn finished(&self, id: &str, result: &CheckResult) {
        reported(&self.reported).push(id.into());
        self.send(Event::BatchHost {
            id: id.into(),
            result: result.clone(),
        });
    }

    fn failed<'a>(&'a self, id: &str, result: &CheckResult) -> batch::Asking<'a> {
        let event = Event::BatchFailed {
            id: id.into(),
            result: result.clone(),
        };
        Box::pin(async move {
            let mut decisions = self.decisions.lock().await;
            // Only the answer to this question counts.
            while decisions.try_recv().is_ok() {}
            self.send(event);
            // A closed window cannot answer; start nothing more.
            decisions.recv().await.unwrap_or(Decision::Abort)
        })
    }
}

impl Worker {
    pub fn new(wake: Wake, paths: RuntimePaths, audit: AuditLog) -> Self {
        let (requests, jobs) = mpsc::channel::<Value>();
        let (events, results) = mpsc::channel();
        let (decisions, answers) = unbounded_channel();
        let terminal = Arc::new(AtomicBool::new(false));
        let woken = terminal.clone();
        let shells = Shells::default();
        let open = shells.clone();
        let cancel = Cancel::default();
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build();
            let mut session = Session::new(&paths, audit);
            let done = Reported::default();
            session.set_batch_observer(Box::new(Relay {
                events: events.clone(),
                wake: wake.clone(),
                decisions: Mutex::new(answers),
                reported: done.clone(),
            }));
            let screen = Arc::new(Screen {
                events: events.clone(),
                wake: wake.clone(),
                woken,
            });
            session.set_console(screen.clone());
            session.set_shells(open, screen);
            session.set_cancel(stop.clone());
            while let Ok(request) = jobs.recv() {
                // A stop meant for the request before this one.
                stop.reset();
                let action = request["action"].as_str().unwrap_or_default().to_string();
                let running = Running::of(&request);
                reported(&done).clear();
                let handled = catch_unwind(AssertUnwindSafe(|| match &runtime {
                    Ok(runtime) => runtime
                        .block_on(session.handle(request, &|message| {
                            let _ = events.send(Event::Progress(message.into()));
                            wake();
                        }))
                        .map_err(|error| format!("{error:#}")),
                    Err(error) => Err(format!("無法啟動網路核心：{error}")),
                }));
                match handled {
                    Ok(result) => {
                        let _ = events.send(Event::Done { action, result });
                        wake();
                    }
                    Err(panic) => {
                        let interrupted = running.upgrade().map(|operation| {
                            let ids = running.unreported(&reported(&done));
                            mark_interrupted(&session, ids, operation)
                        });
                        let _ = events.send(Event::Crashed {
                            detail: panic_message(panic.as_ref()),
                            interrupted,
                        });
                        wake();
                        // Ending the thread closes `requests`, so later sends fail.
                        break;
                    }
                }
            }
        });
        Self {
            requests,
            events: results,
            decisions,
            terminal,
            shells,
            cancel,
        }
    }
}

/// Saves the interrupted upgrades as unknown, so a restart still shows them.
fn mark_interrupted(session: &Session, ids: Vec<String>, operation: &str) -> Interrupted {
    let result = batch::interrupted(operation);
    let marks = ids.iter().map(|id| (id.clone(), result.clone())).collect();
    // The session just panicked; saving may panic too.
    let unsaved = match catch_unwind(AssertUnwindSafe(|| session.save_results(marks))) {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("{error:#}")),
        Err(panic) => Some(panic_message(panic.as_ref())),
    };
    Interrupted {
        ids,
        result,
        unsaved,
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use misf_core::model::Host;
    use serde_json::json;

    #[test]
    fn a_crashed_upgrade_saves_its_unreported_hosts_as_unknown() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: directory.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
        };
        let mut session = Session::new(&paths, AuditLog::new(directory.path()).unwrap());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let run = |session: &mut Session, request: Value| {
            runtime.block_on(session.handle(request, &|_| {}))
        };
        run(
            &mut session,
            json!({"action":"vault_create","passphrase":"test vault 2026"}),
        )
        .unwrap();
        for id in ["a", "b", "c"] {
            let host = Host {
                id: id.into(),
                name: id.into(),
                address: "127.0.0.1".into(),
                port: 22,
                username: "operator".into(),
                password: "secret".into(),
                auth_type: "password".into(),
                os: "Linux".into(),
                ..Default::default()
            };
            run(&mut session, json!({"action":"save","host":host})).unwrap();
        }

        // Only upgrades are marked; a batch leaves out the hosts already reported.
        let check = Running::of(&json!({"action":"batch","operation":"check","ids":["a"]}));
        assert!(check.upgrade().is_none());
        let running =
            Running::of(&json!({"action":"batch","operation":"dist_upgrade","ids":["a","b","c"]}));
        let ids = running.unreported(&["a".into()]);
        assert_eq!(ids, ["b", "c"]);
        let interrupted = mark_interrupted(&session, ids, running.upgrade().unwrap());
        assert!(interrupted.unsaved.is_none());

        let listed = run(&mut session, json!({"action":"list"})).unwrap();
        let status = |id: &str| {
            let host = listed["hosts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|h| h["id"] == id)
                .unwrap();
            (
                host["last_check"]["operation"].as_str().map(str::to_owned),
                host["last_check"]["status"].as_str().map(str::to_owned),
            )
        };
        assert_eq!(status("a"), (None, None));
        for id in ["b", "c"] {
            assert_eq!(
                status(id),
                (Some("dist_upgrade".into()), Some("failed".into()))
            );
        }

        // A single upgrade with the vault locked: the failure to save is reported.
        run(&mut session, json!({"action":"vault_lock"})).unwrap();
        let single = Running::of(&json!({"action":"upgrade","id":"a"}));
        let interrupted = mark_interrupted(&session, single.unreported(&[]), "upgrade");
        assert_eq!(interrupted.ids, ["a"]);
        assert!(interrupted.unsaved.is_some());
    }
}
