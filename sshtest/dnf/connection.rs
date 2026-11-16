//! Connections lost while an upgrade is running.

use crate::support::{Rpm, assert_failed, start};
use misf_sshtest::Account;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_cut_connection_fails_the_run_while_root_s_dnf_finishes_the_upgrade() {
    for pty in [true, false] {
        let lab = start(Account::Root, pty, &["misf-ok", "misf-slow"]).await;
        lab.container.sh("echo 8 > /run/misf-slow.seconds").await;
        let (upgrade, _) = tokio::join!(lab.run_for("upgrade", 60), async {
            lab.container
                .wait_for("test -e /run/misf-slow.started", 60)
                .await;
            lab.relay.cut();
        });
        let upgrade = upgrade.expect("a closed connection ends the run at once");
        // GAP 9: the error does not say an upgrade was under way or that the host may be
        // part-way through it, and what the upgrade had printed so far is not saved.
        assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");
        assert!(!upgrade.error.contains("升級"), "{}", upgrade.error);
        assert!(
            !upgrade.output.contains("DNF upgrade"),
            "{}",
            upgrade.output
        );

        // The host knows nothing of that: dnf carries on and finishes the transaction.
        assert!(lab.container.dnf_running().await);
        lab.container.wait_until_dnf_idle(60).await;
        assert!(lab.container.try_sh("test -e /run/misf-slow.done").await.0);
        assert_eq!(lab.container.rpm("misf-ok").await, "2.0");
        assert_eq!(lab.container.rpm("misf-slow").await, "2.0");

        // While the link is down nothing connects; once it is back the host checks clean.
        assert_eq!(lab.run("check").await.status, "failed");
        lab.relay.restore();
        let check = lab.run("check").await;
        assert_eq!(check.status, "success", "{}", check.error);
        assert!(check.packages.is_empty());
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

    // GAP 10: sudo passes the hang-up on. The package's script dies in the middle of its
    // work, yet rpm counts the package as installed and dnf completes the transaction:
    // both packages are at the new version and nothing records the unfinished script.
    lab.container.wait_until_dnf_idle(60).await;
    assert!(!lab.container.try_sh("test -e /run/misf-slow.done").await.0);
    assert_eq!(lab.container.rpm("misf-ok").await, "2.0");
    assert_eq!(lab.container.rpm("misf-slow").await, "2.0");
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
    let lab = start(Account::Root, false, &["misf-ok", "misf-slow"]).await;
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
    // GAP 9: the same bare error as a closed connection, with the upgrade's output lost.
    assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");
    // The host finished long before MISF gave up.
    assert!(!lab.container.dnf_running().await);
    assert_eq!(lab.container.rpm("misf-ok").await, "2.0");
    assert_eq!(lab.container.rpm("misf-slow").await, "2.0");
}
