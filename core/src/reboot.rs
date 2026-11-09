//! Reboots an APT host when the user confirms it twice in the UI. The command is fixed
//! and scheduled one minute ahead: the SSH command returns cleanly, and `shutdown -c` on
//! the host can still cancel it.

use crate::model::Host;
use crate::ssh;
use anyhow::{Result, ensure};

pub const ROOT: &str = "LC_ALL=C shutdown -r +1";
pub const SUDO: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- shutdown -r +1";

const SECONDS: u64 = 60;

/// The command a reboot runs on this host: root directly, others through sudo.
pub fn command(host: &Host) -> &'static str {
    if host.username == "root" { ROOT } else { SUDO }
}

/// Schedules the reboot and returns the host's redacted reply.
pub async fn reboot(host: &Host, progress: &dyn Fn(&str)) -> Result<String> {
    ensure!(host.manager() == "apt", "重新開機只支援 APT 主機");
    progress("正在連線並驗證 SSH 主機指紋…");
    let session = ssh::authenticate(host, &Default::default()).await?;
    progress("正在排程重新開機…");
    let root = host.username == "root";
    let result = ssh::execute(
        &session,
        command(host),
        !root && host.use_pty,
        if root { "" } else { host.sudo_secret() },
        SECONDS,
    )
    .await;
    ssh::disconnect(&session).await;
    let (code, output) = result?;
    let output = host.redact(&output);
    ensure!(
        code == 0,
        "無法排程重新開機（退出碼 {code}）：{}",
        output.trim()
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_manager::SUDO_PROMPT;

    #[test]
    fn root_reboots_directly_and_others_answer_the_sudo_marker() {
        let mut host = Host {
            username: "root".into(),
            ..Default::default()
        };
        assert_eq!(command(&host), ROOT);
        assert!(!ROOT.contains("sudo"));
        host.username = "operator".into();
        assert_eq!(command(&host), SUDO);
        assert!(SUDO.contains(&format!("sudo -S -p '{SUDO_PROMPT}' -- shutdown -r +1")));
    }
}
