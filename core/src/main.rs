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
        let request = serde_json::from_str(&input)?;
        let store = msif_core::store::Store::new(msif_core::store::Store::default_path()?);
        msif_core::dispatch(request, &store, &|message| {
            emit(json!({"kind": "progress", "message": message}))
        })
        .await
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
