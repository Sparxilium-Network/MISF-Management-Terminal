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
        let mut input = String::new();
        io::stdin()
            .take(8 * 1024 * 1024)
            .read_to_string(&mut input)?;
        let request: serde_json::Value = serde_json::from_str(&input)?;
        let mut session =
            misf_core::vault::StorageSession::new(misf_core::store::Store::default_path()?);
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
    .await;
    match result {
        Ok(data) => emit(json!({"kind": "result", "data": data})),
        Err(error) => {
            emit(json!({"kind": "error", "message": format!("{error:#}")}));
            std::process::exit(1);
        }
    }
}
