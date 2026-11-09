//! Tests of the window state, and of the Slint pages through the testing backend.

use super::{
    App, AppWindow, BatchState, HostState, SettingsState, SettingsWindow,
    app::{SettingsPage, VaultMode, View},
    build,
    format::{connection_rows, copied_commands},
    host_page::Advanced,
};
use crate::{
    i18n::{Language, Translator},
    preferences::{BatchDetail, BatchOrder, UnlockMethod},
    worker::{Event, Worker},
};
use i_slint_backend_testing::ElementHandle;
use misf_core::{
    audit::AuditLog,
    model::{CheckResult, ConnectionTest, Host, UpgradePlan},
    package_manager,
    runtime::RuntimePaths,
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model};
use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

/// An app with no background runtime, hardware authenticator, or user profile: requests
/// land in the returned receiver and events are sent through the sender.
fn test_app(allow_passkey: bool) -> (App, Receiver<Value>, Sender<Event>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let paths = RuntimePaths {
        store: directory.path().join("hosts.db"),
        debug: true,
        debug_allow_passkey: allow_passkey,
    };
    let audit = AuditLog::new(directory.path()).unwrap();
    let (requests, commands) = mpsc::channel();
    let (events, results) = mpsc::channel();
    let app = App::new(
        paths,
        audit,
        Worker {
            requests,
            events: results,
            decisions: tokio::sync::mpsc::unbounded_channel().0,
        },
    );
    assert_eq!(commands.recv().unwrap()["action"], "storage_status");
    (app, commands, events, directory)
}

/// An unlocked vault, as the worker reports it.
fn unlocked(app: &mut App) {
    app.encrypted = true;
    app.locked = false;
}

fn host(id: &str, manager: &str, fingerprint: &str) -> Host {
    Host {
        id: id.into(),
        name: format!("host-{id}"),
        address: "127.0.0.1".into(),
        username: "operator".into(),
        os: "Linux".into(),
        distro: if manager.is_empty() { "" } else { "Debian 13" }.into(),
        package_manager: manager.into(),
        fingerprint: fingerprint.into(),
        ..Default::default()
    }
}

#[test]
fn copied_commands_are_one_paragraph_each_and_unchanged() {
    assert_eq!(
        copied_commands(&["apt-get update", "sh -c 'echo a; echo b'"]),
        "apt-get update\n\nsh -c 'echo a; echo b'\n"
    );
}

#[test]
fn connection_rows_explain_aead_post_quantum_and_unreached_steps() {
    let t = Translator(Language::ZhTw);
    let value = |rows: &[(&str, String)], key: &str| {
        rows.iter().find(|(k, _)| *k == key).unwrap().1.clone()
    };
    let test = ConnectionTest {
        status: "success".into(),
        kex: "mlkem768x25519-sha256".into(),
        cipher: "chacha20-poly1305@openssh.com".into(),
        client_mac: "none".into(),
        server_mac: "none".into(),
        strict_kex: true,
        auth_method: "publickey".into(),
        client_key_algorithm: "ecdsa-sha2-nistp521".into(),
        client_key_fingerprint: "SHA256:abc".into(),
        package_managers: vec!["dnf".into(), "yum".into()],
        ..Default::default()
    };
    let rows = connection_rows(t, &test);
    assert!(value(&rows, t.get("conn.kex")).contains(t.get("conn.post_quantum")));
    assert_eq!(value(&rows, t.get("conn.mac")), t.get("conn.mac_aead"));
    assert_eq!(value(&rows, t.get("conn.strict")), t.get("conn.strict_yes"));
    assert!(value(&rows, t.get("conn.auth")).contains("ecdsa-sha2-nistp521"));
    assert_eq!(value(&rows, t.get("conn.managers")), "DNF、YUM");
    // A test that failed before key exchange shows dashes, not "not strict".
    let failed = ConnectionTest {
        status: "failed".into(),
        ..Default::default()
    };
    let rows = connection_rows(t, &failed);
    assert!(rows.iter().all(|(_, value)| value == "—"), "{rows:?}");
}

#[test]
fn plaintext_prompt_reappears_after_unlock_and_schema_upgrade_waits_for_consent() {
    let (mut app, commands, _events, directory) = test_app(false);
    let full_path = directory.path().join("hosts.db").display().to_string();
    let mut status = json!({"encrypted":false, "locked":true, "plaintext_exists":true, "plaintext_path":full_path});
    // No vault yet: setup opens at once, and no host is ever read unencrypted.
    app.received("storage_status", status.clone());
    assert!(!app.plaintext_delete_prompt);
    assert_eq!(app.vault_dialog.as_ref().unwrap().mode, VaultMode::Create);
    assert!(commands.try_recv().is_err());
    app.vault_dialog = None;
    status["encrypted"] = json!(true);
    app.received("storage_status", status.clone());
    assert!(!app.plaintext_delete_prompt);
    assert_eq!(app.vault_dialog.as_ref().unwrap().mode, VaultMode::Unlock);
    status["locked"] = json!(false);
    for action in ["vault_create", "vault_unlock", "vault_unlock_passkey"] {
        app.received(action, status.clone());
        assert!(app.plaintext_delete_prompt);
        assert!(app.vault_dialog.is_none());
        assert_eq!(commands.recv().unwrap()["action"], "list");
        app.busy = false;
        app.respond_plaintext_delete(false);
        assert!(!app.plaintext_delete_prompt);
        assert!(commands.try_recv().is_err());
    }
    app.check_plaintext_copy();
    app.respond_plaintext_delete(true);
    assert_eq!(
        commands.recv().unwrap(),
        json!({"action":"vault_delete_plaintext", "path":full_path, "confirmed":true})
    );
    status["plaintext_exists"] = json!(false);
    app.received("vault_delete_plaintext", status.clone());
    assert!(!app.plaintext_delete_prompt);
    app.received("vault_unlock", status.clone());
    assert!(!app.plaintext_delete_prompt);
    assert_eq!(commands.recv().unwrap()["action"], "list");
    // An older vault database: no hosts are requested until the upgrade is accepted.
    let mut pending = status.clone();
    pending["locked"] = json!(true);
    pending["pending_upgrade"] = json!(1);
    app.received("vault_unlock", pending.clone());
    assert_eq!(app.schema_upgrade_prompt, Some(1));
    assert!(commands.try_recv().is_err());
    app.received("vault_upgrade_cancel", status.clone());
    assert_eq!(app.schema_upgrade_prompt, None);
    assert_eq!(app.vault_dialog.as_ref().unwrap().mode, VaultMode::Unlock);
    app.received("vault_unlock_passkey", pending);
    app.received("vault_upgrade", status);
    assert_eq!(app.schema_upgrade_prompt, None);
    assert_eq!(commands.recv().unwrap()["action"], "list");
}

#[test]
fn preferred_passkey_starts_once_when_the_vault_is_locked_at_launch() {
    let (mut app, commands, _events, _directory) = test_app(true);
    app.preferences.unlock_method = UnlockMethod::SecurityKey;
    let locked = json!({"encrypted":true, "locked":true});
    app.received("storage_status", locked.clone());
    let dialog = app.vault_dialog.as_ref().unwrap();
    assert_eq!(dialog.mode, VaultMode::Passkey);
    assert!(!dialog.hello);
    if cfg!(target_os = "windows") {
        let request = commands.recv().unwrap();
        assert_eq!(request["action"], "vault_unlock_passkey");
        assert_eq!(request["hello"], false);
    }
    // Locking again later shows the plain dialog instead of prompting by surprise.
    app.busy = false;
    app.received("storage_status", locked);
    assert_eq!(app.vault_dialog.as_ref().unwrap().mode, VaultMode::Unlock);
    assert!(commands.try_recv().is_err());
}

#[test]
fn vault_passwords_are_checked_before_anything_is_sent() {
    let (mut app, commands, _events, _directory) = test_app(false);
    app.open_vault_dialog(VaultMode::Create);
    let dialog = app.vault_dialog.as_mut().unwrap();
    dialog.password = "Short1".into();
    dialog.confirmation = "Other1".into();
    app.submit_vault();
    let t = app.t();
    assert_eq!(
        app.vault_dialog.as_ref().unwrap().error,
        t.get("vault.password_mismatch")
    );
    let dialog = app.vault_dialog.as_mut().unwrap();
    dialog.confirmation = "Short1".into();
    app.submit_vault();
    assert!(app.vault_dialog.as_ref().unwrap().error.contains('8'));
    assert!(commands.try_recv().is_err());
    let dialog = app.vault_dialog.as_mut().unwrap();
    dialog.password = "long enough 42".into();
    dialog.confirmation = "long enough 42".into();
    app.submit_vault();
    let request = commands.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(request["action"], "vault_create");
    assert_eq!(request["passphrase"], "long enough 42");
    // The secret leaves the dialog once it is sent.
    assert!(app.vault_dialog.as_ref().unwrap().password.is_empty());
    assert!(app.vault_dialog.as_ref().unwrap().confirmation.is_empty());
}

#[test]
fn session_log_records_the_run_without_any_secret() {
    let (mut app, _commands, events, _directory) = test_app(false);
    app.send(json!({"action":"vault_unlock","passphrase":"unlock-secret"}));
    let list = json!({"hosts":[{"id":"h","name":"web","password":"ssh-secret",
        "sudo_password":"sudo-secret","private_key":"key-body"}],"keys":[],"groups":[]});
    events
        .send(Event::Progress("正在解鎖 LUKSbox…".into()))
        .unwrap();
    events
        .send(Event::Done {
            action: "list".into(),
            result: Ok(list),
        })
        .unwrap();
    events
        .send(Event::Done {
            action: "check".into(),
            result: Err("無法連線".into()),
        })
        .unwrap();
    app.process_events();

    // A batch opens a titled section, and each host gets its own framed block.
    app.hosts = vec![Host {
        id: "u1".into(),
        name: "UBUNTU1".into(),
        address: "10.0.0.5".into(),
        port: 22,
        username: "deploy".into(),
        password: "ssh-secret".into(),
        package_manager: "apt".into(),
        ..Default::default()
    }];
    app.batch_manager = "apt".into();
    app.send(json!({"action":"batch","operation":"upgrade","ids":["u1"]}));
    events
        .send(Event::BatchHost {
            id: "u1".into(),
            result: CheckResult {
                operation: "upgrade".into(),
                status: "success".into(),
                output: "Setting up openssl (3.0.17) ...\n".into(),
                ..Default::default()
            },
        })
        .unwrap();
    app.process_events();
    // When the batch ends, hosts with kept-back packages are reported for dist-upgrade.
    let kept = json!(CheckResult {
        operation: "upgrade".into(),
        status: "success".into(),
        kept_back: vec!["linux-generic".into()],
        plan: Some(UpgradePlan {
            install: vec![misf_core::model::Package {
                name: "linux-image-7.0.0-43-generic".into(),
                candidate_version: "7.0.0-43.43".into(),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    });
    events
        .send(Event::Done {
            action: "batch".into(),
            result: Ok(json!({"results":[{"id":"u1","result":kept}],"skipped":[]})),
        })
        .unwrap();
    app.process_events();
    assert_eq!(app.kept_back_report.len(), 1);
    assert_eq!(app.kept_back_report[0].packages, ["linux-generic"]);
    assert_eq!(
        app.kept_back_report[0].plan.as_ref().unwrap().install[0].name,
        "linux-image-7.0.0-43-generic"
    );

    let text = app.session_log.render();
    for secret in ["unlock-secret", "ssh-secret", "sudo-secret", "key-body"] {
        assert!(!text.contains(secret), "{secret}");
    }
    for expected in [
        r#"[REQUEST] vault_unlock {"action":"vault_unlock","passphrase":"[已遮蔽]"}"#,
        "[PROGRESS] 正在解鎖 LUKSbox…",
        r#""name":"web""#,
        "[RESULT] check failed",
        "[ERROR] 無法連線",
        "\n* 使用者 進行批次 APT 升級（1 台）  (",
        r#"[REQUEST] batch {"action":"batch","ids":["u1"],"operation":"upgrade"}"#,
        "\n***********\n* UBUNTU1 *\n***********\n主機 ID: u1\n連線: deploy@10.0.0.5:22\n",
        "遠端原始輸出:\nSetting up openssl (3.0.17) ...\n",
        "\n******************\n* END OF UBUNTU1 *\n******************\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{}", text.as_str());
    }
}

#[test]
fn batch_selection_keeps_only_eligible_hosts_and_upgrade_waits_for_confirmation() {
    let (mut app, commands, _events, _directory) = test_app(false);
    let mut busy = host("busy", "apt", "SHA256:busy-fingerprint");
    busy.last_check = Some(CheckResult {
        status: "success".into(),
        packages: vec![Default::default(); 3],
        ..Default::default()
    });
    app.batch_selected = ["trusted", "untrusted", "rocky", "undetected", "gone"]
        .map(String::from)
        .into();
    app.received(
        "list",
        json!({"hosts":[
            host("trusted", "apt", "SHA256:trusted-fingerprint"),
            host("untrusted", "apt", ""),
            host("rocky", "dnf", "SHA256:rocky-fingerprint"),
            host("undetected", "", "SHA256:undetected-fingerprint"),
            busy,
        ]}),
    );
    // APT has the most ready hosts, so only same-manager hosts stay selected.
    assert_eq!(app.batch_manager, "apt");
    assert_eq!(app.batch_selected, ["trusted".to_owned()].into());
    // Hosts that cannot run are never selected by a click.
    app.toggle_batch_host("untrusted");
    app.toggle_batch_host("rocky");
    assert_eq!(app.batch_selected.len(), 1);
    app.received(
        "batch",
        json!({"results":[{"id":"trusted","result":{
            "operation":"upgrade","status":"success",
            "upgraded":[{"name":"openssl","current_version":"3.0.16","candidate_version":"3.0.17","architecture":"amd64","repository":"stable"}]
        }}]}),
    );
    assert_eq!(
        app.hosts[0].last_check.as_ref().unwrap().upgraded[0].name,
        "openssl"
    );
    assert_eq!(
        app.status,
        app.t().format(
            "status.batch_done",
            &[("succeeded", "1".into()), ("failed", "0".into())]
        )
    );
    app.busy = false;
    app.batch_request(package_manager::Mode::Upgrade);
    let confirm = app.upgrade_confirm.as_ref().unwrap();
    assert!(confirm.batch);
    assert_eq!(confirm.ids, ["trusted"]);
    assert!(commands.try_recv().is_err());
    app.confirm_upgrade();
    let request = commands.recv().unwrap();
    assert_eq!(request["action"], "batch");
    assert_eq!(request["operation"], "upgrade");
}

#[test]
fn batch_table_follows_the_chosen_order() {
    let (mut app, _commands, _events, _directory) = test_app(false);
    let checked = |id: &str, group: &str, packages: usize, at: &str| {
        let mut host = host(id, "apt", "SHA256:x");
        host.name = id.into();
        host.group_id = group.into();
        host.last_check = Some(CheckResult {
            status: "success".into(),
            checked_at: at.into(),
            packages: vec![Default::default(); packages],
            ..Default::default()
        });
        host
    };
    app.hosts = vec![
        checked("delta", "", 1, "2026-01-02T00:00:00Z"),
        checked("bravo", "g1", 5, "2026-01-01T00:00:00Z"),
        checked("alpha", "g1", 0, "2026-01-04T00:00:00Z"),
        checked("charlie", "g2", 3, "2026-01-03T00:00:00Z"),
    ];
    app.groups =
        serde_json::from_value(json!([{"id":"g1","name":"First"},{"id":"g2","name":"Second"}]))
            .unwrap();
    let names = |app: &App| -> Vec<String> {
        let (order, _) = app.batch_order(app.t());
        order.iter().map(|&i| app.hosts[i].name.clone()).collect()
    };
    app.preferences.batch_order = BatchOrder::Pending;
    assert_eq!(names(&app), ["bravo", "charlie", "delta", "alpha"]);
    app.preferences.batch_order = BatchOrder::Recent;
    assert_eq!(names(&app), ["alpha", "charlie", "delta", "bravo"]);
    app.preferences.batch_order = BatchOrder::Name;
    assert_eq!(names(&app), ["alpha", "bravo", "charlie", "delta"]);
    app.preferences.batch_order = BatchOrder::Sidebar;
    let (_, headings) = app.batch_order(app.t());
    let headings: Vec<(usize, &str)> = headings
        .iter()
        .map(|(at, name, _)| (*at, name.as_str()))
        .collect();
    let ungrouped = app.t().get("group.ungrouped");
    assert_eq!(headings, [(0, "First"), (2, "Second"), (3, ungrouped)]);
    assert_eq!(names(&app), ["bravo", "alpha", "charlie", "delta"]);
    // A heading's checkbox selects its ready hosts, then none.
    app.prune_batch();
    app.toggle_batch_group(&["bravo".into(), "alpha".into()]);
    assert_eq!(app.batch_selected.len(), 2);
    app.toggle_batch_group(&["bravo".into(), "alpha".into()]);
    assert!(app.batch_selected.is_empty());
    // Dragging a column edge moves width between its neighbours only, never below the floor.
    let before = app.preferences.batch_columns.weights;
    app.drag_column(1, 40.0, 800.0);
    let after = app.preferences.batch_columns.weights;
    let visible = app.preferences.batch_columns.visible();
    let (left, right) = (visible[0].0, visible[1].0);
    assert!(after[left] > before[left] && after[right] < before[right]);
    assert!((after.iter().sum::<f32>() - before.iter().sum::<f32>()).abs() < 1e-3);
    app.drag_column(1, 10_000.0, 800.0);
    let total: f32 = visible
        .iter()
        .map(|(i, _)| app.preferences.batch_columns.weights[*i])
        .sum();
    assert!(app.preferences.batch_columns.weights[right] * 800.0 / total >= 51.9);
}

#[test]
fn dragging_a_host_or_group_in_the_sidebar_saves_the_new_arrangement() {
    let (mut app, commands, _events, _directory) = test_app(false);
    unlocked(&mut app);
    let host = |id: &str, group: &str| {
        json!({"id": id, "name": id, "address": "127.0.0.1", "port": 22,
            "username": "ops", "os": "Linux", "group_id": group})
    };
    let list = json!({
        "hosts": [host("alpha", ""), host("bravo", "g1"), host("charlie", "g1")],
        "groups": [{"id": "g1", "name": "First"}, {"id": "g2", "name": "Second"}],
    });
    let row = |app: &App, id: &str| {
        let row = app.sidebar_rows().into_iter().find(|r| r.id == id).unwrap();
        (row.y, row.height)
    };
    let placement = |request: &Value| -> Vec<(String, String)> {
        request["hosts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| {
                (
                    h["id"].as_str().unwrap().into(),
                    h["group_id"].as_str().unwrap().into(),
                )
            })
            .collect()
    };

    // alpha onto the lower half of charlie: last in the first group.
    app.received("list", list.clone());
    let (y, height) = row(&app, "charlie");
    app.drag_to(false, "alpha", y + height * 0.75, true);
    assert!(
        app.sidebar_rows()
            .iter()
            .any(|r| r.id == "alpha" && r.dragged)
    );
    app.end_drag();
    let request = commands.try_recv().expect("no arrange request");
    assert_eq!(request["action"], "arrange");
    assert_eq!(
        placement(&request),
        [("bravo", "g1"), ("charlie", "g1"), ("alpha", "g1")].map(|(h, g)| (h.into(), g.into()))
    );
    assert_eq!(request["groups"], json!(["g1", "g2"]));
    assert!(app.hosts.iter().all(|h| h.group_id == "g1"));

    // The second group's heading above the first one's: the groups swap.
    app.busy = false;
    app.received("list", list.clone());
    let (y, _) = row(&app, "g1");
    app.drag_to(true, "g2", y + 2.0, true);
    app.end_drag();
    let request = commands.try_recv().expect("no arrange request");
    assert_eq!(request["groups"], json!(["g2", "g1"]));
    assert_eq!(placement(&request)[0], ("alpha".into(), String::new()));

    // Dropping a host back where it was, or outside the list, saves nothing.
    app.busy = false;
    app.received("list", list.clone());
    let (y, height) = row(&app, "bravo");
    app.drag_to(false, "bravo", y + height / 2.0, true);
    app.end_drag();
    let (y, _) = row(&app, "charlie");
    app.drag_to(false, "alpha", y, false);
    app.end_drag();
    assert!(commands.try_recv().is_err());

    // The ungrouped heading moves like a group, as a display preference.
    let (y, _) = row(&app, "g1");
    app.drag_to(true, "", y + 2.0, true);
    app.end_drag();
    assert_eq!(app.preferences.ungrouped_at, Some(0));
    assert!(commands.try_recv().is_err());
    let rows = app.sidebar_rows();
    let ungrouped = rows
        .iter()
        .position(|r| r.kind == 0 && r.id.is_empty())
        .unwrap();
    let first = rows
        .iter()
        .position(|r| r.kind == 0 && r.id == "g1")
        .unwrap();
    assert!(ungrouped < first);

    // Collapsing the ungrouped heading hides its hosts; grouped hosts are indented.
    app.toggle_group("");
    assert!(app.preferences.ungrouped_collapsed);
    let rows = app.sidebar_rows();
    assert!(rows.iter().all(|r| r.id != "alpha"));
    let bravo = rows.iter().find(|r| r.id == "bravo").unwrap();
    assert!(bravo.indent > 0.0);
}

#[test]
fn idle_vault_locks_after_the_chosen_minutes_but_not_while_working() {
    let (mut app, commands, _events, _directory) = test_app(false);
    let start = Instant::now();
    let minute = Duration::from_secs(60);
    app.busy = false;
    unlocked(&mut app);
    app.hosts = vec![host("a", "apt", "")];
    app.advanced_open = true;
    app.last_activity = start;
    // Never: nothing happens however long it is idle.
    app.preferences.auto_lock_minutes = None;
    assert_eq!(app.auto_lock(start + 600 * minute, false), None);
    app.preferences.auto_lock_minutes = Some(5);
    app.last_activity = start;
    assert_eq!(app.auto_lock(start + 4 * minute, false), Some(minute));
    // Input restarts the count.
    assert_eq!(app.auto_lock(start + 4 * minute, true), Some(5 * minute));
    assert_eq!(app.auto_lock(start + 8 * minute, false), Some(minute));
    // Running work restarts it too.
    app.busy = true;
    assert_eq!(app.auto_lock(start + 20 * minute, false), None);
    app.busy = false;
    assert_eq!(app.auto_lock(start + 24 * minute, false), Some(minute));
    assert!(commands.try_recv().is_err());
    assert_eq!(app.hosts.len(), 1);
    assert_eq!(app.auto_lock(start + 25 * minute, false), None);
    assert_eq!(commands.try_recv().unwrap()["action"], "vault_lock");
    assert!(app.hosts.is_empty());
    assert!(!app.advanced_open);
    assert!(app.busy);
    // The first input after a long idle still locks rather than restarting the count.
    app.busy = false;
    app.last_activity = start;
    assert_eq!(app.auto_lock(start + 60 * minute, true), None);
    assert_eq!(commands.try_recv().unwrap()["action"], "vault_lock");
    // A locked vault is never locked again.
    app.busy = false;
    app.locked = true;
    assert_eq!(app.auto_lock(start + 90 * minute, false), None);
    assert!(commands.try_recv().is_err());
}

#[test]
fn settings_tree_groups_preference_pages_under_a_branch() {
    let (mut app, _commands, _events, _directory) = test_app(false);
    let t = app.t();
    let labels = |app: &App| -> Vec<(String, i32)> {
        app.settings_tree()
            .iter()
            .map(|r| (r.label.to_string(), r.level))
            .collect()
    };
    let tree = labels(&app);
    let branch = tree.iter().position(|(_, level)| *level == 1).unwrap();
    assert_eq!(tree[branch].0, t.get("settings.nav_preferences"));
    // The branch's pages follow it, one level in.
    for (offset, key) in [
        "settings.nav_appearance",
        "settings.nav_security",
        "settings.nav_batch",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(tree[branch + 1 + offset], (t.get(key).to_owned(), 2));
    }
    app.settings_page = SettingsPage::Security;
    assert_eq!(
        app.settings_title(),
        format!(
            "{}  ›  {}",
            t.get("settings.nav_preferences"),
            t.get("settings.nav_security")
        )
    );
    // Inside the branch its row collapses and expands it.
    app.settings_branch();
    assert!(!app.preferences_expanded);
    assert!(labels(&app).iter().all(|(_, level)| *level != 2));
    app.settings_branch();
    assert!(app.preferences_expanded);
    // From another page it opens the branch's first page.
    app.settings_page = SettingsPage::About;
    app.preferences_expanded = false;
    app.settings_branch();
    assert_eq!(app.settings_page, SettingsPage::Appearance);
    assert!(app.preferences_expanded);
    // Preferences apply and save at once.
    app.set_preference("auto_lock", 1);
    assert_eq!(
        app.preferences.auto_lock_minutes,
        Some(crate::preferences::AUTO_LOCK_DEFAULT)
    );
    app.set_preference("auto_lock_minutes", 5000);
    assert_eq!(
        app.preferences.auto_lock_minutes,
        Some(crate::preferences::AUTO_LOCK_MAX)
    );
    app.set_preference("language", Language::EnUs.index());
    assert_eq!(app.preferences.language, Language::EnUs);
    let saved = crate::preferences::Preferences::load(&app.settings_path).unwrap();
    assert_eq!(saved.language, Language::EnUs);
}

#[test]
fn advanced_menu_runs_only_what_the_host_supports() {
    let (mut app, _commands, _events, _directory) = test_app(false);
    unlocked(&mut app);
    app.hosts = vec![
        host("rocky", "dnf", "SHA256:x"),
        host("deb", "apt", "SHA256:y"),
    ];
    app.selected = "rocky".into();
    app.advanced_open = true;
    // dist-upgrade and reboot are APT only: choosing them keeps the menu open, untouched.
    for item in [Advanced::DistUpgrade, Advanced::Reboot] {
        app.choose_advanced(item);
        assert!(app.advanced_open);
        assert!(app.upgrade_confirm.is_none() && app.reboot_confirm.is_none());
    }
    app.choose_advanced(Advanced::Upgrade);
    assert!(!app.advanced_open);
    assert_eq!(
        app.upgrade_confirm.as_ref().unwrap().mode,
        package_manager::Mode::Upgrade
    );
    app.upgrade_confirm = None;
    // A reboot asks twice: the risks with an acknowledgement, then a final yes.
    app.selected = "deb".into();
    app.choose_advanced(Advanced::Reboot);
    app.reboot_step(true);
    assert!(!app.reboot_confirm.as_ref().unwrap().final_step);
    app.reboot_confirm.as_mut().unwrap().acknowledged = true;
    app.reboot_step(true);
    assert!(app.reboot_confirm.as_ref().unwrap().final_step);
    app.reboot_step(false);
    assert!(!app.reboot_confirm.as_ref().unwrap().final_step);
}

/// Every page through the Slint testing backend, in one test because the backend is
/// shared by the whole process.
#[test]
fn pages_respond_to_clicks() {
    i_slint_backend_testing::init_no_event_loop();
    let (mut app, commands, _events, _directory) = test_app(false);
    unlocked(&mut app);
    app.busy = false;
    let mut web = host("web", "apt", "SHA256:x");
    web.name = "web-01".into();
    let mut lab = host("lab", "dnf", "SHA256:y");
    lab.name = "lab-01".into();
    app.hosts = vec![web, lab];
    app.selected = "web".into();
    let (ctx, main, settings) = build(app, true).unwrap();
    main.show().unwrap();
    let t = ctx.t();
    let first = |root: &AppWindow, label: &str| {
        ElementHandle::find_by_accessible_label(root, label)
            .next()
            .unwrap_or_else(|| panic!("{label} not shown"))
    };

    // The advanced menu opens from its button; its items carry hints with the commands.
    first(&main, t.get("advanced.button")).invoke_accessible_default_action();
    assert!(ctx.app.borrow().advanced_open);
    let state = main.global::<HostState>();
    assert!(state.get_dist_enabled() && state.get_reboot_enabled());
    assert!(state.get_dist_hint().copy.contains("dist-upgrade"));
    first(&main, t.get("apt.dist_upgrade")).invoke_accessible_default_action();
    assert!(!ctx.app.borrow().advanced_open);
    assert_eq!(
        ctx.app.borrow().upgrade_confirm.as_ref().unwrap().mode,
        package_manager::Mode::DistUpgrade
    );
    ctx.update(|app| app.upgrade_confirm = None);

    // A double-click on a host in the batch table opens its page.
    ctx.update(|app| {
        app.set_view(View::Batch);
        app.preferences.batch_detail = BatchDetail::Popup;
    });
    let rows = main.global::<BatchState>().get_rows();
    assert!(rows.iter().any(|r| r.id == "lab"));
    let lab = ElementHandle::find_by_accessible_label(&main, "lab-01")
        // The sidebar shows the name too; the table is right of it.
        .find(|e| e.size().width > 0.0 && e.absolute_position().x > 300.0)
        .expect("lab-01 not in the table");
    let at = lab.absolute_position();
    let size = lab.size();
    let position = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
    let button = slint::platform::PointerEventButton::Left;
    main.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
    for _ in 0..2 {
        main.window()
            .dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
        main.window()
            .dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
    }
    let app = ctx.app.borrow();
    assert_eq!(app.selected, "lab");
    assert_eq!(app.view, View::Host);
    drop(app);

    // The settings tree opens pages and collapses the Preferences branch.
    ctx.update(|app| {
        app.settings_page = SettingsPage::Vault;
        app.toggle_settings();
    });
    let in_settings =
        |label: &str| ElementHandle::find_by_accessible_label(&settings, label).next();
    in_settings(t.get("settings.nav_security"))
        .unwrap()
        .invoke_accessible_default_action();
    assert_eq!(ctx.app.borrow().settings_page, SettingsPage::Security);
    assert_eq!(
        settings.global::<SettingsState>().get_title().as_str(),
        ctx.app.borrow().settings_title()
    );
    in_settings(t.get("settings.nav_preferences"))
        .unwrap()
        .invoke_accessible_default_action();
    assert!(!ctx.app.borrow().preferences_expanded);
    assert!(in_settings(t.get("settings.nav_batch")).is_none());
    in_settings(t.get("settings.nav_preferences"))
        .unwrap()
        .invoke_accessible_default_action();
    assert!(in_settings(t.get("settings.nav_batch")).is_some());
    assert!(commands.try_recv().is_err());
    let _: &SettingsWindow = &settings;
}
