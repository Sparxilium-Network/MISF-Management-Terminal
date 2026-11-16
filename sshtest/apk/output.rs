//! Output no log should trust: control characters, fake error lines, the sudo prompt
//! marker, and a script that asks a question.

use crate::support::{Apk, assert_failed, assert_plain, names, start};
use misf_sshtest::Account;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn garbage_from_a_package_script_is_cleaned_and_does_not_fail_the_upgrade() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = start(account, pty, &["misf-noise"]).await;
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
        assert!(output.contains("progress 100\n"), "{output}");
        // apk itself ends a script's line at a NUL byte.
        assert!(output.contains("nul and bell: a\n"), "{output}");
        assert!(output.contains("backspaces: xyz\n"), "{output}");
        assert!(output.contains("wide text: 中文輸出\n"), "{output}");
        assert!(
            output.contains("invalid utf-8: \u{fffd}\u{fffd}\u{fffd} end\n"),
            "{output}"
        );
        // GAP 2: an escape sequence that never ends swallows the rest of that command's
        // saved output, here the script's last line and apk's closing "OK:" line.
        assert!(output.contains("an escape that never ends: "), "{output}");
        assert!(!output.contains("misf-noise: end"), "{output}");
        // The live view works line by line and loses only that one line.
        let screen = lab.screen.text();
        assert_plain(&lab, &screen);
        assert!(screen.contains("misf-noise: end\n"), "{screen}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_script_line_that_looks_like_an_apk_error_is_only_text() {
    let lab = start(Account::Root, true, &["misf-eline"]).await;
    let upgrade = lab.run("upgrade").await;
    // apk indents what a script prints behind `* `, so it never starts a line the way
    // apk's own `ERROR:` does.
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert!(
        (upgrade.output).contains("* ERROR: this line is only text printed by a package script")
    );
    assert_eq!(lab.container.apk("misf-eline").await, "2.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn root_ignores_a_package_script_that_prints_the_sudo_prompt_marker() {
    let lab = start(Account::Root, true, &["misf-marker"]).await;
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-marker"]);
    assert_plain(&lab, &upgrade.output);
    assert_eq!(lab.container.sh("cat /run/misf-marker.stdin").await, "");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn the_marker_after_sudo_took_the_password_reads_as_a_wrong_password() {
    let lab = start(Account::Admin, true, &["misf-marker"]).await;
    let upgrade = lab.run("upgrade").await;
    // GAP 1: sudo accepted the password and apk is upgrading; the marker in a package
    // script's output then counts as a second prompt. MISF reports a wrong sudo password,
    // drops the connection and saves nothing of the upgrade step.
    assert_failed(&upgrade, "sudo 密碼錯誤或需要額外驗證");
    assert!(
        !upgrade.output.contains("apk upgrade"),
        "{}",
        upgrade.output
    );
    lab.container.wait_until_apk_idle(60).await;
    let received = lab
        .container
        .sh("cat /run/misf-marker.stdin 2>/dev/null")
        .await;
    assert!(!received.contains(Account::Admin.password()), "{received}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn with_passwordless_sudo_the_marker_is_answered_but_no_script_can_read_it() {
    for pty in [false, true] {
        let lab = start(Account::Deploy, pty, &["misf-marker"]).await;
        let upgrade = lab.run("upgrade").await;
        assert_eq!(upgrade.status, "success", "{}", upgrade.error);
        assert_eq!(lab.container.apk("misf-marker").await, "2.0");
        // sudo never asked, so MISF answers the marker with the password; apk gives its
        // scripts no input, so this one reads nothing.
        assert_eq!(lab.container.sh("cat /run/misf-marker.stdin").await, "");
        assert_plain(&lab, &upgrade.output);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_question_on_the_scripts_input_gets_end_of_input_at_once() {
    for (account, pty) in [(Account::Admin, true), (Account::Root, false)] {
        let lab = start(account, pty, &["misf-prompt"]).await;
        let upgrade = lab.run("upgrade").await;
        assert_eq!(upgrade.status, "success", "{}", upgrade.error);
        assert_eq!(names(&upgrade.upgraded), ["misf-prompt"]);
        assert_eq!(
            lab.container.sh("cat /run/misf-prompt.answer").await.trim(),
            "answer="
        );
        assert!(upgrade.output.contains("misf-prompt: answered"));
    }
}
