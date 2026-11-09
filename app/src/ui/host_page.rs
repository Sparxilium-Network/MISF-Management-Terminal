//! The chosen host's page and the result tabs it shares with the batch view.

use super::{
    App, AppWindow, Ctx, HostState, ResultState,
    app::{KeptBack, RebootConfirm, UpgradeConfirm},
    format::{
        connection_lines, format_time, manager_label, restart_lines, restart_summary, status_label,
        system_label,
    },
    hint, info_lines, keep_inner, sync_rows, texts,
};
use misf_core::{
    audit::{Action, Outcome},
    model::Host,
    package_manager,
};
use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString};

/// An item of the host page's advanced menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Advanced {
    Upgrade,
    DistUpgrade,
    Reboot,
}

impl App {
    pub fn prompt_upgrade(&mut self, ids: Vec<String>, batch: bool, mode: package_manager::Mode) {
        self.upgrade_confirm = Some(UpgradeConfirm { ids, batch, mode });
        self.log(Action::UpgradePrompt, Outcome::Succeeded);
    }

    /// Remembers which of these hosts' upgrades kept packages back, to report them.
    pub fn report_kept_back(&mut self, ids: Vec<String>) {
        let report: Vec<KeptBack> = ids
            .iter()
            .filter_map(|id| {
                let host = self.hosts.iter().find(|h| h.id == *id)?;
                let check = host.last_check.as_ref()?;
                (check.is_upgrade() && !check.kept_back.is_empty()).then(|| KeptBack {
                    id: host.id.clone(),
                    name: host.name.clone(),
                    packages: check.kept_back.clone(),
                    plan: check.plan.clone(),
                })
            })
            .collect();
        if report.is_empty() {
            return;
        }
        let names: Vec<&str> = report.iter().map(|k| k.name.as_str()).collect();
        self.session_log.add(
            "REPORT",
            &format!("dist-upgrade needed on {}", names.join(", ")),
            "",
        );
        self.kept_back_report = report;
    }

    /// Whether the advanced menu's item can run on this host.
    pub fn advanced_enabled(host: &Host, item: Advanced) -> bool {
        match item {
            // An upgrade confirmation must show real commands, so it waits for detection.
            Advanced::Upgrade => package_manager::Manager::from_id(host.manager()).is_some(),
            // APT only: may install new packages and remove installed ones.
            Advanced::DistUpgrade => host.manager() == "apt",
            // Only APT hosts with a confirmed key; the command is fixed per account type.
            Advanced::Reboot => host.manager() == "apt" && !host.fingerprint.is_empty(),
        }
    }

    /// Runs an item chosen from the advanced menu, which then closes.
    pub fn choose_advanced(&mut self, item: Advanced) {
        let Some(host) = self.current().cloned() else {
            return;
        };
        if !Self::advanced_enabled(&host, item) {
            return;
        }
        self.advanced_open = false;
        match item {
            Advanced::Upgrade => {
                self.prompt_upgrade(vec![host.id], false, package_manager::Mode::Upgrade)
            }
            Advanced::DistUpgrade => {
                self.prompt_upgrade(vec![host.id], false, package_manager::Mode::DistUpgrade)
            }
            Advanced::Reboot => {
                self.reboot_confirm = Some(RebootConfirm {
                    command: misf_core::reboot::command(&host),
                    id: host.id,
                    name: host.name,
                    address: host.address,
                    acknowledged: false,
                    final_step: false,
                });
                self.log(Action::RebootPrompt, Outcome::Succeeded);
            }
        }
    }

    pub fn set_result_tab(&mut self, tab: usize) {
        if self.tab != tab {
            self.tab = tab;
            self.log(Action::ResultTab, Outcome::Succeeded);
        }
    }
}

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    let state = w.global::<HostState>();
    let host = app.current().cloned();
    if host.is_none() {
        app.advanced_open = false;
    }
    state.set_has_host(host.is_some());
    state.set_advanced_open(app.advanced_open);
    state.set_delete_hint(hint(
        t.get("hint.delete_title"),
        &[],
        t.get("hint.delete_effect"),
        t.get("hint.delete_risk"),
    ));
    push_results(app, w, host.as_ref());
    let Some(host) = host else {
        return;
    };
    state.set_name(host.name.clone().into());
    state.set_summary(
        format!(
            "{}@{}:{}  ·  {}  ·  {}",
            host.username,
            host.address,
            host.port,
            system_label(t, &host),
            if host.auth_type == "key" {
                "SSH Key"
            } else {
                t.get("auth.password_login")
            }
        )
        .into(),
    );
    state.set_test_hint(hint(
        t.get("hint.test_title"),
        &[package_manager::DETECT],
        t.get("hint.test_effect"),
        t.get("hint.test_risk"),
    ));
    state.set_probe_hint(hint(
        t.get("hint.probe_title"),
        &[],
        t.get("hint.probe_effect"),
        t.get("hint.probe_risk"),
    ));
    let detected = package_manager::Manager::from_id(host.manager());
    let manager = [("manager", manager_label(t, host.manager()).to_owned())];
    let (check_label, upgrade_label, check_effect) = match detected {
        Some(_) => (
            t.format("apt.check", &manager),
            t.format("apt.upgrade", &manager),
            t.get("hint.check_effect").to_owned(),
        ),
        None => (
            t.get("apt.check_detect").to_owned(),
            t.get("apt.upgrade_detect").to_owned(),
            format!(
                "{}\n{}",
                t.get("hint.detect_note"),
                t.get("hint.check_effect")
            ),
        ),
    };
    state.set_check_label(check_label.into());
    state.set_upgrade_label(upgrade_label.into());
    state.set_check_hint(hint(
        &t.format("hint.check_title", &manager),
        &package_manager::package_commands(&host, package_manager::Mode::Check),
        &check_effect,
        t.get("hint.check_risk"),
    ));
    state.set_upgrade_hint(hint(
        &t.format("hint.upgrade_title", &manager),
        &package_manager::package_commands(&host, package_manager::Mode::Upgrade),
        t.get("hint.upgrade_effect"),
        t.get("hint.upgrade_risk"),
    ));
    state.set_dist_hint(hint(
        t.get("hint.dist_upgrade_title"),
        &package_manager::package_commands(&host, package_manager::Mode::DistUpgrade),
        t.get("hint.dist_upgrade_effect"),
        t.get("hint.dist_upgrade_risk"),
    ));
    state.set_reboot_hint(hint(
        t.get("hint.reboot_title"),
        &[misf_core::reboot::command(&host)],
        t.get("hint.reboot_effect"),
        t.get("hint.reboot_risk"),
    ));
    state.set_upgrade_enabled(App::advanced_enabled(&host, Advanced::Upgrade));
    state.set_dist_enabled(App::advanced_enabled(&host, Advanced::DistUpgrade));
    state.set_reboot_enabled(App::advanced_enabled(&host, Advanced::Reboot));
    state.set_fingerprint(
        if host.fingerprint.is_empty() {
            t.get("host.first_trust").into()
        } else {
            t.format(
                "host.fingerprint_value",
                &[("fingerprint", host.fingerprint.clone())],
            )
        }
        .into(),
    );
    let result = host.last_check.as_ref();
    let successful = result.filter(|r| r.status == "success");
    state.set_pending(
        successful
            .map_or("—".into(), |r| r.packages.len().to_string())
            .into(),
    );
    state.set_status(status_label(t, &host).into());
    state.set_last_check(
        result
            .map_or("—".into(), |r| format_time(&r.checked_at))
            .into(),
    );
    state.set_kept_back(
        result
            .filter(|r| r.is_upgrade() && !r.kept_back.is_empty())
            .map(|r| {
                t.format(
                    "kept_back.banner",
                    &[
                        ("count", r.kept_back.len().to_string()),
                        ("packages", r.kept_back.join(", ")),
                    ],
                )
            })
            .unwrap_or_default()
            .into(),
    );
    state.set_restart(
        result
            .and_then(|r| r.restart.as_ref())
            .and_then(|restart| restart_summary(t, restart))
            .map(|summary| t.format("restart.banner", &[("summary", summary)]))
            .unwrap_or_default()
            .into(),
    );
}

/// The result tabs for the chosen host.
fn push_results(app: &mut App, w: &AppWindow, host: Option<&Host>) {
    let t = app.t();
    let state = w.global::<ResultState>();
    state.set_tab(app.tab as i32);
    if std::mem::take(&mut app.reload.package_search) {
        state.set_package_search(app.package_search.clone().into());
    }
    let result = host.and_then(|h| h.last_check.as_ref());
    let upgrade = result.filter(|r| r.is_upgrade());
    let (message, failed, packages, headers) = if app.tab == 1 {
        let message = match upgrade {
            Some(r) if r.status == "success" && r.upgraded.is_empty() => t.get("apt.upgraded_none"),
            Some(r) if r.status == "success" => t.get("apt.upgraded_description"),
            Some(r) => r.error.as_str(),
            None => t.get("apt.upgrade_not_run"),
        };
        (
            message,
            upgrade.is_some_and(|r| r.status == "failed"),
            upgrade.map(|r| r.upgraded.as_slice()).unwrap_or_default(),
            [
                t.get("apt.package"),
                t.get("apt.previous"),
                t.get("apt.installed"),
                t.get("apt.architecture"),
            ],
        )
    } else {
        let message = match result {
            Some(r) if r.status == "success" => match (r.is_upgrade(), r.packages.is_empty()) {
                (false, true) => t.get("apt.up_to_date"),
                (false, false) => t.get("apt.available_description"),
                (true, true) => t.get("apt.upgrade_clean"),
                (true, false) => t.get("apt.held_back"),
            },
            Some(r) => r.error.as_str(),
            None => t.get("apt.check_hint"),
        };
        (
            message,
            result.is_some_and(|r| r.status == "failed"),
            result.map(|r| r.packages.as_slice()).unwrap_or_default(),
            [
                t.get("apt.package"),
                t.get("apt.current"),
                t.get("apt.candidate"),
                t.get("apt.architecture"),
            ],
        )
    };
    state.set_message(message.into());
    state.set_failed(failed);
    state.set_headers(texts(headers));
    let filter = app.package_search.to_lowercase();
    let mut rows: Vec<ModelRc<SharedString>> = packages
        .iter()
        .filter(|p| p.name.to_lowercase().contains(&filter))
        .map(|p| {
            texts([
                p.name.as_str(),
                p.current_version.as_str(),
                p.candidate_version.as_str(),
                p.architecture.as_str(),
            ])
        })
        .collect();
    state.set_no_data(packages.is_empty());
    keep_inner(&state.get_packages(), &mut rows, |row| row);
    sync_rows(state.get_packages(), rows, |rows| state.set_packages(rows));
    state.set_output(
        misf_core::transcript::readable_output(
            &result
                .map(|r| format!("{}\n{}", r.output, r.error))
                .unwrap_or(t.get("apt.not_run").into()),
        )
        .into(),
    );
    state.set_connection(info_lines(connection_lines(
        t,
        host.and_then(|h| h.last_test.as_ref()),
    )));
    state.set_restart(info_lines(
        host.map(|h| restart_lines(t, h)).unwrap_or_default(),
    ));
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<HostState>();
    let c = ctx.clone();
    state.on_delete(move || {
        c.update(|app| {
            if app.current().is_some() {
                app.delete_id = Some(app.selected.clone());
                app.log(Action::DeletePrompt, Outcome::Succeeded);
            }
        })
    });
    let c = ctx.clone();
    state.on_edit(move || c.update(|app| app.open_editor(app.current().cloned())));
    let c = ctx.clone();
    state.on_test(move || c.update(|app| app.start_connection_action("test")));
    let c = ctx.clone();
    state.on_probe(move || {
        c.update(|app| {
            app.after_trust = None;
            app.send(json!({"action":"probe","id":app.selected}));
        })
    });
    let c = ctx.clone();
    state.on_check(move || c.update(|app| app.start_connection_action("check")));
    let c = ctx.clone();
    state.on_reload(move || c.update(|app| app.send(json!({"action":"list"}))));
    let c = ctx.clone();
    state.on_add_first(move || c.update(|app| app.open_editor(None)));
    let c = ctx.clone();
    state.on_advanced_toggled(move |open| c.update(|app| app.advanced_open = open));
    let c = ctx.clone();
    state.on_advanced(move |item| {
        let item = match item {
            0 => Advanced::Upgrade,
            1 => Advanced::DistUpgrade,
            _ => Advanced::Reboot,
        };
        c.update(|app| app.choose_advanced(item))
    });
    let results = w.global::<ResultState>();
    let c = ctx.clone();
    results.on_tab_chosen(move |tab| c.update(|app| app.set_result_tab(tab.max(0) as usize)));
    let c = ctx.clone();
    results.on_package_search_edited(move |text| {
        c.update(|app| {
            app.package_search = text.into();
            app.log(Action::PackageSearch, Outcome::Succeeded);
        })
    });
}
