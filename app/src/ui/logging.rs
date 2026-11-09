//! The in-memory session log: what each request and result adds, and exporting it.

use super::{
    App, Ctx,
    format::{manager_label, operation_label},
};
use chrono::Local;
use misf_core::{
    audit::{Action, Outcome},
    model::CheckResult,
};
use serde_json::Value;

impl App {
    /// Copies a finished request into the session log: check and upgrade results as host
    /// blocks with their remote output, everything else in full with secrets masked.
    pub fn log_done(&mut self, action: &str, result: &Result<Value, String>) {
        let t = self.t();
        let data = match result {
            Ok(data) => data,
            // The message itself is logged once as an ERROR entry.
            Err(_) => {
                return self
                    .session_log
                    .add("RESULT", &format!("{action} failed"), "");
            }
        };
        match action {
            "check" | "upgrade" | "dist_upgrade" => {
                match serde_json::from_value::<CheckResult>(data.clone()) {
                    Ok(check) => {
                        let name = self.host_name(&self.selected);
                        let host = self.hosts.iter().find(|h| h.id == self.selected);
                        self.session_log.host(t, &name, host, &check);
                    }
                    Err(_) => self.session_log.response(action, data),
                }
            }
            "batch" => {
                let results = data["results"].as_array().map_or(&[][..], Vec::as_slice);
                let succeeded = results
                    .iter()
                    .filter(|entry| entry["result"]["status"] == "success")
                    .count();
                self.session_log.add(
                    "RESULT",
                    &format!(
                        "batch finished: succeeded={succeeded} failed={} skipped={}",
                        results.len() - succeeded,
                        data["skipped"]
                    ),
                    "",
                );
            }
            "reboot" => self.session_log.add(
                "RESULT",
                &format!("reboot id={:?}", self.selected),
                data["output"].as_str().unwrap_or_default(),
            ),
            _ => self.session_log.response(action, data),
        }
    }

    /// Opens a titled section in the session log for an operation on remote hosts.
    pub fn log_operation(&mut self, request: &Value) {
        let t = self.t();
        let action = request["action"].as_str().unwrap_or_default();
        let name = self.host_name(request["id"].as_str().unwrap_or_default());
        let operation = |id: &str| operation_label(t, id).to_owned();
        let title = match action {
            "batch" => t.format(
                "log.title_batch",
                &[
                    ("manager", manager_label(t, &self.batch_manager).to_owned()),
                    (
                        "operation",
                        operation(request["operation"].as_str().unwrap_or_default()),
                    ),
                    (
                        "count",
                        request["ids"].as_array().map_or(0, Vec::len).to_string(),
                    ),
                ],
            ),
            "check" | "upgrade" | "dist_upgrade" => {
                let manager = self
                    .hosts
                    .iter()
                    .find(|h| request["id"] == h.id.as_str())
                    .map_or("", |h| h.manager());
                t.format(
                    "log.title_single",
                    &[
                        ("manager", manager_label(t, manager).to_owned()),
                        ("operation", operation(action)),
                        ("name", name),
                    ],
                )
            }
            "test" => t.format("log.title_test", &[("name", name)]),
            "probe" => t.format("log.title_probe", &[("name", name)]),
            "trust" => t.format("log.title_trust", &[("name", name)]),
            "reboot" => t.format("log.title_reboot", &[("name", name)]),
            _ => return,
        };
        self.session_log.banner(&title);
    }
}

/// Saves the session log where the user picks, outside [`Ctx::update`] because the native
/// dialog blocks.
pub fn export_log(ctx: &Ctx) {
    ctx.update(|app| app.log(Action::LogExport, Outcome::Started));
    let t = ctx.t();
    let file_name = format!("misf-log-{}.txt", Local::now().format("%Y%m%d-%H%M%S"));
    let path = rfd::FileDialog::new()
        .set_title(t.get("log.export_title"))
        .set_file_name(&file_name)
        .add_filter("Text", &["txt", "log"])
        .save_file();
    ctx.update(|app| {
        let Some(path) = path else {
            return app.log(Action::LogExport, Outcome::Cancelled);
        };
        match app.session_log.export(&path) {
            Ok(()) => {
                app.log(Action::LogExport, Outcome::Succeeded);
                app.status = t.format("log.exported", &[("path", path.display().to_string())]);
            }
            Err(error) => {
                app.log(Action::LogExport, Outcome::Failed);
                app.error = Some(t.format("log.export_failed", &[("error", error.to_string())]));
            }
        }
    });
}
