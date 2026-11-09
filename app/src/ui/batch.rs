//! The batch view: one package manager's ready hosts, their table, and batch requests.

use super::{
    App, AppWindow, BatchCell, BatchColumnView, BatchRowView, BatchState, Ctx, ManagerChoice,
    app::View,
    format::{
        batch_managers, batch_ready, format_time, manager_label, operation_label, pending_count,
        restart_summary, status_label, system_label,
    },
    hint, keep_inner, sync_rows,
};
use crate::{
    i18n::Translator,
    preferences::{BatchColumn, BatchDetail, BatchOrder},
};
use chrono::DateTime;
use misf_core::{
    audit::{Action, Outcome},
    package_manager,
};
use serde_json::json;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::collections::BTreeSet;

/// The narrowest a column gets while its edge is dragged.
const MIN_COLUMN: f32 = 52.0;

/// A group's heading in the batch table: the row it comes before, its name and host IDs.
pub type Heading = (usize, String, Vec<String>);

impl App {
    /// Keeps the selection to ready hosts of one package manager, defaulting to the most common.
    pub fn prune_batch(&mut self) {
        let managers = batch_managers(&self.hosts);
        if !managers.iter().any(|(id, _)| *id == self.batch_manager) {
            self.batch_manager = managers
                .first()
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
        }
        let (hosts, manager) = (&self.hosts, &self.batch_manager);
        self.batch_selected.retain(|id| {
            hosts
                .iter()
                .any(|h| h.id == *id && batch_ready(h) && h.manager() == manager)
        });
    }

    /// Host indexes in table order, and in sidebar order each group's heading.
    pub fn batch_order(&self, t: Translator) -> (Vec<usize>, Vec<Heading>) {
        // Stable sorts keep the sidebar order among equals.
        let mut order: Vec<usize> = (0..self.hosts.len()).collect();
        let mut headings = Vec::new();
        match self.preferences.batch_order {
            BatchOrder::Sidebar if !self.groups.is_empty() => {
                order.clear();
                for (group, ids) in self.sections() {
                    if ids.is_empty() {
                        continue;
                    }
                    let name = self
                        .groups
                        .iter()
                        .find(|g| g.id == group)
                        .map_or(t.get("group.ungrouped"), |g| g.name.as_str());
                    headings.push((order.len(), name.to_owned(), ids.clone()));
                    order.extend(
                        ids.iter()
                            .filter_map(|id| self.hosts.iter().position(|h| h.id == *id)),
                    );
                }
            }
            BatchOrder::Sidebar => {}
            BatchOrder::Pending => {
                order.sort_by_key(|&i| std::cmp::Reverse(pending_count(&self.hosts[i])));
            }
            BatchOrder::Recent => order.sort_by_key(|&i| {
                std::cmp::Reverse(self.hosts[i].last_check.as_ref().and_then(|r| {
                    DateTime::parse_from_rfc3339(&r.checked_at)
                        .ok()
                        .map(|time| time.timestamp())
                }))
            }),
            BatchOrder::Name => order.sort_by_cached_key(|&i| self.hosts[i].name.to_lowercase()),
        }
        (order, headings)
    }

    /// Ready hosts of the chosen package manager.
    pub fn batch_ready_ids(&self) -> BTreeSet<String> {
        self.hosts
            .iter()
            .filter(|h| batch_ready(h) && h.manager() == self.batch_manager)
            .map(|h| h.id.clone())
            .collect()
    }

    /// The selection in display order, never the set's order, so a request matches what
    /// the user sees.
    pub fn batch_targets(&self) -> Vec<String> {
        let (order, _) = self.batch_order(self.t());
        order
            .iter()
            .map(|&i| &self.hosts[i])
            .filter(|h| self.batch_selected.contains(&h.id))
            .map(|h| h.id.clone())
            .collect()
    }

    pub fn choose_batch_manager(&mut self, id: &str) {
        if self.batch_manager != id {
            self.batch_manager = id.to_owned();
            self.batch_selected.clear();
            self.log(Action::BatchSelect, Outcome::Succeeded);
        }
    }

    pub fn toggle_batch_host(&mut self, id: &str) {
        if !self.batch_ready_ids().contains(id) {
            return;
        }
        if !self.batch_selected.remove(id) {
            self.batch_selected.insert(id.to_owned());
        }
        self.log(Action::BatchSelect, Outcome::Succeeded);
    }

    /// A heading's checkbox picks every host of its group that can run, or none.
    pub fn toggle_batch_group(&mut self, ids: &[String]) {
        let ready = self.batch_ready_ids();
        let eligible: Vec<&String> = ids.iter().filter(|id| ready.contains(*id)).collect();
        if eligible.is_empty() {
            return;
        }
        let all = eligible.iter().all(|id| self.batch_selected.contains(*id));
        for id in eligible {
            if all {
                self.batch_selected.remove(id);
            } else {
                self.batch_selected.insert(id.clone());
            }
        }
        self.log(Action::BatchSelect, Outcome::Succeeded);
    }

    /// Opens a host's page from the batch table.
    pub fn open_host_page(&mut self, id: &str) {
        self.selected = id.to_owned();
        self.batch_popup = false;
        self.set_view(View::Host);
    }

    /// Picks the previous (-1) or next (+1) host in the table's order.
    pub fn step_batch_host(&mut self, step: isize) {
        let (order, _) = self.batch_order(self.t());
        let Some(index) = order
            .iter()
            .position(|&i| self.hosts[i].id == self.selected)
        else {
            return;
        };
        let Some(next) = index.checked_add_signed(step).filter(|i| *i < order.len()) else {
            return;
        };
        let id = self.hosts[order[next]].id.clone();
        self.select_host(&id);
    }

    /// Moves the edge left of visible column `k` by `dx` points in a table `spare` wide:
    /// width moves between the two neighbours; the others keep theirs.
    pub fn drag_column(&mut self, k: usize, dx: f32, spare: f32) {
        let columns = self.preferences.batch_columns.visible();
        if k == 0 || k >= columns.len() || spare <= 0.0 {
            return;
        }
        let weights = &mut self.preferences.batch_columns.weights;
        let total: f32 = columns.iter().map(|(i, _)| weights[*i]).sum();
        let per_point = total / spare;
        let floor = MIN_COLUMN * per_point;
        let (left, right) = (columns[k - 1].0, columns[k].0);
        let shift = (dx * per_point).clamp(
            (floor - weights[left]).min(0.0),
            (weights[right] - floor).max(0.0),
        );
        weights[left] += shift;
        weights[right] -= shift;
    }

    pub fn batch_request(&mut self, mode: package_manager::Mode) {
        let selected = self.batch_targets();
        if selected.is_empty() {
            return;
        }
        match mode {
            package_manager::Mode::Check => {
                self.send(json!({"action":"batch","operation":"check","ids":selected}))
            }
            mode => self.prompt_upgrade(selected, true, mode),
        }
    }
}

/// One host's or heading's row in the batch table, as display text only.
fn rows(app: &App, t: Translator, order: &[usize], headings: &[Heading]) -> Vec<BatchRowView> {
    let ready = app.batch_ready_ids();
    let columns = app.preferences.batch_columns.visible();
    let mut rows = Vec::new();
    let mut line = 0;
    for (row, &index) in order.iter().enumerate() {
        if let Some((number, (_, heading, ids))) =
            headings.iter().enumerate().find(|(_, (at, ..))| *at == row)
        {
            let eligible: Vec<&String> = ids.iter().filter(|id| ready.contains(*id)).collect();
            let chosen = eligible
                .iter()
                .filter(|id| app.batch_selected.contains(**id))
                .count();
            rows.push(BatchRowView {
                heading: true,
                id: number.to_string().into(),
                text: format!("{heading} · {}", ids.len()).into(),
                check: match chosen {
                    0 => 0,
                    n if n == eligible.len() => 2,
                    _ => 1,
                },
                check_enabled: !eligible.is_empty(),
                reason: t.get("batch_order.group_select").into(),
                stripe: true,
                cells: Default::default(),
                ..Default::default()
            });
            line = 0;
        }
        let h = &app.hosts[index];
        // Only display fields are copied out of the host list; credentials stay in place.
        let reason = if h.fingerprint.is_empty() {
            t.get("batch.untrusted").to_owned()
        } else if !batch_ready(h) {
            t.get("batch.undetected").to_owned()
        } else if h.manager() != app.batch_manager {
            t.format(
                "batch.other_manager",
                &[("manager", manager_label(t, h.manager()).into())],
            )
        } else {
            String::new()
        };
        let result = h.last_check.as_ref();
        let success = result.filter(|r| r.status == "success");
        let upgrade = success.filter(|r| r.is_upgrade());
        let dash = || "—".to_owned();
        let phased = upgrade
            .map(|r| r.phased.join(", "))
            .filter(|names| !names.is_empty());
        let cells: Vec<BatchCell> = columns
            .iter()
            .map(|(_, column)| {
                let text = match column {
                    BatchColumn::Host => h.name.clone(),
                    BatchColumn::User => h.username.clone(),
                    BatchColumn::Address => format!("{}:{}", h.address, h.port),
                    BatchColumn::System if h.distro.is_empty() => dash(),
                    BatchColumn::System => system_label(t, h),
                    BatchColumn::Operation => {
                        result.map_or_else(dash, |r| operation_label(t, &r.operation).to_owned())
                    }
                    BatchColumn::Status => status_label(t, h).to_owned(),
                    BatchColumn::Pending => {
                        success.map_or_else(dash, |r| r.packages.len().to_string())
                    }
                    BatchColumn::Phased => {
                        upgrade.map_or_else(dash, |r| r.phased.len().to_string())
                    }
                    BatchColumn::Upgraded => {
                        upgrade.map_or_else(dash, |r| r.upgraded.len().to_string())
                    }
                    BatchColumn::Checked => {
                        result.map_or_else(dash, |r| format_time(&r.checked_at))
                    }
                    BatchColumn::Restart => match result.and_then(|r| r.restart.as_ref()) {
                        Some(restart) => restart_summary(t, restart)
                            .unwrap_or_else(|| t.get("restart.none").to_owned()),
                        None => dash(),
                    },
                };
                let (kind, hint) = match column {
                    BatchColumn::Host => (1, String::new()),
                    BatchColumn::Status => (2, String::new()),
                    BatchColumn::Phased => (
                        3,
                        phased
                            .as_ref()
                            .map(|names| {
                                t.format("batch.phased_hint", &[("packages", names.clone())])
                            })
                            .unwrap_or_default(),
                    ),
                    _ => (0, String::new()),
                };
                BatchCell {
                    text: text.into(),
                    kind,
                    hint: hint.into(),
                }
            })
            .collect();
        rows.push(BatchRowView {
            heading: false,
            id: h.id.clone().into(),
            text: Default::default(),
            check: if app.batch_selected.contains(&h.id) {
                2
            } else {
                0
            },
            check_enabled: reason.is_empty(),
            reason: reason.into(),
            cells: ModelRc::new(VecModel::from(cells)),
            selected: app.selected == h.id,
            stripe: line % 2 == 1,
            failed: result.is_some_and(|r| r.status == "failed"),
        });
        line += 1;
    }
    rows
}

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    let state = w.global::<BatchState>();
    app.prune_batch();
    let managers = batch_managers(&app.hosts);
    state.set_managers(ModelRc::new(VecModel::from(
        managers
            .iter()
            .map(|(id, count)| ManagerChoice {
                id: id.clone().into(),
                label: format!("{} ({count})", manager_label(t, id)).into(),
                selected: app.batch_manager == *id,
            })
            .collect::<Vec<_>>(),
    )));
    let manager = [("manager", manager_label(t, &app.batch_manager).to_owned())];
    let commands = |mode| {
        package_manager::Manager::from_id(&app.batch_manager)
            .map(|m| package_manager::manager_commands(m, false, mode))
            .unwrap_or_default()
    };
    let (order, headings) = app.batch_order(t);
    let selected = app.batch_targets();
    let count = [("count", selected.len().to_string()), manager[0].clone()];
    state.set_selected_text(t.format("batch.selected", &count).into());
    state.set_check_label(t.format("batch.check", &count).into());
    state.set_upgrade_label(t.format("batch.upgrade", &count).into());
    state.set_dist_label(t.format("batch.dist_upgrade", &count).into());
    state.set_show_dist(app.batch_manager == "apt");
    state.set_any_selected(!selected.is_empty());
    state.set_any_ready(!app.batch_ready_ids().is_empty());
    state.set_check_hint(hint(
        &t.format("hint.batch_check_title", &manager),
        &commands(package_manager::Mode::Check),
        t.get("hint.batch_check_effect"),
        t.get("hint.batch_check_risk"),
    ));
    state.set_upgrade_hint(hint(
        &t.format("hint.batch_upgrade_title", &manager),
        &commands(package_manager::Mode::Upgrade),
        t.get("hint.batch_upgrade_effect"),
        t.get("hint.batch_upgrade_risk"),
    ));
    state.set_dist_hint(hint(
        t.get("hint.batch_dist_upgrade_title"),
        &commands(package_manager::Mode::DistUpgrade),
        t.get("hint.dist_upgrade_effect"),
        t.get("hint.dist_upgrade_risk"),
    ));
    let columns = app.preferences.batch_columns.visible();
    let weights = app.preferences.batch_columns.weights;
    state.set_columns(ModelRc::new(VecModel::from(
        columns
            .iter()
            .map(|(i, column)| BatchColumnView {
                label: column.label(t).into(),
                weight: weights[*i],
            })
            .collect::<Vec<_>>(),
    )));
    let mut table = rows(app, t, &order, &headings);
    keep_inner(&state.get_rows(), &mut table, |row| &mut row.cells);
    sync_rows(state.get_rows(), table, |rows| state.set_rows(rows));
    let host = app.current();
    let popup_mode = app.preferences.batch_detail == BatchDetail::Popup;
    if host.is_none() || !popup_mode || app.view != View::Batch {
        app.batch_popup = false;
    }
    let host = app.current();
    state.set_popup_mode(popup_mode);
    state.set_detail_inline(host.is_some() && !popup_mode);
    state.set_popup_shown(app.batch_popup);
    if let Some(host) = host {
        state.set_detail_title(
            t.format("batch.detail", &[("name", host.name.clone())])
                .into(),
        );
        // Previous and next follow the table's sorted order.
        let index = order.iter().position(|&i| app.hosts[i].id == host.id);
        state.set_previous_enabled(index.is_some_and(|i| i > 0));
        state.set_next_enabled(index.is_some_and(|i| i + 1 < order.len()));
    }
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<BatchState>();
    let c = ctx.clone();
    state.on_choose_manager(move |id| c.update(|app| app.choose_batch_manager(&id)));
    let c = ctx.clone();
    state.on_select_all(move || {
        c.update(|app| {
            app.batch_selected = app.batch_ready_ids();
            app.log(Action::BatchSelect, Outcome::Succeeded);
        })
    });
    let c = ctx.clone();
    state.on_clear(move || {
        c.update(|app| {
            app.batch_selected.clear();
            app.log(Action::BatchSelect, Outcome::Succeeded);
        })
    });
    let c = ctx.clone();
    state.on_check(move || c.update(|app| app.batch_request(package_manager::Mode::Check)));
    let c = ctx.clone();
    state.on_upgrade(move || c.update(|app| app.batch_request(package_manager::Mode::Upgrade)));
    let c = ctx.clone();
    state.on_dist_upgrade(move || {
        c.update(|app| app.batch_request(package_manager::Mode::DistUpgrade))
    });
    let c = ctx.clone();
    state.on_toggle(move |id| c.update(|app| app.toggle_batch_host(&id)));
    let c = ctx.clone();
    state.on_toggle_group(move |heading| {
        c.update(|app| {
            let (_, headings) = app.batch_order(app.t());
            if let Some((_, _, ids)) = heading.parse::<usize>().ok().and_then(|i| headings.get(i)) {
                let ids = ids.clone();
                app.toggle_batch_group(&ids);
            }
        })
    });
    let c = ctx.clone();
    state.on_select_host(move |id| c.update(|app| app.select_host(&id)));
    let c = ctx.clone();
    state.on_open_host(move |id| c.update(|app| app.open_host_page(&id)));
    let c = ctx.clone();
    state.on_open_detail(move |id| {
        c.update(|app| {
            app.selected = id.into();
            app.batch_popup = true;
        })
    });
    let c = ctx.clone();
    state.on_column_drag(move |k, dx, spare| {
        c.update(|app| app.drag_column(k.max(0) as usize, dx, spare))
    });
    let c = ctx.clone();
    state.on_column_drag_end(move || {
        c.update(|app| {
            let _ = app.save_preferences();
        })
    });
    let c = ctx.clone();
    state.on_column_reset(move || {
        c.update(|app| {
            app.preferences.batch_columns.reset_widths();
            let _ = app.save_preferences();
        })
    });
    let c = ctx.clone();
    state.on_previous(move || c.update(|app| app.step_batch_host(-1)));
    let c = ctx.clone();
    state.on_next(move || c.update(|app| app.step_batch_host(1)));
    let c = ctx.clone();
    state.on_close_popup(move || c.update(|app| app.batch_popup = false));
}
