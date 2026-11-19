//! The APT lock diagnosis dialog: what holds apt/dpkg on a host, and the fixes the user
//! may run after reading each one's command, effect and risk and confirming it.

use super::{App, AppWindow, AptFixView, AptLockState, Ctx, host_page::Advanced};
use crate::i18n::Translator;
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, VecModel};

/// The open diagnosis, or one being read.
pub struct AptLock {
    pub id: String,
    pub name: String,
    pub address: String,
    /// The core's report; `Null` until the first diagnosis arrives.
    pub diagnosis: Value,
    /// The fix being confirmed.
    pub step: Option<usize>,
    /// What the last fix did, shown above the new findings.
    pub note: String,
}

impl App {
    /// Reads the chosen host's APT state; the dialog opens when it arrives.
    pub fn diagnose_apt(&mut self) {
        let Some(host) = self.current().cloned() else {
            return;
        };
        let diagnosis = match self.apt_lock.take() {
            Some(lock) if lock.id == host.id => lock.diagnosis,
            _ => Value::Null,
        };
        self.apt_lock = Some(AptLock {
            id: host.id.clone(),
            name: host.name,
            address: host.address,
            diagnosis,
            step: None,
            note: String::new(),
        });
        self.send(json!({"action":"apt_diagnose","id":host.id}));
    }

    /// A diagnosis, or a fix's output with the diagnosis after it.
    pub fn received_apt(&mut self, action: &str, data: Value) {
        let t = self.t();
        let Some(lock) = &mut self.apt_lock else {
            return;
        };
        lock.step = None;
        if action == "apt_fix" {
            let output = data["output"].as_str().unwrap_or_default().trim();
            lock.note = if output.is_empty() {
                t.get("apt_lock.done").to_owned()
            } else {
                t.format("apt_lock.done_output", &[("output", output.to_owned())])
            };
            lock.diagnosis = data["diagnosis"].clone();
        } else {
            lock.diagnosis = data;
        }
        let free = lock.diagnosis["summary"]
            .as_array()
            .is_none_or(Vec::is_empty);
        let name = lock.name.clone();
        self.set_status(if free {
            t.format("apt_lock.status_free", &[("name", name)])
        } else {
            t.format("apt_lock.status_found", &[("name", name)])
        });
    }

    fn apt_fix(&self, index: usize) -> Option<&Value> {
        let lock = self.apt_lock.as_ref()?;
        lock.diagnosis["fixes"].as_array()?.get(index)
    }

    /// Opens a fix's confirmation step, or the reboot dialog for a reboot.
    fn choose_apt_fix(&mut self, index: usize) {
        let Some(fix) = self.apt_fix(index) else {
            return;
        };
        if fix["kind"] == "reboot" {
            let id = self.apt_lock.take().map(|lock| lock.id).unwrap_or_default();
            self.selected = id;
            self.choose_advanced(Advanced::Reboot);
            return;
        }
        if let Some(lock) = &mut self.apt_lock {
            lock.step = Some(index);
        }
    }

    /// Runs the confirmed fix; the core checks once more that it still applies.
    fn confirm_apt_fix(&mut self) {
        let Some(lock) = &self.apt_lock else {
            return;
        };
        let Some(index) = lock.step else {
            return;
        };
        let Some(fix) = self.apt_fix(index).cloned() else {
            return;
        };
        let mut request = json!({
            "action": "apt_fix",
            "id": lock.id,
            "fix": fix["kind"],
        });
        for field in ["job", "unit", "pids"] {
            if !fix[field].is_null() {
                request[field] = fix[field].clone();
            }
        }
        self.send(request);
    }
}

fn joined(value: &Value) -> String {
    (value.as_array().into_iter().flatten())
        .filter_map(Value::as_u64)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Each fix as the dialog shows it.
fn views(t: Translator, diagnosis: &Value) -> Vec<AptFixView> {
    (diagnosis["fixes"].as_array().into_iter().flatten())
        .filter_map(|fix| {
            let command = fix["command"].as_str().unwrap_or_default().into();
            Some(match fix["kind"].as_str()? {
                "cancel_job" => AptFixView {
                    title: t
                        .format(
                            "apt_fix.cancel_job_title",
                            &[
                                ("job", fix["job"].as_u64().unwrap_or_default().to_string()),
                                ("unit", fix["unit"].as_str().unwrap_or_default().to_owned()),
                            ],
                        )
                        .into(),
                    why: t.get("apt_fix.cancel_job_why").into(),
                    effect: t.get("apt_fix.cancel_job_effect").into(),
                    risk: t.get("apt_fix.cancel_job_risk").into(),
                    command,
                    reboot: false,
                },
                "end_leftover" => AptFixView {
                    title: t
                        .format(
                            "apt_fix.end_leftover_title",
                            &[("pids", joined(&fix["pids"]))],
                        )
                        .into(),
                    why: t.get("apt_fix.end_leftover_why").into(),
                    effect: t.get("apt_fix.end_leftover_effect").into(),
                    risk: t.get("apt_fix.end_leftover_risk").into(),
                    command,
                    reboot: false,
                },
                "configure" => AptFixView {
                    title: t.get("apt_fix.configure_title").into(),
                    why: t.get("apt_fix.configure_why").into(),
                    effect: t.get("apt_fix.configure_effect").into(),
                    risk: t.get("apt_fix.configure_risk").into(),
                    command,
                    reboot: false,
                },
                "reboot" => AptFixView {
                    title: t.get("apt_fix.reboot_title").into(),
                    why: t.get("apt_fix.reboot_why").into(),
                    effect: t.get("apt_fix.reboot_effect").into(),
                    risk: t.get("apt_fix.reboot_risk").into(),
                    command: Default::default(),
                    reboot: true,
                },
                _ => return None,
            })
        })
        .collect()
}

pub fn push(app: &App, w: &AppWindow) {
    let state = w.global::<AptLockState>();
    let Some(lock) = app
        .apt_lock
        .as_ref()
        .filter(|lock| !lock.diagnosis.is_null())
    else {
        state.set_shown(false);
        state.set_step(-1);
        state.set_acknowledged(false);
        return;
    };
    let t = app.t();
    state.set_shown(true);
    state.set_title(
        t.format("apt_lock.title", &[("name", lock.name.clone())])
            .into(),
    );
    let findings = (lock.diagnosis["summary"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    state.set_findings(findings.into());
    state.set_note(lock.note.clone().into());
    state.set_fixes(ModelRc::new(VecModel::from(views(t, &lock.diagnosis))));
    state.set_confirm_question(
        t.format(
            "apt_lock.confirm_question",
            &[
                ("name", lock.name.clone()),
                ("address", lock.address.clone()),
            ],
        )
        .into(),
    );
    let step = lock.step.map_or(-1, |step| step as i32);
    if state.get_step() != step {
        // Every confirmation starts unacknowledged.
        state.set_acknowledged(false);
    }
    state.set_step(step);
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<AptLockState>();
    let c = ctx.clone();
    state.on_choose(move |index| {
        c.update(|app| app.choose_apt_fix(usize::try_from(index).unwrap_or(usize::MAX)))
    });
    let c = ctx.clone();
    state.on_confirm(move || c.update(App::confirm_apt_fix));
    let c = ctx.clone();
    state.on_back(move || {
        c.update(|app| {
            if let Some(lock) = &mut app.apt_lock {
                lock.step = None;
            }
        })
    });
    let c = ctx.clone();
    state.on_refresh(move || {
        c.update(|app| {
            if let Some(lock) = &app.apt_lock {
                app.selected = lock.id.clone();
            }
            app.diagnose_apt();
        })
    });
    let c = ctx.clone();
    state.on_cancel(move || c.update(|app| app.apt_lock = None));
}
