//! Upgrades that stop half-way or never start: failing package scripts, packages that
//! cannot be installed, an unreadable database, missing tools, a killed pacman, a lock
//! someone else holds or left behind.

use crate::support::{Pacman, assert_failed, names, start};
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
        // pacman prints one `error:` line for the script and still exits 0; MISF reads
        // the line, saves a failure and says the host may be part-way.
        assert_failed(&upgrade, "套件升級失敗（退出碼 0）");
        assert!(upgrade.error.contains("遠端可能已部分升級"));
        assert!(
            (upgrade.output).contains("error: command failed to execute correctly"),
            "{}",
            upgrade.output
        );
        assert!(upgrade.output.contains("script is about to fail"));
        // Even a failing pre_upgrade does not stop pacman installing the package, so
        // both are at 2.0 and the next check finds nothing left to do.
        assert_eq!(lab.container.pacman("misf-ok").await, "2.0");
        assert_eq!(lab.container.pacman(failing).await, "2.0");
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
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        (upgrade.output)
            .contains("unable to satisfy dependency 'misf-nowhere' required by misf-broken"),
        "{}",
        upgrade.output
    );
    // pacman offers to skip the package; --noconfirm answers that with its default, no.
    assert!(upgrade.output.contains("skip the above package"));
    assert_eq!(lab.container.pacman("misf-ok").await, "1.0");
    assert_eq!(lab.container.pacman("misf-broken").await, "1.0");
    assert!(!lab.container.locked().await);
    let check = lab.run("check").await;
    assert_eq!(names(&check.packages), ["misf-broken", "misf-ok"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_broken_repository_stops_before_anything_is_listed_or_installed() {
    let lab = start(Account::Admin, true, &["misf-ok"]).await;
    lab.container
        .sh("printf '%s\\n' '[misf-missing]' 'Server = file:///opt/misf/missing' >> /etc/pacman.conf")
        .await;
    for action in ["check", "upgrade"] {
        let result = lab.run(action).await;
        assert_failed(&result, "更新套件索引失敗（退出碼 1）");
        assert!(
            result.output.contains("==> ERROR: Cannot fetch updates"),
            "{}",
            result.output
        );
        assert!(!result.output.contains("待更新套件"), "{}", result.output);
        // GAP 1: checkupdates hides which repository failed and why, and MISF adds its
        // hint to install pacman-contrib and fakeroot, though both are there.
        assert!(!result.output.contains("misf-missing"), "{}", result.output);
        assert!(result.error.contains("fakeroot"), "{}", result.error);
    }
    assert_eq!(lab.container.pacman("misf-ok").await, "1.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_host_without_checkupdates_or_fakeroot_cannot_be_checked_and_is_told_why() {
    let lab = start(Account::Admin, true, &["misf-ok"]).await;
    lab.container
        .sh("mv /usr/bin/fakeroot /usr/bin/fakeroot.off")
        .await;
    let check = lab.run("check").await;
    assert_failed(&check, "更新套件索引失敗（退出碼 1）");
    assert!(
        check
            .error
            .contains("pacman-contrib（checkupdates）與 fakeroot")
    );
    assert!(
        check.output.contains("Cannot find the fakeroot binary"),
        "{}",
        check.output
    );

    lab.container
        .sh("mv /usr/bin/fakeroot.off /usr/bin/fakeroot && mv /usr/bin/checkupdates /usr/bin/checkupdates.off")
        .await;
    // Neither a check nor an upgrade gets past the refresh, so nothing is installed.
    for action in ["check", "upgrade"] {
        let result = lab.run(action).await;
        assert_failed(&result, "更新套件索引失敗（退出碼 127）");
        assert!(
            result
                .error
                .contains("pacman-contrib（checkupdates）與 fakeroot")
        );
    }
    assert_eq!(lab.container.pacman("misf-ok").await, "1.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_package_file_that_cannot_be_read_fails_the_upgrade_and_changes_nothing() {
    // Gone after the database listed it, as when a mirror is mid-sync.
    let lab = start(Account::Admin, true, &["misf-ok", "misf-eline"]).await;
    lab.container
        .sh("rm /opt/misf/repo/misf-ok-2.0-1-any.pkg.tar.zst")
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        (upgrade.output).contains("error: failed retrieving file 'misf-ok-2.0-1-any.pkg.tar.zst'"),
        "{}",
        upgrade.output
    );
    assert!(
        upgrade
            .output
            .contains("Errors occurred, no packages were upgraded.")
    );
    // pacman fetches everything first, so the package that was fine is not installed.
    assert_eq!(lab.container.pacman("misf-ok").await, "1.0");
    assert_eq!(lab.container.pacman("misf-eline").await, "1.0");
    assert!(!lab.container.locked().await);

    // Cut short, as when a download breaks off.
    let lab = start(Account::Root, false, &["misf-ok"]).await;
    lab.container
        .sh("cd /opt/misf/repo && head -c 300 misf-ok-2.0-1-any.pkg.tar.zst > part && mv part misf-ok-2.0-1-any.pkg.tar.zst")
        .await;
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        upgrade.output.contains("invalid or corrupted package"),
        "{}",
        upgrade.output
    );
    assert_eq!(lab.container.pacman("misf-ok").await, "1.0");
    assert_eq!(names(&lab.run("check").await.packages), ["misf-ok"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn pacman_killed_half_way_leaves_its_lock_and_later_upgrades_cannot_start() {
    let lab = start(Account::Admin, true, &["misf-slow"]).await;
    let (upgrade, _) = tokio::join!(lab.run("upgrade"), async {
        lab.container
            .wait_for("test -e /run/misf-slow.started", 60)
            .await;
        // As the kernel's out-of-memory killer would.
        lab.container.sh("pkill -9 -x pacman").await;
    });
    // GAP 2: the error names the signal but not that an upgrade was cut off, and what the
    // upgrade had printed so far is not saved.
    assert_failed(&upgrade, "遠端命令被訊號中止");
    assert!(!upgrade.error.contains("升級"), "{}", upgrade.error);
    assert!(
        !upgrade.output.contains("pacman upgrade"),
        "{}",
        upgrade.output
    );
    // The package's files were already in place, so it counts as upgraded though its
    // script never finished, and the next check finds nothing to do.
    assert_eq!(lab.container.pacman("misf-slow").await, "2.0");
    assert!(lab.run("check").await.packages.is_empty());

    // GAP 3: pacman's lock file outlived it. Nothing shows until the next update
    // arrives; then every upgrade fails until someone deletes the file by hand.
    assert!(lab.container.locked().await && !lab.container.pacman_running().await);
    lab.container.sh("misf-scenario misf-ok").await;
    let blocked = lab.run("upgrade").await;
    assert_failed(&blocked, "套件升級失敗（退出碼 1）");
    assert!(
        blocked.output.contains("unable to lock database"),
        "{}",
        blocked.output
    );
    assert_eq!(lab.container.pacman("misf-ok").await, "1.0");
    lab.container.sh("rm /var/lib/pacman/db.lck").await;
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(lab.container.pacman("misf-ok").await, "2.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn an_upgrade_while_another_pacman_holds_the_lock_fails_and_works_afterwards() {
    let lab = start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
    lab.container.sh("echo 10 > /run/misf-slow.seconds").await;
    // Someone else is busy with one package.
    lab.container
        .spawn("pacman -Sy --noconfirm misf-slow")
        .await;
    lab.container
        .wait_for("test -e /run/misf-slow.started", 60)
        .await;
    // The check needs no lock: it reads its private copy.
    let check = lab.run("check").await;
    assert_eq!(names(&check.packages), ["misf-ok"]);
    // The upgrade does, and pacman does not wait for it.
    let upgrade = lab.run("upgrade").await;
    assert_failed(&upgrade, "套件升級失敗（退出碼 1）");
    assert!(
        upgrade.output.contains("unable to lock database"),
        "{}",
        upgrade.output
    );
    assert_eq!(lab.container.pacman("misf-ok").await, "1.0");

    lab.container.wait_until_pacman_idle(60).await;
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-ok"]);
    assert_eq!(lab.container.pacman("misf-slow").await, "2.0");
}
