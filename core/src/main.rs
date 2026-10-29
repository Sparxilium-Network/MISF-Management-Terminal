use misf_core::{
    audit::{Action, AuditLog, Outcome},
    runtime::RuntimePaths,
    session::Session,
};
use serde_json::json;
use std::io::{self, Read, Write};

fn emit(value: serde_json::Value) {
    let mut out = io::stdout().lock();
    let _ = serde_json::to_writer(&mut out, &value);
    let _ = writeln!(out);
    let _ = out.flush();
}

#[tokio::main]
async fn main() {
    let result = async {
        let paths = RuntimePaths::from_args(std::env::args_os().skip(1))?;
        let audit = AuditLog::new(paths.store.parent().unwrap())?;
        audit.record(Action::AppStart, Outcome::Succeeded, None, None)?;
        if paths.debug_allow_passkey {
            audit.record(Action::DebugPasskeyEnabled, Outcome::Succeeded, None, None)?;
        }
        let result = run(&paths, audit.clone()).await;
        audit.record(
            Action::AppExit,
            if result.is_ok() {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            },
            None,
            None,
        )?;
        result
    }
    .await;
    match result {
        Ok(data) => emit(json!({"kind": "result", "data": data})),
        Err(error) => {
            emit(json!({"kind": "error", "message": format!("{error:#}")}));
            std::process::exit(1);
        }
    }
}

async fn run(paths: &RuntimePaths, audit: AuditLog) -> anyhow::Result<serde_json::Value> {
    let mut input = zeroize::Zeroizing::new(String::new());
    io::stdin()
        .take(8 * 1024 * 1024)
        .read_to_string(&mut input)?;
    let request: serde_json::Value = serde_json::from_str(&input)?;
    let mut session = Session::new(paths, audit);
    let progress = |message: &str| emit(json!({"kind": "progress", "message": message}));
    if let serde_json::Value::Array(requests) = request {
        let mut results = vec![];
        for request in requests {
            results.push(session.handle(request, &progress).await?);
        }
        Ok(serde_json::Value::Array(results))
    } else {
        session.handle(request, &progress).await
    }
}
