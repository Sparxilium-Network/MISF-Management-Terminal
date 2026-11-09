//! The host list: groups in the chosen order, host cards, and dragging either to rearrange.

use super::{
    App, AppWindow, Ctx, SidebarRow, SidebarState, app::GroupDialog, keep_inner, sync_rows, texts,
};
use crate::preferences::Density;
use misf_core::{
    audit::{Action, Outcome},
    model::Host,
};
use serde_json::{Value, json};
use slint::ComponentHandle;

/// A sidebar row being dragged, and where it would land.
pub struct Drag {
    pub group: bool,
    pub id: String,
    pub drop: Option<(DropAt, f32)>,
    /// The pointer in list content coordinates.
    pub y: f32,
}

/// Where a drag would land: before a host or group, or at the end when `before` is `None`.
#[derive(Clone, Debug, PartialEq)]
pub enum DropAt {
    Host {
        group: String,
        before: Option<String>,
    },
    Group {
        before: Option<String>,
    },
}

/// Row kinds as the Slint list draws them.
const HEADING: i32 = 0;
const HOST: i32 = 1;
const MESSAGE: i32 = 2;

/// Each row's place in the list, by kind, ID, group, top and height.
pub struct Layout {
    rows: Vec<(i32, String, String, f32, f32)>,
}

impl Layout {
    /// Heading and cards of each section, by group ID, with their vertical extent.
    fn sections(&self) -> Vec<(f32, f32, String)> {
        let mut sections: Vec<(f32, f32, String)> = Vec::new();
        for (kind, _, group, top, height) in &self.rows {
            match sections.last_mut() {
                Some(last) if *kind != HEADING && last.2 == *group => last.1 = top + height,
                _ if *kind == MESSAGE => {}
                _ => sections.push((*top, top + height, group.clone())),
            }
        }
        sections
    }

    /// The drop for a host and the y of its insertion line.
    pub fn host_drop(&self, y: f32, dragged: &str) -> Option<(DropAt, f32)> {
        let cards: Vec<_> = self
            .rows
            .iter()
            .filter(|(kind, ..)| *kind == HOST)
            .collect();
        let card = cards
            .iter()
            .position(|(.., top, height)| (top - 3.0..=top + height + 3.0).contains(&y));
        if let Some(index) = card {
            let (_, id, group, top, height) = cards[index];
            if id == dragged {
                return None;
            }
            if y < top + height / 2.0 {
                let at = DropAt::Host {
                    group: group.clone(),
                    before: Some(id.clone()),
                };
                return Some((at, top - 2.5));
            }
            let before = cards[index + 1..]
                .iter()
                .find(|(_, other, ..)| other != dragged)
                .filter(|(_, _, other_group, ..)| other_group == group)
                .map(|(_, other, ..)| other.clone());
            let at = DropAt::Host {
                group: group.clone(),
                before,
            };
            return Some((at, top + height + 2.5));
        }
        // A heading or a section's empty space: the end of that section.
        let (_, bottom, group) = self
            .sections()
            .into_iter()
            .find(|(top, bottom, _)| (*top..*bottom).contains(&y))?;
        let at = DropAt::Host {
            group,
            before: None,
        };
        Some((at, bottom - 2.0))
    }

    /// The drop for a group and the y of its insertion line.
    pub fn group_drop(&self, y: f32, dragged: &str) -> Option<(DropAt, f32)> {
        let sections = self.sections();
        let index = sections
            .iter()
            .position(|(top, bottom, _)| (*top..*bottom).contains(&y))?;
        let (top, bottom, group) = &sections[index];
        if group == dragged {
            return None;
        }
        if y < (top + bottom) / 2.0 {
            let at = DropAt::Group {
                before: Some(group.clone()),
            };
            return Some((at, *top));
        }
        let before = sections[index + 1..]
            .iter()
            .find(|(.., other)| other != dragged)
            .map(|(.., other)| other.clone());
        Some((DropAt::Group { before }, *bottom))
    }
}

impl App {
    /// Group IDs in sidebar order, with "" where the ungrouped hosts sit.
    pub fn section_order(&self) -> Vec<String> {
        let mut order: Vec<String> = self.groups.iter().map(|g| g.id.clone()).collect();
        let at = (self.preferences.ungrouped_at)
            .unwrap_or(order.len())
            .min(order.len());
        order.insert(at, String::new());
        order
    }

    /// Host IDs under each sidebar heading, in sidebar order ("" = ungrouped).
    pub fn sections(&self) -> Vec<(String, Vec<String>)> {
        let mut sections: Vec<(String, Vec<String>)> = (self.section_order().into_iter())
            .map(|group| (group, Vec::new()))
            .collect();
        let ungrouped = sections.iter().position(|(group, _)| group.is_empty());
        for host in &self.hosts {
            let index = sections
                .iter()
                .position(|(group, _)| *group == host.group_id)
                .or(ungrouped)
                .unwrap_or_default();
            sections[index].1.push(host.id.clone());
        }
        sections
    }

    /// The rows the sidebar shows, with their places in the list.
    pub fn sidebar_rows(&self) -> Vec<SidebarRow> {
        let t = self.t();
        let compact = self.preferences.density == Density::Compact;
        let fields = self.preferences.host_card_fields();
        let query = self.search.to_lowercase();
        let searching = !query.is_empty();
        let matches = |h: &Host| {
            format!("{} {} {}", h.name, h.address, h.notes)
                .to_lowercase()
                .contains(&query)
        };
        let show_headers = !self.groups.is_empty();
        let mut rows = Vec::new();
        let mut y = 0.0;
        let mut push = |rows: &mut Vec<SidebarRow>, mut row: SidebarRow, height: f32| {
            row.y = y;
            row.height = height;
            y += height;
            rows.push(row);
        };
        if !self.hosts.iter().any(matches) && !show_headers {
            let text = if self.hosts.is_empty() {
                t.get("host.empty")
            } else {
                t.get("host.no_match")
            };
            let row = SidebarRow {
                kind: MESSAGE,
                text: text.into(),
                ..Default::default()
            };
            push(&mut rows, row, 80.0);
        }
        let dragged = self.drag.as_ref().map(|d| d.id.as_str());
        for (group_id, ids) in self.sections() {
            let hosts: Vec<&Host> = ids
                .iter()
                .filter_map(|id| self.hosts.iter().find(|h| h.id == *id))
                .filter(|h| matches(h))
                .collect();
            if searching && hosts.is_empty() {
                continue;
            }
            let group = self.groups.iter().find(|g| g.id == group_id);
            let collapsed =
                !searching && group.map_or(self.preferences.ungrouped_collapsed, |g| g.collapsed);
            if show_headers {
                let row = SidebarRow {
                    kind: HEADING,
                    id: group_id.clone().into(),
                    text: group
                        .map_or(t.get("group.ungrouped"), |g| g.name.as_str())
                        .into(),
                    count: hosts.len().to_string().into(),
                    collapsed,
                    editable: group.is_some(),
                    ..Default::default()
                };
                push(&mut rows, row, if compact { 24.0 } else { 30.0 });
            }
            if collapsed {
                continue;
            }
            // Hosts in a group sit under its heading; ungrouped ones stay at the edge.
            let indent = if show_headers && !group_id.is_empty() {
                16.0
            } else {
                0.0
            };
            for host in hosts {
                let mut lines = Vec::new();
                if fields.address {
                    lines.push(format!("{}@{}:{}", host.username, host.address, host.port));
                }
                if fields.system && !host.distro.is_empty() {
                    lines.push(host.distro.clone());
                }
                let note = (host.notes.lines())
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .filter(|_| fields.notes);
                lines.extend(note.map(str::to_owned));
                let success = host
                    .last_check
                    .as_ref()
                    .is_some_and(|r| r.status == "success");
                let height = if compact {
                    22.0
                } else if lines.is_empty() {
                    // A card with only its name is shorter, to fit more hosts.
                    39.0
                } else {
                    48.0 + 16.0 * lines.len() as f32
                };
                let row = SidebarRow {
                    kind: HOST,
                    id: host.id.clone().into(),
                    group: group_id.clone().into(),
                    text: host.name.clone().into(),
                    detail: lines.join("  ·  ").into(),
                    tooltip: if compact {
                        lines.join("\n").into()
                    } else {
                        Default::default()
                    },
                    lines: texts(lines),
                    dot: match (fields.status, success) {
                        (false, _) => 0,
                        (true, false) => 1,
                        (true, true) => 2,
                    },
                    indent,
                    selected: self.selected == host.id,
                    dragged: dragged == Some(host.id.as_str()),
                    ..Default::default()
                };
                push(&mut rows, row, height);
            }
        }
        rows
    }

    pub fn toggle_group(&mut self, id: &str) {
        if id.is_empty() {
            // The ungrouped heading collapses like a group, but as a display preference.
            self.preferences.ungrouped_collapsed = !self.preferences.ungrouped_collapsed;
            let _ = self.save_preferences();
            return;
        }
        let Some(group) = self.groups.iter_mut().find(|g| g.id == id) else {
            return;
        };
        group.collapsed = !group.collapsed;
        let collapsed = group.collapsed;
        self.send(json!({"action":"group_collapse","id":id,"collapsed":collapsed}));
    }

    /// Follows a drag of a host or group, at `y` in list content coordinates.
    pub fn drag_to(&mut self, group: bool, id: &str, y: f32, inside: bool) {
        let layout = Layout {
            rows: self
                .sidebar_rows()
                .iter()
                .map(|r| {
                    let group = if r.kind == HEADING { &r.id } else { &r.group };
                    (r.kind, r.id.to_string(), group.to_string(), r.y, r.height)
                })
                .collect(),
        };
        let drop = inside
            .then(|| {
                if group {
                    layout.group_drop(y, id)
                } else {
                    layout.host_drop(y, id)
                }
            })
            .flatten();
        self.drag = Some(Drag {
            group,
            id: id.to_owned(),
            drop,
            y,
        });
    }

    /// Drops the dragged row where it would land, if anywhere.
    pub fn end_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        match drag.drop {
            Some((DropAt::Host { group, before }, _)) if !drag.group => {
                self.move_host(&drag.id, &group, before.as_deref());
            }
            Some((DropAt::Group { before }, _)) if drag.group => {
                self.move_group(&drag.id, before.as_deref());
            }
            _ => {}
        }
    }

    pub fn move_host(&mut self, id: &str, group: &str, before: Option<&str>) {
        let mut sections = self.sections();
        for (_, ids) in &mut sections {
            ids.retain(|host| host != id);
        }
        if let Some((_, ids)) = sections.iter_mut().find(|(g, _)| g == group) {
            let at = before
                .and_then(|b| ids.iter().position(|host| host == b))
                .unwrap_or(ids.len());
            ids.insert(at, id.to_owned());
        }
        let order = self.groups.iter().map(|g| g.id.clone()).collect();
        self.arrange(&sections, order);
    }

    /// Moves a group, or with an empty `id` the ungrouped hosts, before another or to the end.
    pub fn move_group(&mut self, id: &str, before: Option<&str>) {
        let mut order: Vec<String> = (self.section_order().into_iter())
            .filter(|g| g != id)
            .collect();
        let at = before
            .and_then(|b| order.iter().position(|g| g == b))
            .unwrap_or(order.len());
        order.insert(at, id.to_owned());
        // Where the ungrouped hosts sit is a display preference, not part of the host store.
        let ungrouped =
            (order.iter().position(String::is_empty)).filter(|&at| at + 1 < order.len());
        if ungrouped != self.preferences.ungrouped_at {
            self.preferences.ungrouped_at = ungrouped;
            let _ = self.save_preferences();
        }
        order.retain(|g| !g.is_empty());
        self.arrange(&self.sections(), order);
    }

    /// Shows a drag's result at once and saves it; a refused one reloads the stored order.
    pub fn arrange(&mut self, sections: &[(String, Vec<String>)], order: Vec<String>) {
        let placement = |hosts: &[Host]| -> Vec<(String, String)> {
            hosts
                .iter()
                .map(|h| (h.id.clone(), h.group_id.clone()))
                .collect()
        };
        let before = placement(&self.hosts);
        let unchanged_groups = self.groups.iter().map(|g| &g.id).eq(order.iter());
        let mut remaining = std::mem::take(&mut self.hosts);
        for group in std::iter::once("").chain(order.iter().map(String::as_str)) {
            let Some((_, ids)) = sections.iter().find(|(g, _)| g == group) else {
                continue;
            };
            for id in ids {
                if let Some(index) = remaining.iter().position(|h| h.id == *id) {
                    let mut host = remaining.remove(index);
                    host.group_id = group.to_owned();
                    self.hosts.push(host);
                }
            }
        }
        self.hosts.append(&mut remaining);
        let after = placement(&self.hosts);
        if unchanged_groups && before == after {
            return;
        }
        self.groups
            .sort_by_key(|g| order.iter().position(|id| *id == g.id));
        let hosts: Vec<Value> = after
            .iter()
            .map(|(id, group)| json!({"id": id, "group_id": group}))
            .collect();
        self.send(json!({"action":"arrange","groups":order,"hosts":hosts}));
    }

    pub fn open_group_dialog(&mut self, id: Option<&str>) {
        if self.locked {
            return;
        }
        let name = id
            .and_then(|id| self.groups.iter().find(|g| g.id == id))
            .map(|g| g.name.clone())
            .unwrap_or_default();
        self.group_dialog = Some(GroupDialog {
            id: id.map(str::to_owned),
            name,
        });
        self.reload.group = true;
    }

    pub fn save_group(&mut self, name: &str) {
        let name = name.chars().take(64).collect::<String>();
        if self.busy || name.trim().is_empty() {
            return;
        }
        let Some(dialog) = &mut self.group_dialog else {
            return;
        };
        dialog.name = name;
        let request = match &dialog.id {
            Some(id) => json!({"action":"group_rename","id":id,"name":dialog.name}),
            None => json!({"action":"group_add","name":dialog.name}),
        };
        self.send(request);
    }
}

pub fn push(app: &mut App, w: &AppWindow) {
    let t = app.t();
    let state = w.global::<SidebarState>();
    let mut rows = app.sidebar_rows();
    keep_inner(&state.get_rows(), &mut rows, |row| &mut row.lines);
    let height = rows.last().map_or(0.0, |r| r.y + r.height);
    sync_rows(state.get_rows(), rows, |rows| state.set_rows(rows));
    state.set_content_height(height);
    state.set_count_text(
        t.format("host.list_count", &[("count", app.hosts.len().to_string())])
            .into(),
    );
    state.set_locked(app.locked);
    state.set_compact(app.preferences.density == Density::Compact);
    state.set_searching(!app.search.is_empty());
    if std::mem::take(&mut app.reload.search) {
        state.set_search(app.search.clone().into());
    }
    let drag = app.drag.as_ref();
    let line = drag.and_then(|d| d.drop.as_ref()).map(|(_, y)| *y);
    state.set_drop_shown(line.is_some());
    state.set_drop_y(line.unwrap_or_default());
    state.set_drag_y(drag.map_or(0.0, |d| d.y));
    state.set_drag_label(
        drag.map(|d| match (d.group, d.id.as_str()) {
            (true, "") => t.get("group.ungrouped").to_owned(),
            (true, id) => app
                .groups
                .iter()
                .find(|g| g.id == id)
                .map(|g| g.name.clone())
                .unwrap_or_default(),
            (false, id) => app.host_name(id),
        })
        .unwrap_or_default()
        .into(),
    );
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<SidebarState>();
    let c = ctx.clone();
    state.on_search_edited(move |text| {
        c.update(|app| {
            app.search = text.into();
            app.log(Action::HostSearch, Outcome::Succeeded);
        })
    });
    let c = ctx.clone();
    state.on_select(move |id| c.update(|app| app.select_host(&id)));
    let c = ctx.clone();
    state.on_toggle_group(move |id| c.update(|app| app.toggle_group(&id)));
    let c = ctx.clone();
    state.on_rename_group(move |id| c.update(|app| app.open_group_dialog(Some(&id))));
    let c = ctx.clone();
    state.on_delete_group(move |id| {
        c.update(|app| app.send(json!({"action":"group_delete","id":id.as_str()})))
    });
    let c = ctx.clone();
    state.on_add_group(move || c.update(|app| app.open_group_dialog(None)));
    let c = ctx.clone();
    state.on_add_host(move || c.update(|app| app.open_editor(None)));
    let c = ctx.clone();
    state.on_drag_move(move |kind, id, y, inside| {
        c.update(|app| app.drag_to(kind == HEADING, &id, y, inside))
    });
    let c = ctx.clone();
    state.on_drag_end(move || c.update(App::end_drag));
    let c = ctx.clone();
    state.on_drag_cancel(move || c.update(|app| app.drag = None));
}
