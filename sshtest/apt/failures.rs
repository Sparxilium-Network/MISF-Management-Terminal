//! Upgrades that stop half-way or never start: failing scripts, a broken repository,
//! a killed dpkg, a lock someone else holds.

use crate::support::{Dpkg, assert_failed, assert_plain, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_failing_package_script_fails_the_upgrade_and_leaves_it_half_configured() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = start(account, pty, &["misf-ok", "misf-fail"]).await;
        let upgrade = lab.run("upgrade").await;
        assert_failed(&upgrade, "套件升級失敗（退出碼 100）");
        assert!(upgrade.error.contains("遠端可能已部分升級"));
        assert!(
            (upgrade.output).contains("E: Sub-process /usr/bin/dpkg returned an error code (1)"),
            "{}",
            upgrade.output
        );
        assert!(
            upgrade
                .output
                .contains("misf-fail: postinst is about to fail")
        );
        assert_plain(&lab, &upgrade.output);
        // Half-way: one package is done, the other is unpacked but not configured.
        assert_eq!(lab.container.dpkg("misf-ok").await, "ii 2.0");
        assert_eq!(lab.container.dpkg("misf-fail").await, "iF 2.0");

        // GAP 1: the half-configured package is no longer "upgradable", so the next check
        // and the next upgrade both succeed with nothing to do while dpkg is still broken.
        assert!(lab.container.audit().await.contains("misf-fail"));
        for action in ["check", "upgrade"] {
            let after = lab.run(action).await;
            assert_eq!(after.status, "success", "{}", after.error);
            assert!(after.packages.is_empty() && after.upgraded.is_empty());
        }
        assert_eq!(lab.container.dpkg("misf-fail").await, "iF 2.0");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_broken_repository_stops_before_anything_is_listed_or_installed() {
    let lab = start(Account::Admin, true, &["misf-ok"]).await;
    lab.container
        .sh("echo 'deb [trusted=yes] file:/opt/misf/missing ./' >> /etc/apt/sources.list")
        .await;
    for action in ["check", "upgrade"] {
        let result = lab.run(action).await;
        assert_failed(&result, "更新套件索引失敗");
        assert!(result.output.contains("Err:"), "{}", result.output);
        assert!(!result.output.contains("待更新套件"), "{}", result.output);
    }
    assert_eq!(lab.container.dpkg("misf-ok").await, "ii 1.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_package_file_that_cannot_be_fetched_fails_the_upgrade_and_changes_nothing() {
    // Gone after the index listed it, as when a mirror is mid-sync.
    let lab = start(Account::Admin, true, &["misf-ok", "misf-eline"]).await;
    lab.container
        .sh("rm /opt/misf/repo/misf-ok_2.0_all.deb")
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 100）");
    assert!(
        upgrade.output.contains("E: Failed to fetch"),
        "{}",
        upgrade.output
    );
    // APT fetches everything first, so the package it did fetch is not installed either.
    assert_eq!(lab.container.dpkg("misf-ok").await, "ii 1.0");
    assert_eq!(lab.container.dpkg("misf-eline").await, "ii 1.0");

    // Cut short, as when a download breaks off.
    let lab = start(Account::Root, false, &["misf-ok"]).await;
    lab.container
        .sh("cd /opt/misf/repo && head -c 300 misf-ok_2.0_all.deb > part && mv part misf-ok_2.0_all.deb")
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 100）");
    assert!(
        upgrade.output.contains("Hash Sum mismatch"),
        "{}",
        upgrade.output
    );
    assert_eq!(lab.container.dpkg("misf-ok").await, "ii 1.0");
    // Nothing was touched, so the package is still offered.
    assert_eq!(names(&lab.run("check").await.packages), ["misf-ok"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn dpkg_killed_half_way_fails_the_upgrade_and_the_damage_is_not_seen_again() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
    let (upgrade, _) = tokio::join!(lab.run("upgrade"), async {
        lab.container
            .wait_for("test -e /run/misf-slow.started", 60)
            .await;
        // As the kernel's out-of-memory killer would.
        lab.container.sh("pkill -9 -x dpkg").await;
    });
    assert_failed(&upgrade, "套件升級失敗（退出碼 100）");
    assert!(
        upgrade.output.contains("E: Sub-process /usr/bin/dpkg"),
        "{}",
        upgrade.output
    );
    assert!(upgrade.output.contains("misf-slow: working for"));
    assert_eq!(lab.container.dpkg("misf-slow").await, "iF 2.0");
    assert!(lab.container.audit().await.contains("misf-slow"));

    // GAP 1: nothing is "upgradable" any more, so MISF never runs APT again and never
    // sees its "dpkg was interrupted" error; the host looks up to date.
    for action in ["check", "upgrade"] {
        let after = lab.run(action).await;
        assert_eq!(after.status, "success", "{}", after.error);
        assert!(after.packages.is_empty() && after.upgraded.is_empty());
    }
    assert!(lab.container.audit().await.contains("misf-slow"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn an_upgrade_while_another_apt_holds_the_lock_fails_and_works_afterwards() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
    lab.container.sh("echo 10 > /run/misf-slow.seconds").await;
    // Someone else, an unattended upgrade say, is busy with one package.
    lab.container
        .spawn("apt-get -q update && apt-get -y install --only-upgrade misf-slow")
        .await;
    lab.container
        .wait_for("test -e /run/misf-slow.started", 60)
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 100）");
    assert!(
        upgrade.output.contains("Could not get lock"),
        "{}",
        upgrade.output
    );
    assert_eq!(lab.container.dpkg("misf-ok").await, "ii 1.0");

    lab.container.wait_until_apt_idle(60).await;
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-ok"]);
    assert_eq!(lab.container.dpkg("misf-ok").await, "ii 2.0");
    assert_eq!(lab.container.dpkg("misf-slow").await, "ii 2.0");
}
