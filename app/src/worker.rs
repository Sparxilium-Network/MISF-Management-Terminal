use eframe::egui;
use misf_core::store::Store;
use misf_core::vault::StorageSession;
use serde_json::Value;
use std::sync::mpsc::{self, Receiver, Sender};

pub enum Event {
    Progress(String),
    Done {
        action: String,
        result: Result<Value, String>,
    },
}

pub struct Worker {
    pub requests: Sender<Value>,
    pub events: Receiver<Event>,
}

impl Worker {
    pub fn new(ctx: egui::Context) -> Self {
        let (requests, jobs) = mpsc::channel::<Value>();
        let (events, results) = mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build();
            let mut session = Store::default_path().map(StorageSession::new);
            while let Ok(request) = jobs.recv() {
                let action = request["action"].as_str().unwrap_or_default().to_string();
                let result = match (&runtime, &mut session) {
                    (Ok(runtime), Ok(session)) => runtime
                        .block_on(session.handle(request, &|message| {
                            let _ = events.send(Event::Progress(message.into()));
                            ctx.request_repaint();
                        }))
                        .map_err(|error| format!("{error:#}")),
                    (Err(error), _) => Err(format!("無法啟動網路核心：{error}")),
                    (_, Err(error)) => Err(format!("{error:#}")),
                };
                let _ = events.send(Event::Done { action, result });
                ctx.request_repaint();
            }
        });
        Self {
            requests,
            events: results,
        }
    }
}
