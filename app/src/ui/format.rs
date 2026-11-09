//! Labels and text the pages build from hosts and results.

use crate::i18n::Translator;
use chrono::{DateTime, Local};
use misf_core::{
    model::{CheckResult, ConnectionTest, Host, RestartStatus},
    package_manager,
};

/// Batch operations need a confirmed fingerprint and a detected, supported package manager.
pub fn batch_ready(host: &Host) -> bool {
    !host.fingerprint.is_empty() && package_manager::Manager::from_id(host.manager()).is_some()
}
/// Batch-ready package managers with their host counts, most hosts first.
pub fn batch_managers(hosts: &[Host]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for host in hosts.iter().filter(|h| batch_ready(h)) {
        match counts.iter_mut().find(|(id, _)| id == host.manager()) {
            Some((_, count)) => *count += 1,
            None => counts.push((host.manager().into(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

/// Label and value rows for the last "test connection"; empty strings mean not reached.
pub fn connection_rows(t: Translator, test: &ConnectionTest) -> Vec<(&'static str, String)> {
    let or_dash = |value: &str| {
        if value.is_empty() {
            "—".to_owned()
        } else {
            value.to_owned()
        }
    };
    let with_fingerprint = |algorithm: &str, fingerprint: &str| match (algorithm, fingerprint) {
        ("", "") => "—".to_owned(),
        (algorithm, "") => algorithm.to_owned(),
        ("", fingerprint) => fingerprint.to_owned(),
        (algorithm, fingerprint) => format!("{algorithm}\n{fingerprint}"),
    };
    let negotiated = !test.kex.is_empty();
    let kex = if test.kex.contains("mlkem") || test.kex.contains("sntrup") {
        format!("{}  ·  {}", test.kex, t.get("conn.post_quantum"))
    } else {
        or_dash(&test.kex)
    };
    // AEAD ciphers authenticate by themselves, so SSH negotiates no separate MAC.
    let mac = match (test.client_mac.as_str(), test.server_mac.as_str()) {
        ("", "") => "—".to_owned(),
        ("none", "none") => t.get("conn.mac_aead").to_owned(),
        (client, server) if client == server => client.to_owned(),
        (client, server) => t.format(
            "conn.mac_split",
            &[("client", client.into()), ("server", server.into())],
        ),
    };
    let strict = match (negotiated, test.strict_kex) {
        (false, _) => "—".to_owned(),
        (true, true) => t.get("conn.strict_yes").to_owned(),
        (true, false) => t.get("conn.strict_no").to_owned(),
    };
    let auth = match test.auth_method.as_str() {
        "password" => t.get("conn.auth_password").to_owned(),
        "publickey" => format!(
            "{}\n{}",
            t.format(
                "conn.auth_key",
                &[("algorithm", test.client_key_algorithm.clone())]
            ),
            test.client_key_fingerprint
        ),
        _ => "—".to_owned(),
    };
    let system = match (test.distro.as_str(), test.kernel.as_str()) {
        ("", kernel) => or_dash(kernel),
        (distro, "") => distro.to_owned(),
        (distro, kernel) => format!("{distro}  ·  {kernel}"),
    };
    let managers = if test.package_managers.is_empty() {
        "—".to_owned()
    } else {
        test.package_managers
            .iter()
            .map(|id| manager_label(t, id))
            .collect::<Vec<_>>()
            .join("、")
    };
    vec![
        (t.get("conn.server"), or_dash(&test.server_version)),
        (
            t.get("conn.host_key"),
            with_fingerprint(&test.host_key_algorithm, &test.host_key_fingerprint),
        ),
        (t.get("conn.kex"), kex),
        (t.get("conn.cipher"), or_dash(&test.cipher)),
        (t.get("conn.mac"), mac),
        (t.get("conn.strict"), strict),
        (t.get("conn.auth"), auth),
        (t.get("conn.system"), system),
        (t.get("conn.managers"), managers),
    ]
}

pub fn manager_label(t: Translator, id: &str) -> &'static str {
    package_manager::Manager::from_id(id)
        .map(package_manager::Manager::label)
        .unwrap_or(t.get("manager.none"))
}
/// "Ubuntu 26.04 · APT" once detected, otherwise the OS family chosen in the editor.
pub fn system_label(t: Translator, host: &Host) -> String {
    if host.distro.is_empty() {
        host.os.clone()
    } else {
        format!("{} · {}", host.distro, manager_label(t, host.manager()))
    }
}
/// Pending updates from the last successful run; hosts without one sort with zero.
pub fn pending_count(host: &Host) -> usize {
    host.last_check
        .as_ref()
        .filter(|r| r.status == "success")
        .map_or(0, |r| r.packages.len())
}
/// The name of a stored or requested operation: check, upgrade or dist-upgrade.
pub fn operation_label(t: Translator, operation: &str) -> &'static str {
    match operation {
        "upgrade" => t.get("batch.operation_upgrade"),
        "dist_upgrade" => t.get("batch.operation_dist_upgrade"),
        _ => t.get("batch.operation_check"),
    }
}

/// A short line of what needs restarting, or `None` when nothing does.
pub fn restart_summary(t: Translator, restart: &RestartStatus) -> Option<String> {
    let mut parts = vec![];
    if restart.reboot_required {
        parts.push(t.get("restart.reboot_short").to_owned());
    }
    let count = restart.services.len() + restart.containers.len();
    if count > 0 {
        parts.push(t.format("restart.services_short", &[("count", count.to_string())]));
    }
    (!parts.is_empty()).then(|| parts.join(t.get("restart.separator")))
}

pub fn record_result(host: &mut Host, result: CheckResult) {
    if !result.distro.is_empty() {
        host.distro = result.distro.clone();
        host.package_manager = result.package_manager.clone();
    }
    host.last_check = Some(result);
}
pub fn status_label(t: Translator, host: &Host) -> &'static str {
    match host.last_check.as_ref() {
        Some(r) => match (r.status.as_str(), r.is_upgrade()) {
            ("success", false) => t.get("apt.complete"),
            ("success", true) => t.get("apt.upgrade_complete"),
            ("unsupported", _) => t.get("apt.unsupported"),
            (_, false) => t.get("apt.failed"),
            (_, true) => t.get("apt.upgrade_failed"),
        },
        None => t.get("apt.not_checked"),
    }
}

/// The commands a control runs, one per paragraph, as they would be copied.
pub fn copied_commands(commands: &[&str]) -> String {
    let mut text = commands.join("\n\n");
    text.push('\n');
    text
}

pub fn format_time(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Local).format("%m/%d %H:%M").to_string())
        .unwrap_or_else(|_| value.into())
}

/// One line of a text panel, drawn by Slint's `InfoLines`.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    pub value: String,
}

/// How a [`Line`] looks; the numbers are the kinds `InfoLines` draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Body = 0,
    Muted = 1,
    Warning = 2,
    Error = 3,
    Title = 4,
    Bullet = 5,
    Small = 6,
    MonoPair = 7,
    Pair = 8,
    Gap = 9,
    Mono = 10,
}

impl Line {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            value: String::new(),
        }
    }

    pub fn pair(kind: Kind, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            kind,
            text: label.into(),
            value: value.into(),
        }
    }
}

/// The connection details tab: how the last test connected and what it found.
pub fn connection_lines(t: Translator, test: Option<&ConnectionTest>) -> Vec<Line> {
    let Some(test) = test else {
        return vec![Line::new(Kind::Muted, t.get("conn.not_run"))];
    };
    let succeeded = test.status == "success";
    let time = [("time", format_time(&test.tested_at))];
    let mut lines = vec![if succeeded {
        Line::new(Kind::Muted, t.format("conn.succeeded", &time))
    } else {
        Line::new(Kind::Error, t.format("conn.failed", &time))
    }];
    if !succeeded && !test.error.is_empty() {
        lines.push(Line::new(Kind::Error, test.error.clone()));
    }
    lines.push(Line::new(Kind::Gap, ""));
    lines.extend(
        connection_rows(t, test)
            .into_iter()
            .map(|(label, value)| Line::pair(Kind::MonoPair, label, value)),
    );
    lines
}

/// The restart tab: why there is nothing to show, or what needs a reboot or restart.
pub fn restart_lines(t: Translator, host: &Host) -> Vec<Line> {
    if host.manager() != "apt" {
        return vec![Line::new(Kind::Muted, t.get("restart.apt_only"))];
    }
    let Some((checked_at, restart)) = host
        .last_check
        .as_ref()
        .and_then(|r| r.restart.as_ref().map(|s| (&r.checked_at, s)))
    else {
        return vec![Line::new(Kind::Muted, t.get("restart.not_read"))];
    };
    let mut lines = vec![
        Line::new(
            Kind::Small,
            t.format("restart.read_at", &[("time", format_time(checked_at))]),
        ),
        if restart.reboot_required {
            Line::new(Kind::Warning, t.get("restart.reboot_required"))
        } else {
            Line::new(Kind::Body, t.get("restart.no_reboot"))
        },
    ];
    if !restart.running_kernel.is_empty() {
        lines.push(Line::pair(
            Kind::Pair,
            t.get("restart.running_kernel"),
            restart.running_kernel.clone(),
        ));
    }
    if !restart.expected_kernel.is_empty() {
        lines.push(Line::pair(
            Kind::Pair,
            t.get("restart.expected_kernel"),
            restart.expected_kernel.clone(),
        ));
    }
    let list = |lines: &mut Vec<Line>, title: String, names: &[String]| {
        if names.is_empty() {
            return;
        }
        lines.push(Line::new(Kind::Gap, ""));
        lines.push(Line::new(Kind::Title, title));
        lines.extend(
            names
                .iter()
                .map(|name| Line::new(Kind::Bullet, name.clone())),
        );
    };
    let count = |names: &[String]| [("count", names.len().to_string())];
    list(
        &mut lines,
        t.format("restart.packages", &count(&restart.reboot_packages)),
        &restart.reboot_packages,
    );
    if !restart.needrestart {
        lines.push(Line::new(Kind::Gap, ""));
        lines.push(Line::new(Kind::Muted, t.get("restart.no_needrestart")));
        return lines;
    }
    list(
        &mut lines,
        t.format("restart.services", &count(&restart.services)),
        &restart.services,
    );
    list(
        &mut lines,
        t.format("restart.containers", &count(&restart.containers)),
        &restart.containers,
    );
    list(
        &mut lines,
        t.format("restart.sessions", &count(&restart.sessions)),
        &restart.sessions,
    );
    lines.push(Line::new(Kind::Gap, ""));
    if restart.services.is_empty() && restart.containers.is_empty() && restart.sessions.is_empty() {
        lines.push(Line::new(Kind::Body, t.get("restart.no_services")));
    } else {
        lines.push(Line::new(Kind::Muted, t.get("restart.manual")));
    }
    lines
}
