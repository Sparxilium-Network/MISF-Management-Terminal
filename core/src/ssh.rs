use crate::model::{
    CheckResult, Host, Package, parse_packages, parse_pkg_packages, parse_rpm_packages,
};
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
// Wrapped in sh so FreeBSD accounts with a csh login shell parse it too.
pub const DETECT: &str = "sh -c 'uname -s; uname -r; if [ -r /etc/os-release ]; then (. /etc/os-release; printf \"__MISF_NAME__%s\\n__MISF_VERSION__%s\\n__MISF_PRETTY__%s\\n\" \"$NAME\" \"$VERSION_ID\" \"$PRETTY_NAME\"); fi; if command -v apt-get >/dev/null 2>&1 && command -v apt >/dev/null 2>&1; then echo __MISF_PM__apt; fi; for m in dnf yum; do if command -v $m >/dev/null 2>&1; then echo __MISF_PM__$m; fi; done; if command -v pkg >/dev/null 2>&1 && pkg -N >/dev/null 2>&1; then echo __MISF_PM__pkg; fi; true'";
pub const UPDATE: &str = "LC_ALL=C apt-get -q=1 -o APT::Update::Error-Mode=any update";
pub const SUDO_UPDATE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- apt-get -q=1 -o APT::Update::Error-Mode=any update";
// Never prompt: keep locally modified config files and only list services needing restart.
pub const UPGRADE: &str = "LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade";
pub const SUDO_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- env LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade";
pub const LIST: &str = "LC_ALL=C apt list --upgradable";
pub const DNF_UPDATE: &str = "LC_ALL=C dnf -q -y makecache --refresh";
pub const SUDO_DNF_UPDATE: &str =
    "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- dnf -q -y makecache --refresh";
// check-update exits 100 when updates exist; installed versions follow the marker.
pub const DNF_LIST: &str = "LC_ALL=C sh -c 'dnf -q -C check-update; rc=$?; echo __MISF_INSTALLED__; rpm -qa --qf \"%{NAME}.%{ARCH} %|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}\\n\"; exit $rc'";
pub const SUDO_DNF_LIST: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- sh -c 'dnf -q -C check-update; rc=$?; echo __MISF_INSTALLED__; rpm -qa --qf \"%{NAME}.%{ARCH} %|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}\\n\"; exit $rc'";
pub const DNF_UPGRADE: &str = "LC_ALL=C dnf -y upgrade";
pub const SUDO_DNF_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- dnf -y upgrade";
pub const YUM_UPDATE: &str = "LC_ALL=C yum -q -y makecache";
pub const SUDO_YUM_UPDATE: &str =
    "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- yum -q -y makecache";
pub const YUM_LIST: &str = "LC_ALL=C sh -c 'yum -q -C check-update; rc=$?; echo __MISF_INSTALLED__; rpm -qa --qf \"%{NAME}.%{ARCH} %|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}\\n\"; exit $rc'";
pub const SUDO_YUM_LIST: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- sh -c 'yum -q -C check-update; rc=$?; echo __MISF_INSTALLED__; rpm -qa --qf \"%{NAME}.%{ARCH} %|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}\\n\"; exit $rc'";
pub const YUM_UPGRADE: &str = "LC_ALL=C yum -y upgrade";
pub const SUDO_YUM_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- yum -y upgrade";
// `env` keeps these valid under a csh login shell.
pub const PKG_UPDATE: &str = "env LC_ALL=C pkg update";
pub const SUDO_PKG_UPDATE: &str = "env LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- pkg update";
pub const PKG_LIST: &str = "env LC_ALL=C pkg version -URv -l '<'";
pub const PKG_UPGRADE: &str = "env LC_ALL=C pkg upgrade -y";
pub const SUDO_PKG_UPGRADE: &str =
    "env LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- pkg upgrade -y";
const UPGRADE_SECONDS: u64 = 30 * 60;

/// A supported package manager; one batch only ever runs one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Apt,
    Dnf,
    Yum,
    Pkg,
}

impl Manager {
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "apt" => Self::Apt,
            "dnf" => Self::Dnf,
            "yum" => Self::Yum,
            "pkg" => Self::Pkg,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Yum => "yum",
            Self::Pkg => "pkg",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Apt => "APT",
            Self::Dnf => "DNF",
            Self::Yum => "YUM",
            Self::Pkg => "pkg",
        }
    }

    /// Index update, pending listing and upgrade, without sudo for root.
    fn commands(self, root: bool) -> [&'static str; 3] {
        match (self, root) {
            (Self::Apt, true) => [UPDATE, LIST, UPGRADE],
            (Self::Apt, false) => [SUDO_UPDATE, LIST, SUDO_UPGRADE],
            (Self::Dnf, true) => [DNF_UPDATE, DNF_LIST, DNF_UPGRADE],
            (Self::Dnf, false) => [SUDO_DNF_UPDATE, SUDO_DNF_LIST, SUDO_DNF_UPGRADE],
            (Self::Yum, true) => [YUM_UPDATE, YUM_LIST, YUM_UPGRADE],
            (Self::Yum, false) => [SUDO_YUM_UPDATE, SUDO_YUM_LIST, SUDO_YUM_UPGRADE],
            (Self::Pkg, true) => [PKG_UPDATE, PKG_LIST, PKG_UPGRADE],
            (Self::Pkg, false) => [SUDO_PKG_UPDATE, PKG_LIST, SUDO_PKG_UPGRADE],
        }
    }

    fn parse(self, output: &str) -> Result<Vec<Package>> {
        match self {
            Self::Apt => parse_packages(output),
            Self::Dnf | Self::Yum => parse_rpm_packages(output),
            Self::Pkg => parse_pkg_packages(output),
        }
    }
}

/// What DETECT found: kernel name, a display name such as "Debian 13", and the package manager.
#[derive(Debug, PartialEq)]
pub struct Platform {
    pub kernel: String,
    pub distro: String,
    pub manager: Option<Manager>,
}

pub fn parse_platform(output: &str) -> Result<Platform> {
    let mut lines = output.lines().map(str::trim);
    let kernel = lines.next().unwrap_or_default().to_owned();
    ensure!(
        matches!(
            kernel.as_str(),
            "Linux" | "FreeBSD" | "OpenBSD" | "NetBSD" | "DragonFly"
        ),
        "僅支援 Linux／BSD 目標主機"
    );
    let release = lines.next().unwrap_or_default().to_owned();
    let (mut name, mut version, mut pretty, mut found) = ("", "", "", vec![]);
    for line in lines {
        if let Some(value) = line.strip_prefix("__MISF_NAME__") {
            name = value;
        } else if let Some(value) = line.strip_prefix("__MISF_VERSION__") {
            version = value;
        } else if let Some(value) = line.strip_prefix("__MISF_PRETTY__") {
            pretty = value;
        } else if let Some(value) = line.strip_prefix("__MISF_PM__") {
            found.extend(Manager::from_id(value));
        }
    }
    let name = name.trim_end_matches(" GNU/Linux");
    let distro = match (name, version, pretty) {
        ("", _, "") => format!("{kernel} {release}").trim().to_owned(),
        (_, "", pretty) if !pretty.is_empty() => pretty.to_owned(),
        (name, version, _) => format!("{name} {version}").trim().to_owned(),
    };
    // Debian derivatives can carry yum, so the native manager wins.
    let preferred: &[Manager] = match kernel.as_str() {
        "Linux" => &[Manager::Apt, Manager::Dnf, Manager::Yum],
        "FreeBSD" | "DragonFly" => &[Manager::Pkg],
        _ => &[],
    };
    let manager = preferred.iter().copied().find(|m| found.contains(m));
    Ok(Platform {
        kernel,
        distro,
        manager,
    })
}

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

pub async fn test_connection(host: &Host) -> Result<Platform> {
    let session = authenticate(host).await?;
    let result = execute(&session, DETECT, false, "", 20).await;
    session
        .disconnect(Disconnect::ByApplication, "", "")
        .await
        .ok();
    let (code, output) = result?;
    ensure!(code == 0, "無法辨識遠端作業系統");
    parse_platform(&output)
}

/// The commands an operation with this manager runs, in order.
pub fn manager_commands(manager: Manager, root: bool, upgrade: bool) -> Vec<&'static str> {
    let [update, list, install] = manager.commands(root);
    let mut commands = vec![DETECT, update, list];
    if upgrade {
        commands.extend([install, list]);
    }
    commands
}

/// The commands a check or upgrade runs on this host; only DETECT until its manager is known.
pub fn package_commands(host: &Host, upgrade: bool) -> Vec<&'static str> {
    match Manager::from_id(host.manager()) {
        Some(manager) => manager_commands(manager, host.username == "root", upgrade),
        None => vec![DETECT],
    }
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
    let mut transcript = String::new();
    let mut platform = None;
    let result: Result<(Vec<Package>, Vec<Package>)> = async {
        progress("正在連線並驗證 SSH 主機指紋…");
        let session = authenticate(host).await?;
        let operation = async {
            let (code, output) = execute(&session, DETECT, false, "", 20).await?;
            ensure!(code == 0, "無法辨識遠端作業系統");
            let detected = parse_platform(&output)?;
            let manager = detected.manager;
            platform = Some(detected);
            let manager = manager.context(
                "此主機沒有支援的套件管理器（APT、DNF、YUM 或 FreeBSD pkg）；不支援此檢查",
            )?;
            // The confirmation showed the recorded manager's commands; never run another's.
            let expected = host.manager();
            ensure!(
                expected.is_empty() || expected == manager.id(),
                "偵測到的套件管理器（{}）與紀錄（{expected}）不同；請先按「測試連線」更新主機資訊",
                manager.label()
            );
            let [update, list_command, install] = manager.commands(host.username == "root");
            progress("正在更新套件索引…");
            let (code, output) =
                execute(&session, update, host.use_pty, host.sudo_secret(), 300).await?;
            transcript.push_str(&output);
            validate_update(manager, code, &output)?;
            progress("正在讀取待更新套件…");
            let pending = list(&session, host, manager, list_command, &mut transcript).await?;
            if !upgrade || pending.is_empty() {
                return Ok((pending, vec![]));
            }
            progress("正在升級可升級套件…");
            let (code, output) = execute(
                &session,
                install,
                host.use_pty,
                host.sudo_secret(),
                UPGRADE_SECONDS,
            )
            .await?;
            transcript.push_str(&format!("\n--- {} upgrade ---\n", manager.label()));
            transcript.push_str(&output);
            validate_upgrade(code, &output)?;
            progress("正在重新讀取剩餘待更新套件…");
            let remaining = list(&session, host, manager, list_command, &mut transcript).await?;
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
    if let Some(platform) = platform {
        check.distro = platform.distro;
        check.package_manager = platform.manager.map(Manager::id).unwrap_or_default().into();
        if platform.manager.is_none() {
            check.status = "unsupported".into();
        }
    }
    match result {
        Ok((packages, upgraded)) => {
            check.status = "success".into();
            check.packages = packages;
            check.upgraded = upgraded;
        }
        Err(error) => {
            if check.status.is_empty() {
                check.status = "failed".into();
            }
            check.error = host.redact(&format!("{error:#}"));
        }
    }
    check
}

async fn list(
    session: &client::Handle<Verifier>,
    host: &Host,
    manager: Manager,
    command: &str,
    transcript: &mut String,
) -> Result<Vec<Package>> {
    // RPM listings read root's metadata cache, so they go through sudo like the update.
    let (pty, secret) = if command.contains("sudo -S") {
        (host.use_pty, host.sudo_secret())
    } else {
        (false, "")
    };
    let (code, output) = execute(session, command, pty, secret, 120).await?;
    transcript.push_str("\n--- 待更新套件 ---\n");
    transcript.push_str(&output);
    let listed = match manager {
        Manager::Dnf | Manager::Yum => code == 0 || code == 100,
        _ => code == 0,
    };
    ensure!(listed, "無法讀取待更新套件（退出碼 {code}）");
    manager.parse(&output)
}

fn validate_upgrade(code: u32, output: &str) -> Result<()> {
    ensure!(
        code == 0
            && !output.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("E:") || line.starts_with("Error:")
            }),
        "套件升級失敗（退出碼 {code}）；請查看執行輸出，遠端可能已部分升級"
    );
    Ok(())
}

fn validate_update(manager: Manager, code: u32, output: &str) -> Result<()> {
    let failed = output.lines().any(|line| {
        let line = line.trim_start();
        match manager {
            Manager::Apt => {
                line.starts_with("E:")
                    || line.starts_with("Err:")
                    || line.contains("Failed to fetch")
                    || line.contains("Some index files failed to download")
            }
            // Skipped repositories still exit 0 under skip_if_unavailable.
            Manager::Dnf | Manager::Yum => {
                line.starts_with("Error:")
                    || line.contains("Failed to download metadata")
                    || line.starts_with("Ignoring repositories")
            }
            Manager::Pkg => line.contains("Unable to update repository"),
        }
    });
    ensure!(
        code == 0 && !failed,
        "更新套件索引失敗（退出碼 {code}）；未把舊快取當作最新結果"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Manager, Platform, parse_platform, validate_update, validate_upgrade};
    #[test]
    fn partial_repository_failure_with_zero_exit_is_not_a_fresh_result() {
        let apt = Manager::Apt;
        assert!(validate_update(apt, 0, "W: Failed to fetch https://repo.invalid/Packages\nW: Some index files failed to download.").is_err());
        assert!(validate_update(apt, 100, "Reading package lists...").is_err());
        assert!(validate_update(apt, 0, "Hit:1 repository\nReading package lists... Done").is_ok());
        assert!(validate_update(Manager::Dnf, 0, "Ignoring repositories: epel").is_err());
        assert!(validate_update(Manager::Dnf, 0, "Metadata cache created.").is_ok());
        assert!(
            validate_update(Manager::Pkg, 0, "pkg: Unable to update repository FreeBSD").is_err()
        );
    }

    #[test]
    fn detection_names_the_distribution_and_prefers_the_native_manager() {
        let linux = |os: &str, managers: &str| {
            parse_platform(&format!("Linux\n6.12.0\n{os}{managers}")).unwrap()
        };
        assert_eq!(
            linux(
                "__MISF_NAME__Debian GNU/Linux\n__MISF_VERSION__13\n__MISF_PRETTY__Debian GNU/Linux 13 (trixie)\n",
                "__MISF_PM__apt\n__MISF_PM__yum\n"
            ),
            Platform {
                kernel: "Linux".into(),
                distro: "Debian 13".into(),
                manager: Some(Manager::Apt),
            }
        );
        let ubuntu = linux(
            "__MISF_NAME__Ubuntu\n__MISF_VERSION__26.04\n",
            "__MISF_PM__apt\n",
        );
        assert_eq!(ubuntu.distro, "Ubuntu 26.04");
        let rocky = linux(
            "__MISF_NAME__Rocky Linux\n__MISF_VERSION__9.5\n",
            "__MISF_PM__dnf\n__MISF_PM__yum\n",
        );
        assert_eq!(
            (rocky.distro.as_str(), rocky.manager),
            ("Rocky Linux 9.5", Some(Manager::Dnf))
        );
        let sid = linux(
            "__MISF_NAME__Debian GNU/Linux\n__MISF_VERSION__\n__MISF_PRETTY__Debian GNU/Linux trixie/sid\n",
            "",
        );
        assert_eq!(
            (sid.distro.as_str(), sid.manager),
            ("Debian GNU/Linux trixie/sid", None)
        );
        let freebsd = parse_platform("FreeBSD\n14.2-RELEASE\n__MISF_PM__pkg\n").unwrap();
        assert_eq!(
            (freebsd.distro.as_str(), freebsd.manager),
            ("FreeBSD 14.2-RELEASE", Some(Manager::Pkg))
        );
        let openbsd = parse_platform("OpenBSD\n7.6\n__MISF_PM__pkg\n").unwrap();
        assert_eq!(openbsd.manager, None);
        assert!(parse_platform("Darwin\n24.0\n").is_err());
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
