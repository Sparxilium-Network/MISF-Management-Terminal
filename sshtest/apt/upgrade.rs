//! Upgrades that go as planned, and the questions APT is told not to ask.

use crate::support::{Dpkg, assert_plain, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_clean_upgrade_through_sudo_reports_packages_and_the_reboot_request() {
    let lab = start(Account::Admin, true, &["misf-ok"]).await;
    let check = lab.run("check").await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert_eq!(names(&check.packages), ["misf-ok"]);
    let pending = &check.packages[0];
    assert_eq!(
        (
            pending.current_version.as_str(),
            pending.candidate_version.as_str(),
            pending.architecture.as_str(),
            pending.repository.as_str()
        ),
        ("1.0", "2.0", "all", "misf-test")
    );
    // The real needrestart answered in its batch format, and nothing asks for a reboot yet.
    let restart = check.restart.expect("the restart status was read");
    assert!(restart.needrestart && !restart.reboot_required);
    assert!(!restart.running_kernel.is_empty());
    assert_eq!(lab.container.dpkg("misf-ok").await, "ii 1.0");

    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-ok"]);
    assert!(upgrade.packages.is_empty());
    let restart = upgrade.restart.expect("the restart status was read");
    assert!(restart.reboot_required);
    assert_eq!(restart.reboot_packages, ["misf-ok"]);
    assert_eq!(lab.container.dpkg("misf-ok").await, "ii 2.0");
    assert_plain(&lab, &upgrade.output);
    assert_plain(&lab, &lab.screen.text());
    assert!(upgrade.output.contains("Setting up misf-ok (2.0) ..."));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn questions_apt_would_ask_are_answered_without_anyone_there() {
    let lab = start(Account::Admin, true, &["misf-debconf", "misf-conf"]).await;
    let upgrade = lab
        .run_for("upgrade", 120)
        .await
        .expect("no question may keep the upgrade waiting");
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-conf", "misf-debconf"]);
    // DEBIAN_FRONTEND=noninteractive reached debconf through sudo: the critical question
    // got its default instead of a dialog.
    assert_eq!(
        lab.container
            .sh("cat /run/misf-debconf.answer")
            .await
            .trim(),
        "default-answer"
    );
    // --force-confold: the admin's file stays, the package's waits beside it.
    assert_eq!(
        lab.container.sh("cat /etc/misf-conf.conf").await.trim(),
        "setting = changed-by-the-admin"
    );
    assert_eq!(
        (lab.container.sh("cat /etc/misf-conf.conf.dpkg-dist").await).trim(),
        "setting = packaged-v2"
    );
    assert!(
        upgrade
            .output
            .contains("Keeping old config file as default.")
    );
    assert_plain(&lab, &upgrade.output);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn kept_back_packages_get_a_simulated_plan_and_a_dist_upgrade_installs_them() {
    let lab = start(
        Account::Admin,
        true,
        &["misf-ok", "misf-kept", "misf-newdep"],
    )
    .await;
    let check = lab.run("check").await;
    assert_eq!(names(&check.packages), ["misf-kept", "misf-ok"]);
    // A check never reports a dist-upgrade, though its simulation prints the same lines.
    assert!(check.kept_back.is_empty() && check.plan.is_none());

    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-ok"]);
    assert_eq!(names(&upgrade.packages), ["misf-kept"]);
    assert_eq!(upgrade.kept_back, ["misf-kept"]);
    let plan = upgrade.plan.expect("the dist-upgrade was simulated");
    assert_eq!(names(&plan.install), ["misf-newdep"]);
    assert_eq!(names(&plan.upgrade), ["misf-kept"]);
    assert!(plan.remove.is_empty());
    assert_eq!(plan.upgrade[0].current_version, "1.0");
    assert_eq!(plan.upgrade[0].candidate_version, "2.0");
    // The simulation changed nothing.
    assert_eq!(lab.container.dpkg("misf-kept").await, "ii 1.0");

    let dist = lab.run("dist_upgrade").await;
    assert_eq!(dist.status, "success", "{}", dist.error);
    assert_eq!(names(&dist.upgraded), ["misf-kept"]);
    assert!(dist.packages.is_empty() && dist.kept_back.is_empty() && dist.plan.is_none());
    assert_eq!(lab.container.dpkg("misf-kept").await, "ii 2.0");
    assert_eq!(lab.container.dpkg("misf-newdep").await, "ii 1.0");
}
