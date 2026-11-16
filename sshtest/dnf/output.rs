//! Output no log should trust: control characters, fake error lines and the sudo prompt
//! marker printed by a package script.

use crate::support::{Rpm, assert_failed, assert_plain, names, start};
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
        assert!(output.contains("progress 10\nprogress 50\nprogress 100\n"));
        assert!(output.contains("nul and bell: abc\n"));
        assert!(output.contains("backspaces: xyz\n"));
        assert!(output.contains("wide text: 中文輸出\n"));
        // Whatever dnf made of the bytes that are not UTF-8, the line survived.
        assert!(output.contains("invalid utf-8: ") && output.contains(" end\n"));
        // The saved output keeps the 100,000-character line whole.
        assert!(output.contains(&"x".repeat(100_000)));
        assert!(output.contains("misf-noise: after the long line\n"));

        // GAP 6: an escape sequence that never ends swallows the rest of that command's
        // saved output, here the script's last line and dnf's own "Complete!".
        assert!(output.contains("an escape that never ends: "));
        assert!(!output.contains("misf-noise: end"), "{output}");
        assert!(!output.contains("Complete!"), "{output}");

        // The live view works line by line: the long line is cut and marked, and the
        // unfinished escape costs only its own line.
        let screen = lab.screen.text();
        assert_plain(&lab, &screen);
        assert!(screen.contains("（此行過長，已截斷）"));
        assert!(!screen.contains(&"x".repeat(9_000)));
        assert!(screen.contains("misf-noise: end\n"), "{screen}");
        assert!(screen.contains("Complete!\n"));
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn a_script_line_that_looks_like_a_dnf_error_fails_an_upgrade_that_worked() {
    let lab = start(Account::Root, true, &["misf-eline"]).await;
    let upgrade = lab.run("upgrade").await;
    // GAP 3: dnf exited 0 and the package is installed, but any line starting with
    // "Error:" counts as a dnf error, so the run is saved as failed.
    assert_failed(&upgrade, "套件升級失敗（退出碼 0）");
    assert!((upgrade.output).contains("Error: this line is only text printed by a package script"));
    assert!(upgrade.output.contains("Complete!"));
    assert_eq!(lab.container.rpm("misf-eline").await, "2.0");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn root_ignores_a_package_script_that_prints_the_sudo_prompt_marker() {
    let lab = start(Account::Root, true, &["misf-marker"]).await;
    let upgrade = lab.run("upgrade").await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(names(&upgrade.upgraded), ["misf-marker"]);
    assert!(upgrade.output.contains("misf-marker says \n"));
    assert_plain(&lab, &upgrade.output);
    assert_eq!(lab.container.sh("cat /run/misf-marker.stdin").await, "");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn the_marker_after_sudo_took_the_password_reads_as_a_wrong_password() {
    for pty in [true, false] {
        let lab = start(Account::Admin, pty, &["misf-marker"]).await;
        let upgrade = lab.run("upgrade").await;
        // GAP 7: sudo accepted the password and dnf is upgrading; the marker in a package
        // script's output then counts as a second prompt. MISF reports a wrong sudo
        // password, drops the connection and saves nothing of the upgrade step.
        assert_failed(&upgrade, "sudo 密碼錯誤或需要額外驗證");
        assert!(
            !upgrade.output.contains("DNF upgrade"),
            "{}",
            upgrade.output
        );
        // The package itself ends up installed.
        lab.container.wait_until_dnf_idle(60).await;
        assert_eq!(lab.container.rpm("misf-marker").await, "2.0");
        let received = lab.container.sh("cat /run/misf-marker.stdin").await;
        assert!(!received.contains(Account::Admin.password()), "{received}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker: cargo test -p misf-sshtest -- --ignored"]
async fn with_passwordless_sudo_the_marker_is_answered_but_no_script_can_read_it() {
    for pty in [false, true] {
        let lab = start(Account::Deploy, pty, &["misf-marker"]).await;
        let upgrade = lab.run("upgrade").await;
        assert_eq!(upgrade.status, "success", "{}", upgrade.error);
        assert_eq!(lab.container.rpm("misf-marker").await, "2.0");
        // sudo never asked, so MISF answers the marker with the password as it does on
        // APT hosts. rpm gives its scriptlets no input, though, so unlike a dpkg
        // maintainer script this one reads nothing.
        assert_eq!(lab.container.sh("cat /run/misf-marker.stdin").await, "");
        assert_plain(&lab, &upgrade.output);
        assert_plain(&lab, &lab.screen.text());
    }
}
