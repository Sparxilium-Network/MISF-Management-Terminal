//! Upgrades that go as planned.

use crate::support::{Pacman, assert_plain, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_check_leaves_the_system_databases_alone_and_the_upgrade_goes_through_sudo() {
    let packages = ["misf-ok", "misf-conf", "misf-dep", "misf-newdep"];
    let lab = start(Account::Admin, true, &packages).await;
    let check = lab.run("check").await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert_eq!(check.package_manager, "pacman");
    // An installed Arch has no version; the container image carries its build date.
    assert!(check.distro.starts_with("Arch Linux"), "{}", check.distro);
    // The package the upgrade will pull in as a dependency is not an upgrade itself.
    assert_eq!(names(&check.packages), ["misf-conf", "misf-dep", "misf-ok"]);
    let pending = &check.packages[2];
    assert_eq!(
        (
            pending.current_version.as_str(),
            pending.candidate_version.as_str()
        ),
        ("1.0-1", "2.0-1")
    );
    // checkupdates lists neither an architecture nor a repository.
    assert!(pending.architecture.is_empty() && pending.repository.is_empty());
    assert!(check.restart.is_none());

    // The check used no sudo and synced only checkupdates' private copy: the system's
    // own databases still know nothing of the updates, so no partial upgrade can follow.
    let screen = lab.screen.text();
    assert!(
        !(screen.lines()).any(|line| line.starts_with("$ ") && line.contains("sudo")),
        "{screen}"
    );
    assert!(!screen.contains("pacman -S"), "{screen}");
    assert_eq!(lab.container.sh("pacman -Qu || true").await.trim(), "");
    assert_eq!(lab.container.pacman("misf-ok").await, "1.0");

    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(
        names(&upgrade.upgraded),
        ["misf-conf", "misf-dep", "misf-ok"]
    );
    assert!(upgrade.packages.is_empty());
    for package in ["misf-ok", "misf-conf", "misf-dep"] {
        assert_eq!(lab.container.pacman(package).await, "2.0");
    }
    // The upgrade installs new dependencies; only the transcript names them.
    assert_eq!(lab.container.pacman("misf-newdep").await, "1.0");
    assert!(upgrade.output.contains("installing misf-newdep..."));
    // The admin's config file stays; the package's waits beside it, and nobody is asked.
    assert_eq!(
        lab.container.sh("cat /etc/misf-conf.conf").await.trim(),
        "setting = changed-by-the-admin"
    );
    assert_eq!(
        (lab.container.sh("cat /etc/misf-conf.conf.pacnew").await).trim(),
        "setting = packaged-v2"
    );
    assert!(upgrade.output.contains("misf-ok: configured"));
    assert!(!lab.container.locked().await);
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
    assert_eq!(lab.container.pacman("misf-ok").await, "2.0");
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
async fn passwordless_sudo_upgrades_without_a_terminal() {
    let lab = start(Account::Deploy, false, &["misf-ok"]).await;
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-ok"]);
    assert_eq!(lab.container.pacman("misf-ok").await, "2.0");
    assert!(!lab.container.locked().await);
}
