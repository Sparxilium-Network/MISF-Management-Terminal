//! Upgrades that stop half-way or never start: failing package scripts, packages that
//! cannot be installed, unreadable or unsigned indexes, a killed apk, a lock someone else
//! holds.

use crate::support::{Apk, assert_failed, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_failing_script_fails_the_run_though_every_package_went_in() {
    for (account, pty, failing) in [
        (Account::Admin, true, "misf-fail"),
        (Account::Root, false, "misf-prefail"),
    ] {
        let lab = start(account, pty, &["misf-ok", failing]).await;
        let upgrade = lab.run("upgrade").await;
        // apk reports the script with its own `ERROR:` line and exits 1, so MISF saves a
        // failure and says the host may be part-way.
        assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
        assert!(upgrade.error.contains("遠端可能已部分升級"));
        assert!(
            (upgrade.output).contains(&format!("ERROR: lib/apk/exec/{failing}-2.0-r0.")),
            "{}",
            upgrade.output
        );
        assert!(
            upgrade.output.contains("the pre script is about to fail")
                || upgrade.output.contains("the post script is about to fail")
        );
        // Even a failing pre-upgrade script does not stop apk installing the package, so
        // both are at 2.0 and the next check finds nothing left.
        assert_eq!(lab.container.apk("misf-ok").await, "2.0");
        assert_eq!(lab.container.apk(failing).await, "2.0");
        let check = lab.run("check").await;
        assert_eq!(check.status, "success", "{}", check.error);
        assert!(check.packages.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn one_package_that_cannot_be_installed_blocks_every_upgrade() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-broken"]).await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗");
    assert!(
        (upgrade.output).contains("misf-nowhere (no such package)"),
        "{}",
        upgrade.output
    );
    // apk refuses the whole upgrade rather than skip the one package.
    assert_eq!(lab.container.apk("misf-ok").await, "1.0");
    assert_eq!(lab.container.apk("misf-broken").await, "1.0");
    let check = lab.run("check").await;
    assert_eq!(names(&check.packages), ["misf-broken", "misf-ok"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_missing_index_stops_before_anything_is_listed_or_installed() {
    let lab = start(Account::Admin, true, &["misf-ok"]).await;
    // `apk update` only warns about it and still exits 0.
    lab.container
        .sh("echo /opt/misf/missing >> /etc/apk/repositories")
        .await;
    for action in ["check", "upgrade"] {
        let result = lab.run(action).await;
        assert_failed(&result, "更新套件索引失敗（退出碼 0）");
        assert!(
            (result.output).contains("WARNING: opening /opt/misf/missing"),
            "{}",
            result.output
        );
        assert!(!result.output.contains("待更新套件"), "{}", result.output);
    }
    assert_eq!(lab.container.apk("misf-ok").await, "1.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn an_index_without_a_trusted_signature_is_refused() {
    let lab = start(Account::Root, false, &["misf-ok"]).await;
    // Rebuilt and left unsigned, as a tampered or misconfigured mirror would serve it.
    lab.container
        .sh("cd /opt/misf/repo/$(apk --print-arch) && apk index --quiet --rewrite-arch $(apk --print-arch) -o APKINDEX.tar.gz *.apk 2>/dev/null")
        .await;
    let check = lab.run("check").await;
    assert_failed(&check, "更新套件索引失敗（退出碼 0）");
    assert!(
        check.output.contains("UNTRUSTED signature"),
        "{}",
        check.output
    );
    assert_eq!(lab.container.apk("misf-ok").await, "1.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn apk_killed_half_way_fails_the_run_and_the_next_upgrade_finishes() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
    let (upgrade, _) = tokio::join!(lab.run("upgrade"), async {
        lab.container
            .wait_for("test -e /run/misf-slow.started", 60)
            .await;
        // As the kernel's out-of-memory killer would.
        lab.container.sh("pkill -9 -x apk").await;
    });
    // GAP 6: as on DNF, the error names the signal but not that an upgrade was cut off.
    assert_failed(&upgrade, "遠端命令被訊號中止");
    assert!(!upgrade.error.contains("升級"), "{}", upgrade.error);
    assert!(
        !upgrade.output.contains("apk upgrade"),
        "{}",
        upgrade.output
    );
    // apk records what it installed only at the end, so nothing of this run counts:
    // both packages are still at their old version and both are offered again, the one
    // whose script had already finished included.
    lab.container.wait_until_apk_idle(60).await;
    assert_eq!(lab.container.apk("misf-ok").await, "1.0");
    assert_eq!(lab.container.apk("misf-slow").await, "1.0");
    let check = lab.run("check").await;
    assert_eq!(names(&check.packages), ["misf-ok", "misf-slow"]);
    lab.container.sh("echo 1 > /run/misf-slow.seconds").await;
    let again = lab.run("upgrade").await;
    assert_eq!(again.status, "success", "{}", again.error);
    assert!(again.packages.is_empty());
    assert_eq!(lab.container.apk("misf-ok").await, "2.0");
    assert_eq!(lab.container.apk("misf-slow").await, "2.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn while_another_apk_holds_the_lock_not_even_a_check_can_run() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
    lab.container.sh("echo 10 > /run/misf-slow.seconds").await;
    // Someone else is busy with one package.
    lab.container
        .spawn("apk upgrade --no-progress misf-slow")
        .await;
    lab.container
        .wait_for("test -e /run/misf-slow.started", 60)
        .await;
    // `apk update` takes the database lock and does not wait for it, so the refresh
    // fails and neither a check nor an upgrade gets any further.
    for action in ["check", "upgrade"] {
        let result = lab.run(action).await;
        assert_failed(&result, "更新套件索引失敗（退出碼 99）");
        assert!(
            result.output.contains("ERROR: Unable to lock database"),
            "{}",
            result.output
        );
    }
    assert_eq!(lab.container.apk("misf-ok").await, "1.0");

    // apk's lock is released with its process; nothing is left to clean up.
    lab.container.wait_until_apk_idle(60).await;
    assert_eq!(lab.container.apk("misf-slow").await, "2.0");
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-ok"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_package_file_that_cannot_be_read_fails_the_upgrade_after_the_others_went_in() {
    // Gone after the index listed it, as when a mirror is mid-sync.
    let lab = start(Account::Admin, true, &["misf-ok", "misf-eline"]).await;
    lab.container
        .sh("rm /opt/misf/repo/*/misf-ok-2.0-r0.apk")
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        (upgrade.output).contains("ERROR: misf-ok-2.0-r0: package mentioned in index not found"),
        "{}",
        upgrade.output
    );
    // apk fetches each package as it gets to it: unlike APT, DNF and pacman, the ones
    // before the missing file are already installed. The rest is offered again.
    assert_eq!(lab.container.apk("misf-eline").await, "2.0");
    assert_eq!(lab.container.apk("misf-ok").await, "1.0");
    assert_eq!(names(&lab.run("check").await.packages), ["misf-ok"]);

    // Cut short, as when a download breaks off.
    let lab = start(Account::Root, false, &["misf-ok"]).await;
    lab.container
        .sh("cd /opt/misf/repo/* && head -c 300 misf-ok-2.0-r0.apk > part && mv part misf-ok-2.0-r0.apk")
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        upgrade.output.contains("ERROR: misf-ok-2.0-r0:"),
        "{}",
        upgrade.output
    );
    assert_eq!(lab.container.apk("misf-ok").await, "1.0");
}
