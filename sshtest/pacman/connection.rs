//! Connections lost while an upgrade is running.

use crate::support::{Pacman, assert_failed, start};
use misf_sshtest::Account;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_cut_connection_with_a_pty_fails_the_run_while_root_s_pacman_finishes() {
    let lab = start(Account::Root, true, &["misf-ok", "misf-slow"]).await;
    lab.container.sh("echo 8 > /run/misf-slow.seconds").await;
    let (upgrade, _) = tokio::join!(lab.run_for("upgrade", 60), async {
        lab.container
            .wait_for("test -e /run/misf-slow.started", 60)
            .await;
        lab.relay.cut();
    });
    let upgrade = upgrade.expect("a closed connection ends the run at once");
    // GAP 8: the error does not say an upgrade was under way or that the host may be
    // part-way through it, and what the upgrade had printed so far is not saved.
    assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");
    assert!(!upgrade.error.contains("升級"), "{}", upgrade.error);
    assert!(
        !upgrade.output.contains("pacman upgrade"),
        "{}",
        upgrade.output
    );

    // The host knows nothing of that: pacman carries on, finishes and tidies up.
    assert!(lab.container.pacman_running().await);
    lab.container.wait_until_pacman_idle(60).await;
    assert!(lab.container.try_sh("test -e /run/misf-slow.done").await.0);
    assert_eq!(lab.container.pacman("misf-ok").await, "2.0");
    assert_eq!(lab.container.pacman("misf-slow").await, "2.0");
    assert!(!lab.container.locked().await);

    // While the link is down nothing connects; once it is back the host checks clean.
    assert_eq!(lab.run("check").await.status, "failed");
    lab.relay.restore();
    let check = lab.run("check").await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert!(check.packages.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_cut_connection_without_a_pty_leaves_pacman_s_lock_behind() {
    for account in [Account::Root, Account::Admin] {
        let lab = start(account, false, &["misf-slow"]).await;
        lab.container.sh("echo 8 > /run/misf-slow.seconds").await;
        let (upgrade, _) = tokio::join!(lab.run_for("upgrade", 60), async {
            lab.container
                .wait_for("test -e /run/misf-slow.started", 60)
                .await;
            lab.relay.cut();
        });
        let upgrade = upgrade.expect("a closed connection ends the run at once");
        assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");

        // The package script runs to its end and the package is installed.
        lab.container.wait_until_pacman_idle(60).await;
        assert!(lab.container.try_sh("test -e /run/misf-slow.done").await.0);
        assert_eq!(lab.container.pacman("misf-slow").await, "2.0");
        // GAP 3: pacman then dies writing to the closed connection, before it removes
        // its lock file. The host checks clean, and the next update cannot be installed
        // until someone deletes the file by hand.
        assert!(lab.container.locked().await);
        lab.relay.restore();
        let check = lab.run("check").await;
        assert_eq!(check.status, "success", "{}", check.error);
        assert!(check.packages.is_empty());
        lab.container.sh("misf-scenario misf-ok").await;
        let blocked = lab.run("upgrade").await;
        assert_failed(&blocked, "套件升級失敗（退出碼 1）");
        assert!(
            blocked.output.contains("unable to lock database"),
            "{}",
            blocked.output
        );
        assert_eq!(lab.container.pacman("misf-ok").await, "1.0");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_cut_connection_under_sudo_with_a_pty_cuts_the_running_script_short() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
    lab.container.sh("echo 8 > /run/misf-slow.seconds").await;
    let (upgrade, _) = tokio::join!(lab.run_for("upgrade", 60), async {
        lab.container
            .wait_for("test -e /run/misf-slow.started", 60)
            .await;
        lab.relay.cut();
    });
    let upgrade = upgrade.expect("a closed connection ends the run at once");
    assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");

    // GAP 9: sudo passes the hang-up on. The package's script dies in the middle of its
    // work, yet the package counts as installed and nothing records the unfinished
    // script. This time pacman does remove its lock.
    lab.container.wait_until_pacman_idle(60).await;
    assert!(!lab.container.try_sh("test -e /run/misf-slow.done").await.0);
    assert_eq!(lab.container.pacman("misf-slow").await, "2.0");
    assert!(!lab.container.locked().await);
    lab.relay.restore();
    let check = lab.run("check").await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert!(check.packages.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_silently_dead_link_is_given_up_on_after_minutes_of_keepalives() {
    if std::env::var("MISF_SSHTEST_SLOW").as_deref() != Ok("1") {
        eprintln!("skipped: takes about four minutes; set MISF_SSHTEST_SLOW=1 to run it");
        return;
    }
    let lab = start(Account::Root, true, &["misf-ok", "misf-slow"]).await;
    lab.container.sh("echo 8 > /run/misf-slow.seconds").await;
    let started = Instant::now();
    let (upgrade, _) = tokio::join!(lab.run_for("upgrade", 600), async {
        lab.container
            .wait_for("test -e /run/misf-slow.started", 60)
            .await;
        // Packets vanish and nobody closes anything, as when a route or a VPN dies.
        lab.relay.freeze();
    });
    let upgrade = upgrade.expect("unanswered keepalives end the run well inside ten minutes");
    // Six keepalives 30 seconds apart go unanswered before the session is dropped.
    let waited = started.elapsed();
    assert!(
        waited > Duration::from_secs(150),
        "gave up after {waited:?}"
    );
    // GAP 8: the same bare error as a closed connection, with the upgrade's output lost.
    assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");
    // The host finished long before MISF gave up.
    assert_eq!(lab.container.pacman("misf-ok").await, "2.0");
    assert_eq!(lab.container.pacman("misf-slow").await, "2.0");
}
