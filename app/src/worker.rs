use eframe::egui;
use misf_core::{
    audit::AuditLog,
    batch::{self, Decision},
    model::CheckResult,
    runtime::RuntimePaths,
    session::Session,
};
use serde_json::Value;
use std::sync::mpsc::{self, Receiver, Sender};
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
}

pub struct Worker {
    pub requests: Sender<Value>,
    pub events: Receiver<Event>,
    /// Answers to [`Event::BatchFailed`], one per question.
    pub decisions: UnboundedSender<Decision>,
}

/// Forwards batch progress to the UI and waits for its answer to a failure.
struct Relay {
    events: Sender<Event>,
    ctx: egui::Context,
    decisions: Mutex<UnboundedReceiver<Decision>>,
}

impl Relay {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
        self.ctx.request_repaint();
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
    pub fn new(ctx: egui::Context, paths: RuntimePaths, audit: AuditLog) -> Self {
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
                ctx: ctx.clone(),
                decisions: Mutex::new(answers),
            }));
            while let Ok(request) = jobs.recv() {
                let action = request["action"].as_str().unwrap_or_default().to_string();
                let result = match &runtime {
                    Ok(runtime) => runtime
                        .block_on(session.handle(request, &|message| {
                            let _ = events.send(Event::Progress(message.into()));
                            ctx.request_repaint();
                        }))
                        .map_err(|error| format!("{error:#}")),
                    Err(error) => Err(format!("無法啟動網路核心：{error}")),
                };
                let _ = events.send(Event::Done { action, result });
                ctx.request_repaint();
            }
        });
        Self {
            requests,
            events: results,
            decisions,
        }
    }
}
