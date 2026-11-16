//! Package scripts that want a person: full-screen dialogs and questions on the terminal.

use crate::support::{Rpm, assert_plain, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_dialog_cannot_open_on_the_dumb_terminal_and_the_upgrade_goes_on() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = start(account, pty, &["misf-dialog"]).await;
        let upgrade = lab
            .run_for("upgrade", 120)
            .await
            .expect("a dialog that cannot draw must not keep the upgrade waiting");
        // MISF asks for a "dumb" terminal (or none), which whiptail refuses. GAP 1: that
        // is a failed %post, which dnf and so MISF count as a success.
        assert_eq!(upgrade.status, "success", "{}", upgrade.error);
        assert!(
            (upgrade.output).contains("Error in POSTIN scriptlet in rpm package misf-dialog"),
            "{}",
            upgrade.output
        );
        assert_plain(&lab, &upgrade.output);
        assert_eq!(lab.container.rpm("misf-dialog").await, "2.0");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_question_on_the_scripts_input_gets_end_of_input_at_once() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = start(account, pty, &["misf-prompt"]).await;
        let upgrade = lab
            .run_for("upgrade", 120)
            .await
            .expect("rpm gives scriptlets no input to wait on");
        assert_eq!(upgrade.status, "success", "{}", upgrade.error);
        assert_eq!(names(&upgrade.upgraded), ["misf-prompt"]);
        // The script read an empty answer and carried on with its default.
        assert_eq!(
            lab.container.sh("cat /run/misf-prompt.answer").await.trim(),
            "answer="
        );
        assert!(upgrade.output.contains("misf-prompt: answered"));
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_dialog_drawn_on_the_terminal_keeps_the_upgrade_waiting() {
    for account in [Account::Admin, Account::Root] {
        let lab = start(account, true, &["misf-tui"]).await;
        // GAP 8: the script went to the controlling terminal itself; nobody can press the
        // key, and MISF waits for its 30-minute limit. The test stops after 15 seconds,
        // which drops the connection as the Stop button does.
        assert!(lab.run_for("upgrade", 15).await.is_none());
        // The live view shows the dialog as text, without its escape sequences.
        let screen = lab.screen.text();
        assert_plain(&lab, &screen);
        assert!(
            screen.contains("A package opened a full-screen dialog."),
            "{screen}"
        );
        assert!(screen.contains("<Ok>"));
        // Losing the terminal ends the dialog, and dnf finishes the transaction: the
        // stop leaves nothing running and nothing half-installed.
        lab.container.wait_until_dnf_idle(60).await;
        assert_eq!(lab.container.rpm("misf-tui").await, "2.0");
        let check = lab.run("check").await;
        assert_eq!(check.status, "success", "{}", check.error);
        assert!(check.packages.is_empty());
    }

    // Without a PTY there is no terminal to draw on, and the script fails at once.
    let lab = start(Account::Root, false, &["misf-tui"]).await;
    let upgrade = lab
        .run_for("upgrade", 120)
        .await
        .expect("no terminal, no dialog");
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert!(upgrade.output.contains("/dev/tty"), "{}", upgrade.output);
    assert_eq!(lab.container.rpm("misf-tui").await, "2.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_question_asked_on_the_terminal_keeps_the_upgrade_waiting() {
    let lab = start(Account::Admin, true, &["misf-tty"]).await;
    // GAP 8: with a PTY there is a terminal to ask on, and nobody to answer.
    assert!(lab.run_for("upgrade", 15).await.is_none());
    assert!(
        lab.container
            .try_sh("test -e /run/misf-tty.started")
            .await
            .0
    );
    lab.container.wait_until_dnf_idle(60).await;
    assert_eq!(lab.container.rpm("misf-tty").await, "2.0");

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
