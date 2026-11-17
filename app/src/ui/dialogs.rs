//! Confirmations and short questions: upgrades, reboots, a failed batch host, host keys,
//! deleting a host, naming a group, and errors.

use super::{
    App, AppWindow, BatchFailureState, Ctx, DeleteState, ErrorState, GroupState, KeptBackDetails,
    KeptBackHost, KeptBackState, KeptBackWindow, RebootState, TrustState, UpgradeState,
    app::{KeptBack, UpgradeConfirm},
    format::{Kind, Line, manager_label},
    info_lines, sync_rows, texts,
};
use crate::i18n::Translator;
use misf_core::{
    audit::{Action, Outcome},
    batch,
    model::{Host, Package},
    package_manager,
};
use serde_json::json;
use slint::{ComponentHandle, SharedString};

impl App {
    pub fn confirm_upgrade(&mut self) {
        let Some(confirm) = self.upgrade_confirm.take() else {
            return;
        };
        if confirm.batch {
            self.send(json!({"action":"batch","operation":confirm.mode.id(),"ids":confirm.ids}));
        } else {
            self.selected = confirm.ids[0].clone();
            self.start_connection_action(confirm.mode.id());
        }
    }

    pub fn cancel_upgrade(&mut self) {
        self.upgrade_confirm = None;
        self.log(Action::UpgradeCancel, Outcome::Cancelled);
    }

    /// Ticks or unticks a host in the dist-upgrade report.
    pub fn toggle_kept_back(&mut self, index: usize) {
        if let Some(entry) = self.kept_back_report.get_mut(index) {
            entry.chosen = !entry.chosen;
        }
    }

    /// Runs the dist-upgrade on the ticked hosts; the report itself is the confirmation.
    pub fn run_kept_back(&mut self) {
        let ids: Vec<String> = (self.kept_back_report.iter())
            .filter(|k| k.chosen)
            .map(|k| k.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        self.kept_back_report.clear();
        let batch = ids.len() > 1;
        self.upgrade_confirm = Some(UpgradeConfirm {
            ids,
            batch,
            mode: package_manager::Mode::DistUpgrade,
        });
        self.confirm_upgrade();
    }

    pub fn cancel_kept_back(&mut self) {
        self.kept_back_report.clear();
        self.log(Action::UpgradeCancel, Outcome::Cancelled);
    }

    pub fn reboot_step(&mut self, next: bool) {
        let Some(confirm) = &mut self.reboot_confirm else {
            return;
        };
        if !next {
            confirm.final_step = false;
        } else if !confirm.final_step {
            if confirm.acknowledged {
                confirm.final_step = true;
            }
        } else {
            let id = confirm.id.clone();
            self.reboot_confirm = None;
            self.selected = id.clone();
            self.send(json!({"action":"reboot","id":id}));
        }
    }

    pub fn cancel_reboot(&mut self) {
        self.reboot_confirm = None;
        self.log(Action::RebootCancel, Outcome::Cancelled);
    }

    /// Answers the worker's question after a batch host failed.
    pub fn decide_batch(&mut self, decision: batch::Decision) {
        self.session_log
            .add("UI", &format!("batch_decision {decision:?}"), "");
        self.batch_failure = None;
        if self.worker.decisions.send(decision).is_err() {
            self.error = Some(self.t().get("status.worker_ended").into());
        }
    }

    pub fn accept_trust(&mut self) {
        if let Some(trust) = &self.trust {
            let request = json!({"action":"trust","id":trust.host_id,"fingerprint":trust.actual});
            self.send(request);
        }
    }

    pub fn cancel_trust(&mut self) {
        self.log(Action::TrustCancel, Outcome::Succeeded);
        self.trust = None;
        self.after_trust = None;
        self.set_status(self.t().get("trust.cancelled"));
    }
}

/// The targets of an upgrade confirmation and what it will run.
fn upgrade_targets(app: &App) -> (Vec<&Host>, Vec<&'static str>) {
    let Some(confirm) = &app.upgrade_confirm else {
        return (vec![], vec![]);
    };
    let targets: Vec<&Host> = confirm
        .ids
        .iter()
        .filter_map(|id| app.hosts.iter().find(|h| h.id == *id))
        .collect();
    // A batch shares one manager; root targets simply skip sudo.
    let commands = match targets.first() {
        Some(host) if !confirm.batch => package_manager::package_commands(host, confirm.mode),
        Some(host) => package_manager::Manager::from_id(host.manager())
            .map(|m| package_manager::manager_commands(m, false, confirm.mode))
            .unwrap_or_default(),
        None => vec![],
    };
    (targets, commands)
}

/// A package as the kept-back report lists it: name and version change.
fn package_line(p: &Package) -> String {
    if p.candidate_version.is_empty() {
        format!("    {}  {}", p.name, p.current_version)
    } else if p.current_version.is_empty() {
        format!("    {}  {}", p.name, p.candidate_version)
    } else {
        format!(
            "    {}  {} → {}",
            p.name, p.current_version, p.candidate_version
        )
    }
}

/// What the report says under a host's name: the simulation in one line.
fn kept_back_summary(t: Translator, entry: &KeptBack) -> String {
    match &entry.plan {
        Some(plan) if entry.has_plan() => t.format(
            "kept_back.summary",
            &[
                ("kept", entry.packages.len().to_string()),
                ("install", plan.install.len().to_string()),
                ("upgrade", plan.upgrade.len().to_string()),
                ("remove", plan.remove.len().to_string()),
            ],
        ),
        _ => t.format(
            "kept_back.no_plan",
            &[("kept", entry.packages.len().to_string())],
        ),
    }
}

/// The details window: per host, the kept-back packages, the simulated changes and the
/// simulation's own output.
fn kept_back_details(t: Translator, report: &[KeptBack]) -> Vec<Line> {
    let mut lines = vec![Line::new(Kind::Muted, t.get("kept_back.commands"))];
    let commands = package_manager::manager_commands(
        package_manager::Manager::Apt,
        false,
        package_manager::Mode::DistUpgrade,
    );
    lines.extend(commands.into_iter().map(|c| Line::new(Kind::Mono, c)));
    for entry in report {
        lines.push(Line::new(Kind::Gap, ""));
        lines.push(Line::new(Kind::Title, entry.name.clone()));
        lines.push(Line::new(
            Kind::Body,
            t.format(
                "kept_back.packages",
                &[
                    ("count", entry.packages.len().to_string()),
                    ("packages", entry.packages.join(", ")),
                ],
            ),
        ));
        match &entry.plan {
            Some(plan) if entry.has_plan() => {
                let count = |list: &[Package]| [("count", list.len().to_string())];
                for (title, list, kind) in [
                    (
                        t.format("kept_back.install", &count(&plan.install)),
                        &plan.install,
                        Kind::Body,
                    ),
                    (
                        t.format("kept_back.upgrade", &count(&plan.upgrade)),
                        &plan.upgrade,
                        Kind::Body,
                    ),
                    (
                        t.format("kept_back.remove", &count(&plan.remove)),
                        &plan.remove,
                        Kind::Error,
                    ),
                ] {
                    if list.is_empty() {
                        continue;
                    }
                    lines.push(Line::new(kind, title));
                    lines.extend(list.iter().map(|p| Line::new(Kind::Mono, package_line(p))));
                }
            }
            _ => lines.push(Line::new(Kind::Warning, t.get("kept_back.no_change"))),
        }
        lines.push(Line::new(Kind::Muted, t.get("kept_back.simulation")));
        lines.push(Line::new(
            Kind::Mono,
            if entry.simulation.is_empty() {
                t.get("kept_back.no_output").to_owned()
            } else {
                entry.simulation.clone()
            },
        ));
    }
    lines
}

/// Fills the details window from the open report and shows it.
pub fn show_kept_back_details(app: &App, details: &KeptBackWindow) {
    let t = app.t();
    let state = details.global::<KeptBackDetails>();
    state.set_heading(
        t.format(
            "kept_back.title",
            &[("count", app.kept_back_report.len().to_string())],
        )
        .into(),
    );
    state.set_lines(info_lines(kept_back_details(t, &app.kept_back_report)));
    let _ = details.show();
}

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    let upgrade = w.global::<UpgradeState>();
    upgrade.set_shown(app.upgrade_confirm.is_some());
    if let Some(confirm) = &app.upgrade_confirm {
        let (targets, commands) = upgrade_targets(app);
        let manager = targets
            .first()
            .map(|h| manager_label(t, h.manager()))
            .unwrap_or_default();
        let dist = confirm.mode == package_manager::Mode::DistUpgrade;
        let title = [
            ("count", targets.len().to_string()),
            ("manager", manager.into()),
        ];
        upgrade.set_title(
            if dist {
                t.format("upgrade.dist_title", &title)
            } else {
                t.format("upgrade.title", &title)
            }
            .into(),
        );
        upgrade.set_names(texts(targets.iter().map(|h| h.name.as_str())));
        upgrade.set_dist(dist);
        // The last simulation of each host, so removals are seen before confirming.
        let mut plans = Vec::new();
        for host in targets.iter().filter(|_| dist) {
            let Some(plan) = host.last_check.as_ref().and_then(|r| r.plan.as_ref()) else {
                continue;
            };
            plans.push(Line::new(
                Kind::Body,
                t.format(
                    "upgrade.dist_plan",
                    &[
                        ("name", host.name.clone()),
                        ("install", plan.install.len().to_string()),
                        ("upgrade", plan.upgrade.len().to_string()),
                        ("remove", plan.remove.len().to_string()),
                    ],
                ),
            ));
            if !plan.remove.is_empty() {
                let removed: Vec<&str> = plan.remove.iter().map(|p| p.name.as_str()).collect();
                plans.push(Line::new(
                    Kind::Error,
                    t.format("upgrade.dist_remove", &[("packages", removed.join(", "))]),
                ));
            }
        }
        upgrade.set_plans(info_lines(plans));
        upgrade.set_batch(confirm.batch);
        upgrade.set_can_confirm(!targets.is_empty() && commands.len() > 1);
        upgrade.set_commands(texts(commands));
    } else {
        upgrade.set_acknowledged(false);
    }

    let kept = w.global::<KeptBackState>();
    kept.set_shown(!app.kept_back_report.is_empty());
    if app.kept_back_report.is_empty() {
        kept.set_acknowledged(false);
    } else {
        kept.set_title(
            t.format(
                "kept_back.title",
                &[("count", app.kept_back_report.len().to_string())],
            )
            .into(),
        );
        let rows: Vec<KeptBackHost> = (app.kept_back_report.iter())
            .map(|entry| KeptBackHost {
                name: entry.name.clone().into(),
                summary: kept_back_summary(t, entry).into(),
                warn: !entry.has_plan()
                    || entry.plan.as_ref().is_some_and(|p| !p.remove.is_empty()),
                chosen: entry.chosen,
            })
            .collect();
        sync_rows(kept.get_hosts(), rows, |model| kept.set_hosts(model));
        let chosen = (app.kept_back_report.iter()).filter(|k| k.chosen).count();
        kept.set_can_run(chosen > 0);
        kept.set_run_label(
            t.format("kept_back.dist_upgrade", &[("count", chosen.to_string())])
                .into(),
        );
    }

    let reboot = w.global::<RebootState>();
    reboot.set_shown(app.reboot_confirm.is_some());
    match &app.reboot_confirm {
        Some(confirm) => {
            reboot.set_final_step(confirm.final_step);
            reboot.set_title(
                t.format("reboot.title", &[("name", confirm.name.clone())])
                    .into(),
            );
            reboot.set_question(
                t.format(
                    "reboot.final_question",
                    &[
                        ("name", confirm.name.clone()),
                        ("address", confirm.address.clone()),
                    ],
                )
                .into(),
            );
            reboot.set_command(confirm.command.into());
        }
        None => reboot.set_acknowledged(false),
    }

    let failure = w.global::<BatchFailureState>();
    failure.set_shown(app.batch_failure.is_some());
    if let Some(failed) = &app.batch_failure {
        let operation = if failed.upgrade {
            t.get("batch.operation_upgrade")
        } else {
            t.get("batch.operation_check")
        };
        failure.set_title(
            t.format(
                "batch_failure.title",
                &[
                    ("name", failed.name.clone()),
                    ("operation", operation.into()),
                ],
            )
            .into(),
        );
        failure.set_error(failed.error.clone().into());
    }

    let trust = w.global::<TrustState>();
    trust.set_shown(app.trust.is_some());
    if let Some(dialog) = &app.trust {
        trust.set_changed(!dialog.old.is_empty());
        trust.set_address(dialog.address.clone().into());
        trust.set_actual(dialog.actual.clone().into());
        trust.set_previous(dialog.old.clone().into());
    }

    let delete = w.global::<DeleteState>();
    delete.set_shown(app.delete_id.is_some());
    if let Some(id) = &app.delete_id {
        let name = app
            .hosts
            .iter()
            .find(|h| h.id == *id)
            .map(|h| h.name.as_str())
            .unwrap_or(t.get("host.generic"));
        delete.set_prompt(
            t.format("host.delete_prompt", &[("name", name.into())])
                .into(),
        );
    }

    let group = w.global::<GroupState>();
    let dialog = app.group_dialog.as_ref().filter(|_| !app.locked);
    group.set_shown(dialog.is_some());
    if let Some(dialog) = dialog {
        group.set_renaming(dialog.id.is_some());
        if std::mem::take(&mut app.reload.group) {
            group.set_name(dialog.name.clone().into());
        }
    }

    let error = w.global::<ErrorState>();
    error.set_shown(app.error.is_some());
    error.set_text(app.error.clone().unwrap_or_default().into());
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let upgrade = w.global::<UpgradeState>();
    let c = ctx.clone();
    upgrade.on_confirm(move || {
        c.update_with(|app, w| {
            if w.global::<UpgradeState>().get_acknowledged() {
                app.confirm_upgrade();
            }
        })
    });
    let c = ctx.clone();
    upgrade.on_cancel(move || c.update(App::cancel_upgrade));

    let kept = w.global::<KeptBackState>();
    let c = ctx.clone();
    kept.on_toggle(move |index| c.update(|app| app.toggle_kept_back(index as usize)));
    let c = ctx.clone();
    kept.on_run(move || {
        c.update_with(|app, w| {
            if w.global::<KeptBackState>().get_acknowledged() {
                app.run_kept_back();
            }
        })
    });
    let c = ctx.clone();
    kept.on_cancel(move || c.update(App::cancel_kept_back));
    let c = ctx.clone();
    kept.on_details(move || c.show_kept_back_details());

    let reboot = w.global::<RebootState>();
    let c = ctx.clone();
    reboot.on_next(move || {
        c.update_with(|app, w| {
            if let Some(confirm) = &mut app.reboot_confirm {
                confirm.acknowledged = w.global::<RebootState>().get_acknowledged();
            }
            app.reboot_step(true);
        })
    });
    let c = ctx.clone();
    reboot.on_confirm(move || c.update(|app| app.reboot_step(true)));
    let c = ctx.clone();
    reboot.on_back(move || c.update(|app| app.reboot_step(false)));
    let c = ctx.clone();
    reboot.on_cancel(move || c.update(App::cancel_reboot));

    let failure = w.global::<BatchFailureState>();
    let c = ctx.clone();
    failure.on_abort(move || c.update(|app| app.decide_batch(batch::Decision::Abort)));
    let c = ctx.clone();
    failure.on_resume(move || c.update(|app| app.decide_batch(batch::Decision::Continue)));

    let trust = w.global::<TrustState>();
    let c = ctx.clone();
    trust.on_accept(move || c.update(App::accept_trust));
    let c = ctx.clone();
    trust.on_cancel(move || c.update(App::cancel_trust));

    let delete = w.global::<DeleteState>();
    let c = ctx.clone();
    delete.on_confirm(move || {
        c.update(|app| {
            if let Some(id) = app.delete_id.clone() {
                app.send(json!({"action":"delete","id":id}));
            }
        })
    });
    let c = ctx.clone();
    delete.on_cancel(move || {
        c.update(|app| {
            app.log(Action::DeleteCancel, Outcome::Succeeded);
            app.delete_id = None;
        })
    });

    let group = w.global::<GroupState>();
    let c = ctx.clone();
    group.on_save(move || {
        c.update_with(|app, w| {
            let name: SharedString = w.global::<GroupState>().get_name();
            app.save_group(&name);
        })
    });
    let c = ctx.clone();
    group.on_cancel(move || c.update(|app| app.group_dialog = None));

    let error = w.global::<ErrorState>();
    let c = ctx.clone();
    error.on_close(move || {
        c.update(|app| {
            app.error = None;
            app.log(Action::ErrorDismiss, Outcome::Succeeded);
        })
    });
}
