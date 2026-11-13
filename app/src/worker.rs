use misf_core::{
    audit::AuditLog,
    batch::{self, Decision},
    model::CheckResult,
    runtime::RuntimePaths,
    session::Session,
};
use serde_json::Value;
use std::sync::{
    Arc,
    mpsc::{self, Receiver, Sender},
};
use tokio::sync::{
    Mutex,
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};

pub enum Event {
    Progress(String),
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
    Crashed(String),
}

/// Asks the UI thread to read [`Worker::events`]; called after every event is sent.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Worker {
    pub requests: Sender<Value>,
    pub events: Receiver<Event>,
    /// Answers to [`Event::BatchFailed`], one per question.
    pub decisions: UnboundedSender<Decision>,
}

/// Forwards batch progress to the UI and waits for its answer to a failure.
struct Relay {
    events: Sender<Event>,
    wake: Wake,
    decisions: Mutex<UnboundedReceiver<Decision>>,
}

impl Relay {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
        (self.wake)();
    }
}

impl batch::Observer for Relay {
    fn finished(&self, id: &str, result: &CheckResult) {
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
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build();
            let mut session = Session::new(&paths, audit);
            session.set_batch_observer(Box::new(Relay {
                events: events.clone(),
                wake: wake.clone(),
                decisions: Mutex::new(answers),
            }));
            while let Ok(request) = jobs.recv() {
                let action = request["action"].as_str().unwrap_or_default().to_string();
                let handled =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &runtime {
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
                        let _ = events.send(Event::Crashed(panic_message(panic.as_ref())));
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
        }
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}
