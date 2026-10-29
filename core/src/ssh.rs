use crate::model::{CheckResult, Host, parse_packages};
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
const UPDATE: &str = "LC_ALL=C apt-get -q=1 -o APT::Update::Error-Mode=any update";
const SUDO_UPDATE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- apt-get -q=1 -o APT::Update::Error-Mode=any update";
pub const LIST: &str = "LC_ALL=C apt list --upgradable";

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

pub async fn check(host: &Host, progress: &dyn Fn(&str)) -> CheckResult {
    let checked_at = Utc::now().to_rfc3339();
    if host.os == "BSD" {
        return CheckResult {
            checked_at,
            status: "unsupported".into(),
            error: "BSD 不支援 APT 檢查".into(),
            ..Default::default()
        };
    }
    let mut transcript = String::new();
    let result: Result<Vec<crate::model::Package>> = async {
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
            progress("正在執行 apt-get update，只更新套件索引…");
            let command = if host.username == "root" {
                UPDATE
            } else {
                SUDO_UPDATE
            };
            let (code, output) =
                execute(&session, command, host.use_pty, host.sudo_secret(), 300).await?;
            transcript.push_str(&output);
            validate_update(code, &output)?;
            progress("正在讀取待更新套件…");
            let (code, output) = execute(&session, LIST, false, "", 30).await?;
            transcript.push_str("\n--- 待更新套件 ---\n");
            transcript.push_str(&output);
            ensure!(code == 0, "無法讀取待更新套件（退出碼 {code}）");
            parse_packages(&output)
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
        output: host.redact(&transcript),
        ..Default::default()
    };
    match result {
        Ok(packages) => {
            check.status = "success".into();
            check.packages = packages;
        }
        Err(error) => {
            check.status = "failed".into();
            check.error = host.redact(&format!("{error:#}"));
        }
    }
    check
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
    use super::validate_update;
    #[test]
    fn partial_repository_failure_with_zero_exit_is_not_a_fresh_result() {
        assert!(validate_update(0, "W: Failed to fetch https://repo.invalid/Packages\nW: Some index files failed to download.").is_err());
        assert!(validate_update(100, "Reading package lists...").is_err());
        assert!(validate_update(0, "Hit:1 repository\nReading package lists... Done").is_ok());
    }
}
