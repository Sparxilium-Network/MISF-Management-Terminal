//! Connections lost while an upgrade is running.

use crate::support::{Apk, assert_failed, names, start};
use misf_sshtest::Account;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_cut_connection_fails_the_run_while_root_s_apk_finishes_the_upgrade() {
    let lab = start(Account::Root, false, &["misf-ok", "misf-slow"]).await;
    lab.container.sh("echo 8 > /run/misf-slow.seconds").await;
    let (upgrade, _) = tokio::join!(lab.run_for("upgrade", 60), async {
        lab.container
            .wait_for("test -e /run/misf-slow.started", 60)
            .await;
        lab.relay.cut();
    });
    let upgrade = upgrade.expect("a closed connection ends the run at once");
    // GAP 5: as on DNF, the error does not say an upgrade was under way, and what the
    // upgrade had printed so far is not saved.
    assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");

    // Without a terminal nothing hangs up on apk: it finishes the upgrade.
    lab.container.wait_until_apk_idle(60).await;
    assert!(lab.container.try_sh("test -e /run/misf-slow.done").await.0);
    assert_eq!(lab.container.apk("misf-ok").await, "2.0");
    assert_eq!(lab.container.apk("misf-slow").await, "2.0");

    lab.relay.restore();
    let check = lab.run("check").await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert!(check.packages.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_cut_connection_with_a_pty_ends_apk_and_the_whole_upgrade_is_offered_again() {
    for account in [Account::Admin, Account::Root] {
        let lab = start(account, true, &["misf-ok", "misf-slow"]).await;
        lab.container.sh("echo 8 > /run/misf-slow.seconds").await;
        let (upgrade, _) = tokio::join!(lab.run_for("upgrade", 60), async {
            lab.container
                .wait_for("test -e /run/misf-slow.started", 60)
                .await;
            lab.relay.cut();
        });
        let upgrade = upgrade.expect("a closed connection ends the run at once");
        assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");

        // With a terminal the hang-up reaches apk, root's too, and it dies with the
        // package script still running. GAP 3: nothing records that the first package's
        // script had already run; apk committed nothing, so both are pending again and
        // the next upgrade runs every script a second time.
        lab.container.wait_until_apk_idle(60).await;
        assert!(!lab.container.try_sh("test -e /run/misf-slow.done").await.0);
        assert_eq!(lab.container.apk("misf-ok").await, "1.0");
        assert_eq!(lab.container.apk("misf-slow").await, "1.0");
        lab.relay.restore();
        let check = lab.run("check").await;
        assert_eq!(names(&check.packages), ["misf-ok", "misf-slow"]);
        lab.container.sh("echo 1 > /run/misf-slow.seconds").await;
        let again = lab.run("upgrade").await;
        assert_eq!(again.status, "success", "{}", again.error);
        assert_eq!(lab.container.apk("misf-slow").await, "2.0");
    }
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
    // GAP 5: the same bare error as a closed connection, with the upgrade's output lost.
    assert_failed(&upgrade, "SSH 連線結束但沒有命令退出碼");
    // The host finished long before MISF gave up.
    assert_eq!(lab.container.apk("misf-ok").await, "2.0");
    assert_eq!(lab.container.apk("misf-slow").await, "2.0");
}
