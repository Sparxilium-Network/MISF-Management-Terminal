//! Upgrades that go as planned.

use crate::support::{Apk, assert_plain, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_clean_upgrade_through_sudo_reports_packages_and_keeps_changed_config() {
    let packages = ["misf-ok", "misf-conf", "misf-dep", "misf-newdep"];
    let lab = start(Account::Admin, true, &packages).await;
    let check = lab.run("check").await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert_eq!(check.package_manager, "apk");
    assert!(check.distro.starts_with("Alpine Linux"), "{}", check.distro);
    // The package the upgrade will pull in as a dependency is not an upgrade itself.
    assert_eq!(names(&check.packages), ["misf-conf", "misf-dep", "misf-ok"]);
    let pending = &check.packages[2];
    assert_eq!(
        (
            pending.current_version.as_str(),
            pending.candidate_version.as_str()
        ),
        ("1.0-r0", "2.0-r0")
    );
    assert!(!pending.architecture.is_empty());
    // Restart status is read for APT only.
    assert!(check.restart.is_none());
    assert_eq!(lab.container.apk("misf-ok").await, "1.0");

    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(
        names(&upgrade.upgraded),
        ["misf-conf", "misf-dep", "misf-ok"]
    );
    assert!(upgrade.packages.is_empty());
    for package in ["misf-ok", "misf-conf", "misf-dep"] {
        assert_eq!(lab.container.apk(package).await, "2.0");
    }
    // The upgrade installs new dependencies; only the transcript names them.
    assert_eq!(lab.container.apk("misf-newdep").await, "1.0");
    assert!(upgrade.output.contains("Installing misf-newdep"));
    // The admin's config file stays; the package's waits beside it, and nobody is asked.
    assert_eq!(
        lab.container.sh("cat /etc/misf-conf.conf").await.trim(),
        "setting = changed-by-the-admin"
    );
    assert_eq!(
        (lab.container.sh("cat /etc/misf-conf.conf.apk-new").await).trim(),
        "setting = packaged-v2"
    );
    assert!(upgrade.output.contains("misf-ok: configured"));
    assert_plain(&lab, &upgrade.output);
    assert_plain(&lab, &lab.screen.text());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn root_upgrades_without_sudo_or_a_terminal_and_has_no_dist_upgrade() {
    let lab = start(Account::Root, false, &["misf-ok"]).await;
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-ok"]);
    assert_eq!(lab.container.apk("misf-ok").await, "2.0");
    let commands = lab.screen.text();
    assert!(
        !(commands.lines()).any(|line| line.starts_with("$ ") && line.contains("sudo")),
        "root's commands must not go through sudo"
    );

    // dist-upgrade is APT's; asking for it here fails before any package command runs.
    let before = lab.screen.text().matches("$ ").count();
    let dist = lab.run("dist_upgrade").await;
    assert_eq!(dist.status, "failed");
    assert!(dist.error.contains("只有 APT 支援"), "{}", dist.error);
    let commands = lab.screen.text().matches("$ ").count() - before;
    assert_eq!(commands, 1, "only the detection command may run");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn passwordless_sudo_lists_without_sudo_and_upgrades_through_it() {
    let lab = start(Account::Deploy, false, &["misf-ok"]).await;
    let check = lab.run("check").await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert_eq!(names(&check.packages), ["misf-ok"]);
    let screen = lab.screen.text();
    let commands: Vec<&str> = screen.lines().filter(|l| l.starts_with("$ ")).collect();
    // Refresh through sudo, listing as the account itself.
    assert!(
        commands
            .iter()
            .any(|c| c.contains("sudo") && c.contains("apk update"))
    );
    assert!(
        commands
            .iter()
            .any(|c| !c.contains("sudo") && c.contains("apk list"))
    );
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(lab.container.apk("misf-ok").await, "2.0");
}
