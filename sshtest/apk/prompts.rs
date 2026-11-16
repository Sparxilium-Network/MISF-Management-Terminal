//! Package scripts that want a person: full-screen dialogs and questions on the terminal.

use crate::support::{Apk, assert_plain, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_dialog_cannot_open_without_a_terminal_type_and_the_upgrade_goes_on() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = start(account, pty, &["misf-dialog"]).await;
        let upgrade = lab
            .run_for("upgrade", 120)
            .await
            .expect("a dialog that cannot draw must not keep the upgrade waiting");
        assert_eq!(upgrade.status, "success", "{}", upgrade.error);
        // apk runs scripts with a clean environment: even with a PTY they have no TERM.
        assert!(
            upgrade
                .output
                .contains("TERM environment variable needs set."),
            "{}",
            upgrade.output
        );
        assert_plain(&lab, &upgrade.output);
        assert_eq!(lab.container.apk("misf-dialog").await, "2.0");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_dialog_drawn_on_the_terminal_keeps_the_upgrade_waiting_and_is_asked_again() {
    for account in [Account::Admin, Account::Root] {
        let lab = start(account, true, &["misf-tui"]).await;
        // GAP 4: the script went to the controlling terminal itself; nobody can press the
        // key, and MISF waits for its 30-minute limit. The test stops after 15 seconds,
        // which drops the connection as the Stop button does.
        assert!(lab.run_for("upgrade", 15).await.is_none());
        let screen = lab.screen.text();
        assert_plain(&lab, &screen);
        assert!(
            screen.contains("A package opened a full-screen dialog."),
            "{screen}"
        );
        // Losing the terminal ends apk before it records anything: the package is still
        // at its old version and still pending, so the next upgrade meets the same dialog.
        lab.container.wait_until_apk_idle(60).await;
        assert_eq!(lab.container.apk("misf-tui").await, "1.0");
        let check = lab.run("check").await;
        assert_eq!(names(&check.packages), ["misf-tui"]);
        assert!(lab.run_for("upgrade", 15).await.is_none());
    }

    // Without a PTY there is no terminal to draw on, and the script fails at once.
    let lab = start(Account::Root, false, &["misf-tui"]).await;
    let upgrade = lab
        .run_for("upgrade", 120)
        .await
        .expect("no terminal, no dialog");
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert!(upgrade.output.contains("/dev/tty"), "{}", upgrade.output);
    assert_eq!(lab.container.apk("misf-tui").await, "2.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_question_asked_on_the_terminal_keeps_the_upgrade_waiting() {
    let lab = start(Account::Admin, true, &["misf-tty"]).await;
    // GAP 4: with a PTY there is a terminal to ask on, and nobody to answer.
    assert!(lab.run_for("upgrade", 15).await.is_none());
    assert!(
        lab.container
            .try_sh("test -e /run/misf-tty.started")
            .await
            .0
    );
    lab.container.wait_until_apk_idle(60).await;
    assert_eq!(lab.container.apk("misf-tty").await, "1.0");

    // Without a PTY the script finds no terminal and uses its default.
    let lab = start(Account::Root, false, &["misf-tty"]).await;
    let upgrade = lab
        .run_for("upgrade", 120)
        .await
        .expect("no terminal, no question");
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(
        lab.container.sh("cat /run/misf-tty.answer").await.trim(),
        "no terminal"
    );
}
