//! The SSH transport: host-key checks, sign-in and running one command.
//! What runs over it for package management lives in `package_manager`.

use crate::model::{ConnectionTest, Host, decode_private_key};
use crate::package_manager::{DETECT, Platform, SUDO_PROMPT as PROMPT, parse_platform};
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

/// Output kept from one remote command; more aborts the command.
const LIMIT: usize = 32 * 1024 * 1024;

pub(crate) type Details = Arc<Mutex<ConnectionTest>>;
/// A signed-in SSH session.
pub(crate) type Session = client::Handle<Verifier>;

pub(crate) struct Verifier {
    expected: Option<String>,
    observed: Arc<Mutex<Option<String>>>,
    /// Filled in as the handshake goes, so a failed test still shows how far it got.
    details: Details,
}

impl client::Handler for Verifier {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        *self.observed.lock().unwrap() = Some(fingerprint.clone());
        self.details.lock().unwrap().host_key_fingerprint = fingerprint.clone();
        Ok(self
            .expected
            .as_ref()
            .is_none_or(|expected| *expected == fingerprint))
    }

    async fn kex_done(
        &mut self,
        _shared_secret: Option<&[u8]>,
        names: &russh::Names,
        session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let mut details = self.details.lock().unwrap();
        details.server_version = server_version(session.remote_sshid());
        details.kex = names.kex.as_ref().into();
        details.host_key_algorithm = names.key.as_str().into();
        details.cipher = names.cipher.as_ref().into();
        details.client_mac = names.client_mac.as_ref().into();
        details.server_mac = names.server_mac.as_ref().into();
        details.strict_kex = names.strict_kex();
        Ok(())
    }
}

/// The server picks this text, so control characters go and it stays within RFC 4253's 255.
fn server_version(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control())
        .take(255)
        .collect::<String>()
        .trim()
        .to_owned()
}

async fn connect(host: &Host, probe: bool, details: &Details) -> Result<client::Handle<Verifier>> {
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
                details: details.clone(),
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
    result.map_err(|error| handshake_error(error, "無法建立 SSH 連線；請確認位址、埠及網路"))
}

/// Names the cause when the server offers only host keys this build cannot verify, e.g. RSA.
fn handshake_error(error: russh::Error, fallback: &'static str) -> anyhow::Error {
    if let russh::Error::NoCommonAlgo {
        kind: russh::AlgorithmKind::Key,
        theirs,
        ..
    } = &error
    {
        return anyhow::anyhow!(
            "伺服器只提供不支援的主機金鑰類型（{}）；MISF 不支援 RSA，請在伺服器啟用 Ed25519 或 ECDSA 主機金鑰",
            theirs.join("、")
        );
    }
    anyhow::Error::new(error).context(fallback)
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
                details: Details::default(),
            },
        ),
    )
    .await
    .context("讀取主機指紋逾時")?
    .map_err(|error| handshake_error(error, "無法讀取 SSH 主機指紋"))?;
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

pub(crate) async fn authenticate(host: &Host, details: &Details) -> Result<Session> {
    let key = if host.auth_type == "password" {
        None
    } else {
        Some(decode_private_key(&host.private_key, host.passphrase())?)
    };
    let (method, key_algorithm, key_fingerprint) = match &key {
        None => ("password", String::new(), String::new()),
        Some(key) => (
            "publickey",
            key.algorithm().as_str().to_owned(),
            key.public_key().fingerprint(HashAlg::Sha256).to_string(),
        ),
    };
    let mut session = connect(host, false, details).await?;
    let auth = timeout(Duration::from_secs(20), async {
        match key {
            None => {
                session
                    .authenticate_password(&host.username, &host.password)
                    .await
            }
            // RSA keys are rejected above, so no signature hash needs choosing.
            Some(key) => {
                session
                    .authenticate_publickey(
                        &host.username,
                        PrivateKeyWithHashAlg::new(Arc::new(key), None),
                    )
                    .await
            }
        }
    })
    .await
    .context("SSH 認證逾時")?
    .context("SSH 認證失敗")?;
    ensure!(auth.success(), "SSH 帳號、密碼或私鑰認證失敗");
    let mut details = details.lock().unwrap();
    details.auth_method = method.into();
    details.client_key_algorithm = key_algorithm;
    details.client_key_fingerprint = key_fingerprint;
    drop(details);
    Ok(session)
}

/// Ends a session; a failure to say goodbye changes nothing for the caller.
pub(crate) async fn disconnect(session: &Session) {
    session
        .disconnect(Disconnect::ByApplication, "", "")
        .await
        .ok();
}

/// Runs one command; answers a sudo prompt once with `secret`. Returns exit code and output.
pub(crate) async fn execute(
    session: &Session,
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
                        "遠端輸出超過 32 MiB 限制"
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
    .context("遠端命令逾時；請確認目標主機上的套件管理器狀態")?
}

/// Signs in, runs DETECT and records what was negotiated; failures are recorded, not returned.
pub async fn test_connection(host: &Host) -> ConnectionTest {
    let tested_at = Utc::now().to_rfc3339();
    let details = Details::default();
    let result: Result<Platform> = async {
        let session = authenticate(host, &details).await?;
        let result = execute(&session, DETECT, false, "", 20).await;
        disconnect(&session).await;
        let (code, output) = result?;
        ensure!(code == 0, "無法辨識遠端作業系統");
        parse_platform(&output)
    }
    .await;
    let mut test = details.lock().unwrap().clone();
    test.tested_at = tested_at;
    match result {
        Ok(platform) => {
            test.status = "success".into();
            test.kernel = platform.kernel;
            test.distro = platform.distro;
            test.package_managers = platform
                .available
                .iter()
                .map(|m| m.id().to_owned())
                .collect();
        }
        Err(error) => {
            test.status = "failed".into();
            test.error = host.redact(&format!("{error:#}"));
        }
    }
    test
}
