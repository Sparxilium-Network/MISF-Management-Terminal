//! Transactions that stop half-way or never start: failing scriptlets, packages that
//! cannot be installed, a broken repository, a killed dnf, a lock someone else holds.

use crate::support::{Rpm, assert_failed, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_failing_post_script_is_only_a_line_in_the_output() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-fail"]).await;
    let upgrade = lab.run("upgrade").await;
    // GAP 1: rpm treats a failed %post as a warning and dnf exits 0, so the run is saved
    // as a success; only the transcript says the package's script did not finish.
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-fail", "misf-ok"]);
    assert!(
        (upgrade.output).contains("Error in POSTIN scriptlet in rpm package misf-fail"),
        "{}",
        upgrade.output
    );
    assert!(
        upgrade
            .output
            .contains("misf-fail: the post script is about to fail")
    );
    assert_eq!(lab.container.rpm("misf-fail").await, "2.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_failing_pre_script_fails_the_upgrade_and_the_package_is_offered_again() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = start(account, pty, &["misf-ok", "misf-prefail"]).await;
        let upgrade = lab.run("upgrade").await;
        assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
        assert!(upgrade.error.contains("遠端可能已部分升級"));
        assert!(
            upgrade.output.contains("Error: Transaction failed"),
            "{}",
            upgrade.output
        );
        assert!(
            upgrade
                .output
                .contains("misf-prefail: the pre script is about to fail")
        );
        // Half-way: the other package went through, this one stayed as it was.
        assert_eq!(lab.container.rpm("misf-ok").await, "2.0");
        assert_eq!(lab.container.rpm("misf-prefail").await, "1.0");
        // Unlike a half-configured APT package, it is still listed as pending.
        let check = lab.run("check").await;
        assert_eq!(check.status, "success", "{}", check.error);
        assert_eq!(names(&check.packages), ["misf-prefail"]);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn one_package_that_cannot_be_installed_blocks_every_upgrade() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-broken"]).await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        (upgrade.output).contains("nothing provides misf-nowhere needed by misf-broken"),
        "{}",
        upgrade.output
    );
    // Enterprise Linux sets `best`, so dnf refuses the whole transaction rather than
    // skip the one package: the healthy update is not installed either.
    assert_eq!(lab.container.rpm("misf-ok").await, "1.0");
    assert_eq!(lab.container.rpm("misf-broken").await, "1.0");
    let check = lab.run("check").await;
    assert_eq!(names(&check.packages), ["misf-broken", "misf-ok"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_broken_repository_stops_before_anything_is_listed_or_installed() {
    let lab = start(Account::Admin, true, &["misf-ok"]).await;
    lab.container
        .sh("printf '%s\\n' '[misf-missing]' 'name=missing' 'baseurl=file:///opt/misf/missing' 'enabled=1' 'gpgcheck=0' > /etc/yum.repos.d/missing.repo")
        .await;
    for action in ["check", "upgrade"] {
        let result = lab.run(action).await;
        assert_failed(&result, "更新套件索引失敗");
        assert!(
            (result.output).contains("Failed to download metadata for repo 'misf-missing'"),
            "{}",
            result.output
        );
        assert!(!result.output.contains("待更新套件"), "{}", result.output);
    }
    assert_eq!(lab.container.rpm("misf-ok").await, "1.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_repository_dnf_may_skip_is_missed_by_the_check_and_fails_a_good_upgrade() {
    let lab = start(Account::Admin, true, &["misf-ok"]).await;
    lab.container
        .sh("printf '%s\\n' '[misf-missing]' 'name=missing' 'baseurl=file:///opt/misf/missing' 'enabled=1' 'gpgcheck=0' 'skip_if_unavailable=1' > /etc/yum.repos.d/missing.repo")
        .await;
    // GAP 2: `dnf -q makecache` says nothing about a repository it skipped, so the check
    // succeeds on what the other repositories offer and never mentions the missing one.
    let check = lab.run("check").await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert_eq!(names(&check.packages), ["misf-ok"]);
    assert!(!check.output.contains("misf-missing"), "{}", check.output);

    // GAP 3: the upgrade does print dnf's "Error:" line about that repository. dnf goes
    // on, installs the update and exits 0, but the line makes MISF save a failure.
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 0）");
    assert!(
        upgrade
            .output
            .contains("Ignoring repositories: misf-missing")
    );
    assert!(upgrade.output.contains("Complete!"));
    assert_eq!(lab.container.rpm("misf-ok").await, "2.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_package_file_that_cannot_be_read_fails_the_upgrade_and_changes_nothing() {
    // Gone after the metadata listed it, as when a mirror is mid-sync.
    let lab = start(Account::Admin, true, &["misf-ok", "misf-eline"]).await;
    lab.container
        .sh("rm /opt/misf/repo/misf-ok-2.0-1.noarch.rpm")
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        upgrade.output.contains("incorrect checksum"),
        "{}",
        upgrade.output
    );
    // Nothing of the transaction ran, so the package that was fine is not installed either.
    assert_eq!(lab.container.rpm("misf-ok").await, "1.0");
    assert_eq!(lab.container.rpm("misf-eline").await, "1.0");

    // Cut short, as when a download breaks off.
    let lab = start(Account::Root, false, &["misf-ok"]).await;
    lab.container
        .sh("cd /opt/misf/repo && head -c 300 misf-ok-2.0-1.noarch.rpm > part && mv part misf-ok-2.0-1.noarch.rpm")
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        upgrade.output.contains("incorrect checksum"),
        "{}",
        upgrade.output
    );
    assert_eq!(lab.container.rpm("misf-ok").await, "1.0");
    assert_eq!(names(&lab.run("check").await.packages), ["misf-ok"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn dnf_killed_half_way_leaves_a_duplicate_that_is_not_seen_again() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
    let (upgrade, _) = tokio::join!(lab.run("upgrade"), async {
        lab.container
            .wait_for("test -e /run/misf-slow.started", 60)
            .await;
        // As the kernel's out-of-memory killer would.
        lab.container.sh("pkill -9 -f '(^|/)dnf[0-9-]* '").await;
    });
    // GAP 4: the error names the signal but not that an upgrade was cut off, and what the
    // upgrade had printed so far is not saved.
    assert_failed(&upgrade, "遠端命令被訊號中止");
    assert!(!upgrade.error.contains("升級"), "{}", upgrade.error);
    assert!(
        !upgrade.output.contains("DNF upgrade"),
        "{}",
        upgrade.output
    );
    // Both versions of the package are now installed, and the other was never reached.
    assert_eq!(lab.container.rpm("misf-slow").await, "1.0 2.0");
    assert_eq!(lab.container.rpm("misf-ok").await, "1.0");

    // GAP 5: the next upgrade installs what is left and succeeds; nothing looks at the
    // duplicate, which stays until someone repairs it by hand.
    let again = lab.run("upgrade").await;
    assert_eq!(again.status, "success", "{}", again.error);
    assert_eq!(names(&again.upgraded), ["misf-ok"]);
    assert!(again.packages.is_empty());
    assert_eq!(lab.container.rpm("misf-slow").await, "1.0 2.0");
    let (healthy, problems) = lab.container.try_sh("dnf -q check 2>&1").await;
    assert!(!healthy && problems.contains("misf-slow"), "{problems}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn an_upgrade_waits_for_another_dnf_to_release_the_lock() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
    lab.container.sh("echo 10 > /run/misf-slow.seconds").await;
    // Someone else, an automatic update say, is busy with one package.
    lab.container.spawn("dnf -y upgrade misf-slow").await;
    lab.container
        .wait_for("test -e /run/misf-slow.started", 60)
        .await;
    // dnf queues behind the other process instead of failing, as APT does.
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert!(
        upgrade.output.contains("Waiting for process with pid"),
        "{}",
        upgrade.output
    );
    assert_eq!(names(&upgrade.upgraded), ["misf-ok"]);
    assert!(lab.container.try_sh("test -e /run/misf-slow.done").await.0);
    assert_eq!(lab.container.rpm("misf-ok").await, "2.0");
    assert_eq!(lab.container.rpm("misf-slow").await, "2.0");
}
