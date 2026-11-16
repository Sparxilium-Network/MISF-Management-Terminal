//! APT against a real Debian/Ubuntu container: upgrades that fail half-way, lost
//! connections, output no log should trust, and package scripts that wait for a person.
//!
//! Every test starts its own container, so they can run in any order, in parallel and
//! again and again. They need Docker and are ignored by default:
//!
//! ```text
//! cargo test -p misf-sshtest -- --ignored
//! ```
//!
//! One test takes about four minutes and only runs when `MISF_SSHTEST_SLOW=1` is set.
//!
//! The assertions describe what MISF does today. Where that is a gap rather than the
//! wanted behaviour, the test says `GAP` and `sshtest/README.md` lists it; fixing the gap
//! in core means changing that assertion with it.

use misf_core::model::{CheckResult, Package};
use misf_sshtest::{Account, Lab};
use std::time::{Duration, Instant};

fn names(packages: &[Package]) -> Vec<&str> {
    packages
        .iter()
        .map(|package| package.name.as_str())
        .collect()
}

/// Saved output and live lines are plain text: no control characters but the newline,
/// and none of the host's secrets.
fn assert_plain(lab: &Lab, text: &str) {
    let host = lab.host();
    assert!(
        !text.chars().any(|c| c.is_control() && c != '\n'),
        "control characters left in:\n{text:?}"
    );
    assert!(!text.contains(&host.password), "the password leaked");
    // The live view also shows each command (`$ …`), and sudo commands name the marker.
    assert!(
        !(text.lines())
            .any(|line| !line.starts_with("$ ") && line.contains("__MISF_SUDO_PROMPT__")),
        "the sudo prompt marker was left in the output"
    );
}

/// A failed run names the step and never claims packages.
fn assert_failed(result: &CheckResult, error: &str) {
    assert_eq!(result.status, "failed", "{}", result.output);
    assert!(result.error.contains(error), "{}", result.error);
    assert!(result.packages.is_empty() && result.upgraded.is_empty());
    assert!(result.restart.is_none());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_clean_upgrade_through_sudo_reports_packages_and_the_reboot_request() {
    let lab = Lab::start(Account::Admin, true, &["misf-ok"]).await;
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
async fn a_failing_package_script_fails_the_upgrade_and_leaves_it_half_configured() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = Lab::start(account, pty, &["misf-ok", "misf-fail"]).await;
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
async fn questions_apt_would_ask_are_answered_without_anyone_there() {
    let lab = Lab::start(Account::Admin, true, &["misf-debconf", "misf-conf"]).await;
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
    let lab = Lab::start(
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_broken_repository_stops_before_anything_is_listed_or_installed() {
    let lab = Lab::start(Account::Admin, true, &["misf-ok"]).await;
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
    let lab = Lab::start(Account::Admin, true, &["misf-ok", "misf-eline"]).await;
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
    let lab = Lab::start(Account::Root, false, &["misf-ok"]).await;
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
async fn garbage_from_a_package_script_is_cleaned_and_does_not_fail_the_upgrade() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = Lab::start(account, pty, &["misf-noise"]).await;
        let upgrade = lab.run("upgrade").await;
        assert_eq!(upgrade.status, "success", "{}", upgrade.error);
        assert_eq!(names(&upgrade.upgraded), ["misf-noise"]);
        assert_plain(&lab, &upgrade.output);
        let output = &upgrade.output;
        // Colours and the window title are gone, redraws became lines, backspaces applied.
        assert!(
            output.contains("red text then after the title\n"),
            "{output}"
        );
        assert!(output.contains("progress 10%\nprogress 50%\nprogress 100%\n"));
        assert!(output.contains("nul and bell: abc\n"));
        assert!(output.contains("backspaces: xyz\n"));
        assert!(output.contains("wide text: 中文輸出\n"));
        // Bytes that are not UTF-8 become replacement characters instead of an error.
        assert!(output.contains("invalid utf-8: \u{fffd}\u{fffd}\u{fffd} end\n"));
        // The saved output keeps the 100,000-character line whole.
        assert!(output.contains(&"x".repeat(100_000)));
        assert!(output.contains("misf-noise: after the long line\n"));

        // GAP 2: an escape sequence that never ends swallows the rest of that command's
        // saved output, here the script's last line and all that APT printed after it.
        assert!(output.contains("an escape that never ends: "));
        assert!(!output.contains("misf-noise: end"), "{output}");

        // The live view works line by line: the long line is cut and marked, and the
        // unfinished escape costs only its own line.
        let screen = lab.screen.text();
        assert_plain(&lab, &screen);
        assert!(screen.contains("（此行過長，已截斷）"));
        assert!(!screen.contains(&"x".repeat(9_000)));
        assert!(screen.contains("misf-noise: after the long line\n"));
        assert!(screen.contains("misf-noise: end\n"), "{screen}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_script_line_that_looks_like_an_apt_error_fails_an_upgrade_that_worked() {
    let lab = Lab::start(Account::Root, true, &["misf-eline"]).await;
    let upgrade = lab.run("upgrade").await;
    // GAP 3: APT exited 0 and the package is fully installed, but any line starting with
    // "E:" counts as an APT error, so the run is saved as failed.
    assert_failed(&upgrade, "套件升級失敗（退出碼 0）");
    assert!((upgrade.output).contains("E: this line is only text printed by a package script"));
    assert_eq!(lab.container.dpkg("misf-eline").await, "ii 2.0");
    assert!(lab.container.audit().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn root_ignores_a_package_script_that_prints_the_sudo_prompt_marker() {
    let lab = Lab::start(Account::Root, true, &["misf-marker"]).await;
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-marker"]);
    assert!(upgrade.output.contains("misf-marker says \n"));
    assert_plain(&lab, &upgrade.output);
    // The script was sent nothing.
    assert_eq!(lab.container.sh("cat /run/misf-marker.stdin").await, "");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn the_marker_after_sudo_took_the_password_reads_as_a_wrong_password() {
    for pty in [true, false] {
        let lab = Lab::start(Account::Admin, pty, &["misf-marker"]).await;
        let upgrade = lab.run("upgrade").await;
        // GAP 4: sudo accepted the password and APT is upgrading; the marker in a package
        // script's output then counts as a second prompt. MISF reports a wrong sudo
        // password, drops the connection and saves nothing of the upgrade step.
        assert_failed(&upgrade, "sudo 密碼錯誤或需要額外驗證");
        assert!(
            !upgrade.output.contains("APT upgrade"),
            "{}",
            upgrade.output
        );
        // The upgrade itself goes on without MISF and finishes.
        lab.container.wait_until_apt_idle(60).await;
        assert_eq!(lab.container.dpkg("misf-marker").await, "ii 2.0");
        // The password was sent once, to sudo, and never to the script.
        let received = lab.container.sh("cat /run/misf-marker.stdin").await;
        assert!(!received.contains(Account::Admin.password()), "{received}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn with_passwordless_sudo_the_marker_gets_the_password_typed_into_the_script() {
    for pty in [false, true] {
        let lab = Lab::start(Account::Deploy, pty, &["misf-marker"]).await;
        let upgrade = lab.run("upgrade").await;
        assert_eq!(upgrade.status, "success", "{}", upgrade.error);
        assert_eq!(lab.container.dpkg("misf-marker").await, "ii 2.0");
        // GAP 5 (documented in the user guide): sudo never asked, so the first marker in
        // the output is answered as if it were sudo's prompt, and the package script
        // reads the SSH password from its input.
        assert_eq!(
            lab.container.sh("cat /run/misf-marker.stdin").await.trim(),
            Account::Deploy.password(),
            "pty={pty}"
        );
        // It still never reaches the saved output or the live view.
        assert_plain(&lab, &upgrade.output);
        assert_plain(&lab, &lab.screen.text());
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_dialog_cannot_open_on_the_dumb_terminal_and_fails_at_once() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = Lab::start(account, pty, &["misf-dialog"]).await;
        let upgrade = lab
            .run_for("upgrade", 120)
            .await
            .expect("a dialog that cannot draw must not keep the upgrade waiting");
        // MISF asks for a "dumb" terminal (or none), which whiptail refuses.
        assert_failed(&upgrade, "套件升級失敗（退出碼 100）");
        assert!(
            (upgrade.output).contains("dpkg: error processing package misf-dialog (--configure):"),
            "{}",
            upgrade.output
        );
        assert_plain(&lab, &upgrade.output);
        assert_eq!(lab.container.dpkg("misf-dialog").await, "iF 2.0");
        assert!(!lab.container.apt_running().await);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_drawn_dialog_keeps_the_upgrade_waiting_and_outlives_the_stop() {
    // APT gives dpkg a terminal of its own, so the dialog is drawn with or without a PTY.
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = Lab::start(account, pty, &["misf-tui"]).await;
        // GAP 6: nobody can press the key, and MISF waits for its 30-minute limit. The
        // test stops after 15 seconds, which drops the connection as the Stop button does.
        let upgrade = lab.run_for("upgrade", 15).await;
        assert!(upgrade.is_none(), "the dialog was expected to block");
        // The live view shows the dialog as text, without its escape sequences.
        let screen = lab.screen.text();
        assert_plain(&lab, &screen);
        assert!(
            screen.contains("A package opened a full-screen dialog."),
            "{screen}"
        );
        assert!(screen.contains("<Ok>"));

        // GAP 7: the stop does not end the upgrade. apt-get keeps the dpkg lock and the
        // dialog keeps waiting on a terminal no one is attached to.
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(lab.container.apt_running().await);
        assert!(!lab.container.try_sh("test -e /run/misf-tui.status").await.0);
        assert_eq!(lab.container.dpkg("misf-tui").await, "iF 2.0");

        // GAP 1 again: with apt-get still stuck there, a check and an upgrade both report
        // success with nothing to do; the upgrade never runs APT, so never meets the lock.
        for action in ["check", "upgrade"] {
            let after = lab.run(action).await;
            assert_eq!(after.status, "success", "{}", after.error);
            assert!(after.packages.is_empty() && after.upgraded.is_empty());
        }
        assert!(lab.container.apt_running().await);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_script_waiting_for_an_answer_blocks_until_the_connection_goes() {
    // With a PTY the question waits on a terminal that never closes.
    let lab = Lab::start(Account::Admin, true, &["misf-prompt"]).await;
    // GAP 6: MISF sends neither an answer nor end-of-input, and waits for its limit.
    assert!(lab.run_for("upgrade", 15).await.is_none());
    let screen = lab.screen.text();
    assert_plain(&lab, &screen);
    assert!(
        lab.container
            .try_sh("test -e /run/misf-prompt.started")
            .await
            .0
    );
    // GAP 7: after the stop the script is still waiting and apt-get still holds the lock.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(lab.container.apt_running().await);
    assert!(
        !lab.container
            .try_sh("test -e /run/misf-prompt.answer")
            .await
            .0
    );
    assert_eq!(lab.container.dpkg("misf-prompt").await, "iF 2.0");

    // Without a PTY the wait is the same, but dropping the connection ends the script's
    // input: it reads an empty answer and the upgrade finishes by itself.
    let lab = Lab::start(Account::Root, false, &["misf-prompt"]).await;
    assert!(lab.run_for("upgrade", 15).await.is_none());
    lab.container.wait_until_apt_idle(60).await;
    assert_eq!(
        lab.container.sh("cat /run/misf-prompt.answer").await.trim(),
        ""
    );
    assert_eq!(lab.container.dpkg("misf-prompt").await, "ii 2.0");
    assert!(lab.container.audit().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_cut_connection_fails_the_run_while_the_host_finishes_the_upgrade() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = Lab::start(account, pty, &["misf-ok", "misf-slow"]).await;
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
        assert!(!upgrade.error.contains("dpkg"), "{}", upgrade.error);
        assert!(
            !upgrade.output.contains("APT upgrade"),
            "{}",
            upgrade.output
        );
        assert!(!upgrade.output.contains("misf-slow: working"));

        // The host knows nothing of that: apt-get carries on, with or without a PTY,
        // and finishes the upgrade.
        assert!(lab.container.apt_running().await);
        lab.container.wait_until_apt_idle(60).await;
        assert!(lab.container.try_sh("test -e /run/misf-slow.done").await.0);
        assert_eq!(lab.container.dpkg("misf-ok").await, "ii 2.0");
        assert_eq!(lab.container.dpkg("misf-slow").await, "ii 2.0");
        assert!(lab.container.audit().await.is_empty());

        // While the link is down nothing connects; once it is back the host checks clean.
        let down = lab.run("check").await;
        assert_eq!(down.status, "failed");
        lab.relay.restore();
        let check = lab.run("check").await;
        assert_eq!(check.status, "success", "{}", check.error);
        assert!(check.packages.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn dpkg_killed_half_way_fails_the_upgrade_and_the_damage_is_not_seen_again() {
    let lab = Lab::start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
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
    let lab = Lab::start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_silently_dead_link_is_given_up_on_after_minutes_of_keepalives() {
    if std::env::var("MISF_SSHTEST_SLOW").as_deref() != Ok("1") {
        eprintln!("skipped: takes about four minutes; set MISF_SSHTEST_SLOW=1 to run it");
        return;
    }
    let lab = Lab::start(Account::Admin, true, &["misf-ok", "misf-slow"]).await;
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
    assert!(
        !upgrade.output.contains("APT upgrade"),
        "{}",
        upgrade.output
    );
    // The host finished long before MISF gave up.
    assert!(!lab.container.apt_running().await);
    assert_eq!(lab.container.dpkg("misf-ok").await, "ii 2.0");
    assert_eq!(lab.container.dpkg("misf-slow").await, "ii 2.0");
}
