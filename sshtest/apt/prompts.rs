//! Package scripts that want a person: full-screen dialogs and questions on the terminal.

use crate::support::{Dpkg, assert_failed, assert_plain, start};
use misf_sshtest::Account;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_dialog_cannot_open_on_the_dumb_terminal_and_fails_at_once() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = start(account, pty, &["misf-dialog"]).await;
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
        let lab = start(account, pty, &["misf-tui"]).await;
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
    let lab = start(Account::Admin, true, &["misf-prompt"]).await;
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
    let lab = start(Account::Root, false, &["misf-prompt"]).await;
    assert!(lab.run_for("upgrade", 15).await.is_none());
    lab.container.wait_until_apt_idle(60).await;
    assert_eq!(
        lab.container.sh("cat /run/misf-prompt.answer").await.trim(),
        ""
    );
    assert_eq!(lab.container.dpkg("misf-prompt").await, "ii 2.0");
    assert!(lab.container.audit().await.is_empty());
}
