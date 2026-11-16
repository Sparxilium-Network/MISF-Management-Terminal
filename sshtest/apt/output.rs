//! Output no log should trust: control characters, fake error lines and the sudo
//! prompt marker printed by a package script.

use crate::support::{Dpkg, assert_failed, assert_plain, names, start};
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
    let lab = start(Account::Root, true, &["misf-eline"]).await;
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
    let lab = start(Account::Root, true, &["misf-marker"]).await;
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
        let lab = start(Account::Admin, pty, &["misf-marker"]).await;
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
        let lab = start(Account::Deploy, pty, &["misf-marker"]).await;
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
