use crate::model::{CheckResult, Host, Package, parse_packages};
use anyhow::{Context, Result, bail, ensure};
use chrono::Utc;
use russh::{
    ChannelMsg, Disconnect, Pty, client,
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::timeout;

const PROMPT: &str = "__MISF_SUDO_PROMPT__";
const LIMIT: usize = 2 * 1024 * 1024;
pub const DETECT: &str = "uname -s; if command -v apt-get >/dev/null 2>&1 && command -v apt >/dev/null 2>&1; then printf '__MISF_APT__\\n'; fi";
pub const UPDATE: &str = "LC_ALL=C apt-get -q=1 -o APT::Update::Error-Mode=any update";
pub const SUDO_UPDATE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- apt-get -q=1 -o APT::Update::Error-Mode=any update";
// Never prompt: keep locally modified config files and only list services needing restart.
pub const UPGRADE: &str = "LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade";
pub const SUDO_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- env LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade";
pub const LIST: &str = "LC_ALL=C apt list --upgradable";
const UPGRADE_SECONDS: u64 = 30 * 60;

struct Verifier {
    expected: Option<String>,
    observed: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Verifier {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        *self.observed.lock().unwrap() = Some(fingerprint.clone());
        Ok(self
            .expected
            .as_ref()
            .is_none_or(|expected| *expected == fingerprint))
    }
}

async fn connect(host: &Host, probe: bool) -> Result<client::Handle<Verifier>> {
    if !probe {
        ensure!(!host.fingerprint.is_empty(), "請先確認並信任 SSH 主機指紋");
    }
    let observed = Arc::new(Mutex::new(None));
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(360)),
        // Long dpkg triggers can be silent; keepalive replies keep the session active.
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 6,
        ..Default::default()
    });
    let result = timeout(
        Duration::from_secs(15),
        client::connect(
            config,
            (host.address.as_str(), host.port),
            Verifier {
                expected: if probe {
                    None
                } else {
                    Some(host.fingerprint.clone())
                },
                observed: observed.clone(),
            },
        ),
    )
    .await
    .context("SSH 連線逾時（15 秒）")?;
    if let Some(actual) = observed.lock().unwrap().as_ref()
        && !probe
        && *actual != host.fingerprint
    {
        bail!(
            "SSH 主機指紋已變更，已阻止連線。預期 {}；收到 {}。請編輯主機並重新確認指紋。",
            host.fingerprint,
            actual
        );
    }
    result.context("無法建立 SSH 連線；請確認位址、埠及網路")
}

pub async fn probe(host: &Host) -> Result<String> {
    // Handshake only: this connection never submits a username or credentials.
    let observed = Arc::new(Mutex::new(None));
    let config = Arc::new(client::Config::default());
    let session = timeout(
        Duration::from_secs(15),
        client::connect(
            config,
            (host.address.as_str(), host.port),
            Verifier {
                expected: None,
                observed: observed.clone(),
            },
        ),
    )
    .await
    .context("讀取主機指紋逾時")?
    .context("無法讀取 SSH 主機指紋")?;
    session
        .disconnect(Disconnect::ByApplication, "", "")
        .await
        .ok();
    let fingerprint = observed
        .lock()
        .unwrap()
        .clone()
        .context("伺服器沒有提供主機指紋")?;
    Ok(fingerprint)
}

async fn authenticate(host: &Host) -> Result<client::Handle<Verifier>> {
    let mut session = connect(host, false).await?;
    let auth = timeout(Duration::from_secs(20), async {
        if host.auth_type == "password" {
            session
                .authenticate_password(&host.username, &host.password)
                .await
        } else {
            let key = russh::keys::decode_secret_key(&host.private_key, host.passphrase())
                .map_err(|_| russh::Error::CouldNotReadKey)?;
            let algorithm = session.best_supported_rsa_hash().await?.flatten();
            session
                .authenticate_publickey(
                    &host.username,
                    PrivateKeyWithHashAlg::new(Arc::new(key), algorithm),
                )
                .await
        }
    })
    .await
    .context("SSH 認證逾時")?
    .context("SSH 認證失敗")?;
    ensure!(auth.success(), "SSH 帳號、密碼或私鑰認證失敗");
    Ok(session)
}

async fn execute(
    session: &client::Handle<Verifier>,
    command: &str,
    pty: bool,
    secret: &str,
    seconds: u64,
) -> Result<(u32, String)> {
    timeout(Duration::from_secs(seconds), async {
        let mut channel = session.channel_open_session().await?;
        if pty {
            channel
                .request_pty(
                    true,
                    "dumb",
                    120,
                    40,
                    0,
                    0,
                    &[(Pty::ECHO, 0), (Pty::ECHONL, 0)],
                )
                .await?;
        }
        channel.exec(true, command).await?;
        let mut output = Vec::new();
        let mut exit = None;
        let mut sent = false;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    ensure!(
                        output.len() + data.len() <= LIMIT,
                        "遠端輸出超過 2 MiB 限制"
                    );
                    output.extend_from_slice(&data);
                    let prompts = String::from_utf8_lossy(&output).matches(PROMPT).count();
                    if prompts > 0 && !sent {
                        ensure!(
                            !secret.is_empty(),
                            "sudo 要求密碼；請在主機設定填寫 sudo 密碼"
                        );
                        ensure!(!secret.contains(['\n', '\r']), "sudo 密碼不能包含換行");
                        channel.data(format!("{secret}\n").as_bytes()).await?;
                        sent = true;
                    } else if prompts > 1 {
                        bail!("sudo 密碼錯誤或需要額外驗證");
                    }
                }
                ChannelMsg::ExitStatus { exit_status } => exit = Some(exit_status),
                ChannelMsg::ExitSignal { .. } => bail!("遠端命令被訊號中止"),
                ChannelMsg::Failure => {
                    bail!("遠端拒絕 PTY 或命令執行請求；可在主機設定停用 PTY 後重試")
                }
                _ => {}
            }
        }
        let output = crate::transcript::readable_output(
            &String::from_utf8_lossy(&output).replace(PROMPT, ""),
        );
        Ok((exit.context("SSH 連線結束但沒有命令退出碼")?, output))
    })
    .await
    .context("遠端命令逾時；請確認目標主機上的 APT 狀態")?
}

pub async fn test_connection(host: &Host) -> Result<String> {
    let session = authenticate(host).await?;
    let result = execute(&session, "uname -s", false, "", 20).await;
    session
        .disconnect(Disconnect::ByApplication, "", "")
        .await
        .ok();
    let (code, output) = result?;
    ensure!(code == 0, "無法辨識遠端作業系統");
    let os = output.trim();
    ensure!(
        matches!(os, "Linux" | "FreeBSD" | "OpenBSD" | "NetBSD" | "DragonFly"),
        "僅支援 Linux／BSD 目標主機"
    );
    Ok(os.into())
}

/// The commands an APT operation runs for this host, in order.
pub fn apt_commands(host: &Host, upgrade: bool) -> Vec<&'static str> {
    let root = host.username == "root";
    let mut commands = vec![DETECT, if root { UPDATE } else { SUDO_UPDATE }, LIST];
    if upgrade {
        commands.extend([if root { UPGRADE } else { SUDO_UPGRADE }, LIST]);
    }
    commands
}

pub async fn check(host: &Host, progress: &dyn Fn(&str)) -> CheckResult {
    run_apt(host, false, progress).await
}

pub async fn upgrade(host: &Host, progress: &dyn Fn(&str)) -> CheckResult {
    run_apt(host, true, progress).await
}

// Generic so a batch task's no-op progress keeps the future `Send`.
pub(crate) async fn run_apt<P: Fn(&str) + ?Sized>(
    host: &Host,
    upgrade: bool,
    progress: &P,
) -> CheckResult {
    let checked_at = Utc::now().to_rfc3339();
    let operation = if upgrade { "upgrade" } else { "check" };
    if host.os == "BSD" {
        return CheckResult {
            checked_at,
            operation: operation.into(),
            status: "unsupported".into(),
            error: "BSD 不支援 APT 檢查".into(),
            ..Default::default()
        };
    }
    let mut transcript = String::new();
    let result: Result<(Vec<Package>, Vec<Package>)> = async {
        progress("正在連線並驗證 SSH 主機指紋…");
        let session = authenticate(host).await?;
        let operation = async {
            let (code, output) = execute(&session, DETECT, false, "", 20).await?;
            ensure!(
                code == 0
                    && output.lines().next() == Some("Linux")
                    && output.contains("__MISF_APT__"),
                "此主機沒有 APT，或不是 Linux；不支援此檢查"
            );
            let commands = apt_commands(host, upgrade);
            progress("正在執行 apt-get update，只更新套件索引…");
            let (code, output) =
                execute(&session, commands[1], host.use_pty, host.sudo_secret(), 300).await?;
            transcript.push_str(&output);
            validate_update(code, &output)?;
            progress("正在讀取待更新套件…");
            let pending = list(&session, &mut transcript).await?;
            if !upgrade || pending.is_empty() {
                return Ok((pending, vec![]));
            }
            progress("正在執行 apt-get upgrade，安裝可升級套件…");
            let (code, output) = execute(
                &session,
                commands[3],
                host.use_pty,
                host.sudo_secret(),
                UPGRADE_SECONDS,
            )
            .await?;
            transcript.push_str("\n--- apt-get upgrade ---\n");
            transcript.push_str(&output);
            validate_upgrade(code, &output)?;
            progress("正在重新讀取剩餘待更新套件…");
            let remaining = list(&session, &mut transcript).await?;
            let upgraded = pending
                .into_iter()
                .filter(|package| !remaining.iter().any(|left| left.name == package.name))
                .collect();
            Ok((remaining, upgraded))
        }
        .await;
        session
            .disconnect(Disconnect::ByApplication, "", "")
            .await
            .ok();
        operation
    }
    .await;
    let mut check = CheckResult {
        checked_at,
        operation: operation.into(),
        output: host.redact(&transcript),
        ..Default::default()
    };
    match result {
        Ok((packages, upgraded)) => {
            check.status = "success".into();
            check.packages = packages;
            check.upgraded = upgraded;
        }
        Err(error) => {
            check.status = "failed".into();
            check.error = host.redact(&format!("{error:#}"));
        }
    }
    check
}

async fn list(session: &client::Handle<Verifier>, transcript: &mut String) -> Result<Vec<Package>> {
    let (code, output) = execute(session, LIST, false, "", 30).await?;
    transcript.push_str("\n--- 待更新套件 ---\n");
    transcript.push_str(&output);
    ensure!(code == 0, "無法讀取待更新套件（退出碼 {code}）");
    parse_packages(&output)
}

fn validate_upgrade(code: u32, output: &str) -> Result<()> {
    ensure!(
        code == 0
            && !output
                .lines()
                .any(|line| line.trim_start().starts_with("E:")),
        "apt-get upgrade 失敗（退出碼 {code}）；請查看執行輸出，遠端可能已部分升級"
    );
    Ok(())
}

fn validate_update(code: u32, output: &str) -> Result<()> {
    ensure!(
        code == 0
            && !output.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("E:")
                    || line.starts_with("Err:")
                    || line.contains("Failed to fetch")
                    || line.contains("Some index files failed to download")
            }),
        "更新套件索引失敗（退出碼 {code}）；未把舊快取當作最新結果"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_update, validate_upgrade};
    #[test]
    fn partial_repository_failure_with_zero_exit_is_not_a_fresh_result() {
        assert!(validate_update(0, "W: Failed to fetch https://repo.invalid/Packages\nW: Some index files failed to download.").is_err());
        assert!(validate_update(100, "Reading package lists...").is_err());
        assert!(validate_update(0, "Hit:1 repository\nReading package lists... Done").is_ok());
    }

    #[test]
    fn upgrade_errors_are_failures_even_with_zero_exit() {
        assert!(
            validate_upgrade(0, "E: Sub-process /usr/bin/dpkg returned an error code (1)").is_err()
        );
        assert!(validate_upgrade(100, "Setting up libc6").is_err());
        assert!(validate_upgrade(0, "Setting up libc6 (2.36-9) ...").is_ok());
    }
}
