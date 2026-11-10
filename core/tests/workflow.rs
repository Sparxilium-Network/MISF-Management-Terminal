use misf_core::{
    batch::{self, Decision},
    dispatch, dispatch_with,
    model::{CheckResult, Host},
    package_manager::{self, Manager, Mode, apt, dnf},
    reboot, ssh,
    store::Store,
};
use russh::{
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Server as _, Session, VerifiedPublicKey},
    *,
};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

const APT_HOST: &str = "Linux\n6.12.0\n__MISF_NAME__Debian GNU/Linux\n__MISF_VERSION__13\n__MISF_PRETTY__Debian GNU/Linux 13 (trixie)\n__MISF_PM__apt\n";
const DNF_HOST: &str = "Linux\n5.14.0\n__MISF_NAME__Rocky Linux\n__MISF_VERSION__9.5\n__MISF_PM__dnf\n__MISF_PM__yum\n";

#[derive(Default)]
struct State {
    auth_attempts: AtomicUsize,
    commands: Mutex<Vec<String>>,
    pty_count: AtomicUsize,
    detect: String,
    fail_update: bool,
    wrong_sudo: bool,
    /// Set once an upgrade ran: later listings report only the held-back package.
    upgraded: AtomicBool,
    /// Set once a dist-upgrade ran: later listings are empty.
    dist_upgraded: AtomicBool,
}

impl State {
    fn reply(&self, command: &str) -> (String, u32) {
        let upgraded = self.upgraded.load(Ordering::SeqCst);
        if command == package_manager::DETECT {
            (self.detect.clone(), 0)
        } else if command == apt::LIST {
            let held = "libc6/stable 2.36-9 amd64 [upgradable from: 2.36-8]\n";
            let listing = if self.dist_upgraded.load(Ordering::SeqCst) {
                "Listing...\n".into()
            } else if upgraded {
                format!("Listing...\n{held}")
            } else {
                format!(
                    "Listing...\nopenssl/stable-security 3.0.17 amd64 [upgradable from: 3.0.16]\n{held}"
                )
            };
            (listing, 0)
        } else if [dnf::LIST, dnf::SUDO_LIST].contains(&command) {
            // A long name wraps onto its own line, as yum and dnf do on narrow terminals.
            let held = "kernel-core-with-a-very-long-package-name.x86_64\r\n      5.14.0-503.el9   baseos\r\n";
            let fresh = if upgraded {
                ""
            } else {
                "openssl.x86_64   1:3.0.7-28.el9   baseos\r\n"
            };
            (
                format!(
                    "\r\nLast metadata expiration check: 0:00:01 ago.\r\n{fresh}{held}__MISF_INSTALLED__\r\nopenssl.x86_64 1:3.0.7-27.el9\r\nkernel-core-with-a-very-long-package-name.x86_64 5.14.0-427.el9\r\n"
                ),
                100,
            )
        } else if [apt::UPGRADE, apt::SUDO_UPGRADE].contains(&command) {
            self.upgraded.store(true, Ordering::SeqCst);
            // libc6 needs a new package, so a plain upgrade keeps it back.
            (
                "Setting up openssl (3.0.17) ...\r\nThe following packages have been kept back:\r\n  libc6\r\n1 upgraded, 0 newly installed, 0 to remove and 1 not upgraded.\r\n".into(),
                0,
            )
        } else if command == apt::PREVIEW {
            // openssl's update is held back for a phased rollout on this host.
            (
                "NOTE: This is only a simulation!
The following packages have been kept back:
  libc6
Not upgrading yet due to phasing:
  openssl

0 upgraded, 0 newly installed, 0 to remove and 2 not upgraded.
"
                .into(),
                0,
            )
        } else if command == apt::SIMULATE {
            (
                "NOTE: This is only a simulation!\nInst libc6 [2.36-8] (2.36-9 Debian:13/stable [amd64])\nInst libc6-extra (2.36-9 Debian:13/stable [amd64])\nRemv old-libc [1.0]\n".into(),
                0,
            )
        } else if [apt::DIST_UPGRADE, apt::SUDO_DIST_UPGRADE].contains(&command) {
            self.upgraded.store(true, Ordering::SeqCst);
            self.dist_upgraded.store(true, Ordering::SeqCst);
            (
                "Setting up openssl (3.0.17) ...\r\nSetting up libc6 (2.36-9) ...\r\n".into(),
                0,
            )
        } else if [dnf::UPGRADE, dnf::SUDO_UPGRADE].contains(&command) {
            self.upgraded.store(true, Ordering::SeqCst);
            ("Setting up openssl (3.0.17) ...\r\n".into(), 0)
        } else if [apt::UPDATE, apt::SUDO_UPDATE].contains(&command) {
            if self.fail_update {
                ("E: repository unavailable\n".into(), 100)
            } else {
                (
                    "0% [Working]\r\x1b[2K0% [Connecting]\r\x1b[32mHit:1 test-repo\x1b[0m\r\nReading package lists... Done\r\n".into(),
                    0,
                )
            }
        } else if [dnf::UPDATE, dnf::SUDO_UPDATE].contains(&command) {
            if self.fail_update {
                (
                    "Error: Failed to download metadata for repo 'baseos'\n".into(),
                    1,
                )
            } else {
                ("Metadata cache created.\r\n".into(), 0)
            }
        } else if [apt::STATUS, apt::SUDO_STATUS].contains(&command) {
            (
                "__MISF_REBOOT__\n__MISF_REBOOT_PKG__linux-image-7.0.0-43-generic\nNEEDRESTART-VER: 3.6\nNEEDRESTART-KCUR: 7.0.0-42-generic\nNEEDRESTART-KEXP: 7.0.0-43-generic\nNEEDRESTART-KSTA: 3\nNEEDRESTART-SVC: systemd-logind.service\nNEEDRESTART-SVC: dbus.service\n".into(),
                0,
            )
        } else if [reboot::ROOT, reboot::SUDO].contains(&command) {
            (
                "Reboot scheduled for Mon 2026-11-09 10:01:00 UTC, use 'shutdown -c' to cancel.\n"
                    .into(),
                0,
            )
        } else {
            ("unexpected command\n".into(), 127)
        }
    }
}

#[derive(Clone)]
struct Mock {
    state: Arc<State>,
    /// Commands waiting for their sudo password, by channel.
    waiting: HashMap<ChannelId, String>,
}
impl server::Server for Mock {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        Self {
            state: self.state.clone(),
            waiting: HashMap::new(),
        }
    }
}

fn finish(
    session: &mut Session,
    channel: ChannelId,
    text: &str,
    code: u32,
) -> Result<(), russh::Error> {
    session.data(channel, text.as_bytes().to_vec())?;
    session.exit_status_request(channel, code)?;
    session.eof(channel)?;
    session.close(channel)
}

impl server::Handler for Mock {
    type Error = russh::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        self.state.auth_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(
            if ["operator", "root"].contains(&user) && password == "ssh-secret" {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            },
        )
    }
    async fn auth_publickey(
        &mut self,
        user: &str,
        _: VerifiedPublicKey<'_>,
    ) -> Result<server::Auth, Self::Error> {
        self.state.auth_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "operator" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        assert!(modes.contains(&(Pty::ECHO, 0)));
        self.state.pty_count.fetch_add(1, Ordering::SeqCst);
        session.channel_success(channel)
    }
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        self.state.commands.lock().unwrap().push(command.clone());
        session.channel_success(channel)?;
        if command.contains("sudo -S") {
            self.waiting.insert(channel, command);
            // Deliberately split the sudo marker across SSH packets.
            session.data(channel, b"__MISF_SUDO_".to_vec())?;
            session.data(channel, b"PROMPT__".to_vec())
        } else {
            let (text, code) = self.state.reply(&command);
            finish(session, channel, &text, code)
        }
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(command) = self.waiting.remove(&channel) {
            assert_eq!(data, b"sudo-secret\n");
            if self.state.wrong_sudo {
                session.data(channel, b"Sorry, try again.\n__MISF_SUDO_PROMPT__".to_vec())
            } else {
                let (text, code) = self.state.reply(&command);
                finish(session, channel, &text, code)
            }
        } else {
            Ok(())
        }
    }
}

struct Fixture {
    host: Host,
    state: Arc<State>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fixture(fail_update: bool, wrong_sudo: bool) -> Fixture {
    fixture_on(APT_HOST, fail_update, wrong_sudo).await
}

async fn fixture_on(detect: &str, fail_update: bool, wrong_sudo: bool) -> Fixture {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(State {
        detect: detect.into(),
        fail_update,
        wrong_sudo,
        ..Default::default()
    });
    let mut server = Mock {
        state: state.clone(),
        waiting: HashMap::new(),
    };
    let task = tokio::spawn(async move {
        server.run_on_socket(config, &listener).await.unwrap();
    });
    let host = Host {
        id: "fixture".into(),
        name: "Test Linux".into(),
        address: "127.0.0.1".into(),
        port,
        username: "operator".into(),
        os: "Linux".into(),
        auth_type: "password".into(),
        password: "ssh-secret".into(),
        sudo_password: "sudo-secret".into(),
        use_pty: true,
        fingerprint,
        package_manager: package_manager::parse_platform(detect)
            .unwrap()
            .manager
            .map(Manager::id)
            .unwrap_or_default()
            .into(),
        ..Default::default()
    };
    Fixture { host, state, task }
}

#[tokio::test]
async fn password_pty_sudo_and_packages_work_end_to_end() {
    let fixture = fixture(false, false).await;
    let test = ssh::test_connection(&fixture.host).await;
    assert_eq!(test.status, "success", "{}", test.error);
    assert_eq!(test.kernel, "Linux");
    assert_eq!(test.distro, "Debian 13");
    assert_eq!(test.package_managers, ["apt"]);
    assert!(
        test.server_version.starts_with("SSH-2.0-"),
        "{}",
        test.server_version
    );
    assert!(!test.kex.is_empty() && !test.cipher.is_empty() && !test.client_mac.is_empty());
    assert_eq!(test.host_key_algorithm, "ssh-ed25519");
    assert_eq!(test.host_key_fingerprint, fixture.host.fingerprint);
    assert_eq!(test.auth_method, "password");
    assert!(test.client_key_algorithm.is_empty());
    let check = package_manager::check(&fixture.host, &|_| {}).await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert_eq!(check.packages.len(), 2);
    assert_eq!(check.packages[0].current_version, "3.0.16");
    assert_eq!(check.packages[0].candidate_version, "3.0.17");
    assert_eq!(check.distro, "Debian 13");
    assert_eq!(check.package_manager, "apt");
    // The simulated upgrade finds what phasing defers, but a check reports no
    // dist-upgrade: kept-back packages are only reported after a real upgrade.
    assert_eq!(check.phased, ["openssl"]);
    assert!(check.kept_back.is_empty() && check.plan.is_none());
    assert!(check.output.contains("APT upgrade 模擬"));
    // The index update and the restart status both go through sudo with a PTY.
    assert_eq!(fixture.state.pty_count.load(Ordering::SeqCst), 2);
    assert!(!check.output.contains("sudo-secret"));
    assert!(!check.output.contains("__MISF_SUDO_PROMPT__"));
    assert!(!check.output.contains(['\r', '\x1b']));
    assert!(
        check
            .output
            .contains("0% [Working]\n0% [Connecting]\nHit:1 test-repo\n")
    );
    let commands = fixture.state.commands.lock().unwrap();
    assert!(
        commands.iter().any(
            |command| command.contains("-- apt-get -q=1 -o APT::Update::Error-Mode=any update")
        )
    );
    assert!(
        commands
            .iter()
            .all(|c| !c.contains("ssh-secret") && !c.contains("sudo-secret"))
    );
    // Nothing but the read-only simulation names an upgrade, and it runs without sudo.
    assert!(
        !commands
            .iter()
            .filter(|c| *c != apt::PREVIEW)
            .any(|c| c.contains(" upgrade") || c.contains(" install"))
    );
    // The check ran exactly what its confirmation lists, after the connection test.
    let expected = package_manager::package_commands(&fixture.host, Mode::Check);
    assert_eq!(commands[commands.len() - expected.len()..], expected[..]);
}

#[tokio::test]
async fn sudo_upgrade_reports_installed_and_held_back_packages() {
    let fixture = fixture(false, false).await;
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert_eq!(result.operation, "upgrade");
    assert_eq!(result.upgraded.len(), 1);
    assert_eq!(result.upgraded[0].name, "openssl");
    assert_eq!(result.packages.len(), 1);
    assert_eq!(result.packages[0].name, "libc6");
    assert!(result.output.contains("Setting up openssl"));
    assert!(!result.output.contains("sudo-secret"));
    // Update, upgrade and restart status use sudo; listings and the simulation do not.
    assert_eq!(fixture.state.pty_count.load(Ordering::SeqCst), 3);
    // libc6 was kept back, so the dist-upgrade plan was simulated, changing nothing.
    assert_eq!(result.kept_back, ["libc6"]);
    let plan = result.plan.as_ref().unwrap();
    assert_eq!(plan.upgrade[0].name, "libc6");
    assert_eq!(plan.install[0].name, "libc6-extra");
    assert_eq!(plan.remove[0].name, "old-libc");
    // Section titles are boxed to their display width: five CJK characters are ten columns.
    assert!(
        result
            .output
            .contains("\n**************\n* 待更新套件 *\n**************\n")
    );
    let commands = fixture.state.commands.lock().unwrap();
    assert_eq!(
        *commands,
        package_manager::package_commands(&fixture.host, Mode::Upgrade)
    );
    assert!(commands.iter().any(|c| c == apt::SIMULATE));
    assert!(
        !commands
            .iter()
            .any(|c| c.contains("dist-upgrade") && c != apt::SIMULATE)
    );
    assert!(
        commands
            .iter()
            .all(|c| !c.contains("ssh-secret") && !c.contains("sudo-secret"))
    );
}

#[tokio::test]
async fn root_upgrade_runs_without_sudo() {
    let mut fixture = fixture(false, false).await;
    fixture.host.username = "root".into();
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert_eq!(result.upgraded.len(), 1);
    let commands = fixture.state.commands.lock().unwrap();
    assert!(commands.iter().any(|c| c == apt::UPGRADE));
    assert!(!commands.iter().any(|c| c.contains("sudo")));
    // Restart status is read last, after the upgrade and the second listing.
    assert_eq!(
        *commands,
        package_manager::package_commands(&fixture.host, Mode::Upgrade)
    );
    assert_eq!(commands.last().unwrap(), apt::STATUS);
    let restart = result.restart.unwrap();
    assert!(restart.reboot_required && restart.needrestart);
    assert_eq!(restart.running_kernel, "7.0.0-42-generic");
    assert_eq!(restart.expected_kernel, "7.0.0-43-generic");
    assert_eq!(restart.services, ["systemd-logind.service", "dbus.service"]);
    assert!(result.output.contains("NEEDRESTART-SVC: dbus.service"));
}

#[tokio::test]
async fn regular_accounts_read_restart_status_through_sudo() {
    let fixture = fixture(false, false).await;
    let result = package_manager::check(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert!(result.restart.unwrap().reboot_required);
    let commands = fixture.state.commands.lock().unwrap();
    assert_eq!(commands.last().unwrap(), apt::SUDO_STATUS);
    assert!(!commands.iter().any(|c| c == apt::STATUS));
}

#[tokio::test]
async fn dist_upgrade_runs_only_its_own_command_and_clears_kept_back_packages() {
    let fixture = fixture(false, false).await;
    let result = package_manager::dist_upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert_eq!(result.operation, "dist_upgrade");
    assert!(result.is_upgrade());
    assert_eq!(result.upgraded.len(), 2);
    assert!(result.packages.is_empty() && result.kept_back.is_empty());
    assert!(result.plan.is_none());
    assert!(result.output.contains("* APT dist-upgrade *"));
    // Nothing was kept back, so no simulation ran.
    assert_eq!(
        *fixture.state.commands.lock().unwrap(),
        [
            package_manager::DETECT,
            apt::SUDO_UPDATE,
            apt::LIST,
            apt::SUDO_DIST_UPGRADE,
            apt::LIST,
            apt::SUDO_STATUS,
        ]
    );

    // Other managers have no dist-upgrade and stop before touching packages.
    let rocky = fixture_on(DNF_HOST, false, false).await;
    let result = package_manager::dist_upgrade(&rocky.host, &|_| {}).await;
    assert_eq!(result.status, "failed");
    assert!(result.error.contains("dist-upgrade"), "{}", result.error);
    assert_eq!(
        *rocky.state.commands.lock().unwrap(),
        [package_manager::DETECT]
    );
}

#[tokio::test]
async fn reboot_runs_one_fixed_command_and_only_on_apt_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.db"));
    let fixture = fixture(false, false).await;
    let id = register(&store, &fixture, "debian").await;
    fixture.state.commands.lock().unwrap().clear();
    let response = dispatch(json!({"action":"reboot","id":id}), &store, &|_| {})
        .await
        .unwrap();
    assert!(response["output"].as_str().unwrap().contains("shutdown -c"));
    assert_eq!(*fixture.state.commands.lock().unwrap(), [reboot::SUDO]);

    let mut root = fixture.host.clone();
    root.username = "root".into();
    reboot::reboot(&root, &|_| {}).await.unwrap();
    assert_eq!(
        fixture.state.commands.lock().unwrap().last().unwrap(),
        reboot::ROOT
    );

    let rocky = fixture_on(DNF_HOST, false, false).await;
    let error = reboot::reboot(&rocky.host, &|_| {}).await.unwrap_err();
    assert!(error.to_string().contains("APT"), "{error}");
    assert!(rocky.state.commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_index_update_never_starts_an_upgrade() {
    let fixture = fixture(true, false).await;
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "failed");
    assert!(result.upgraded.is_empty());
    let commands = fixture.state.commands.lock().unwrap();
    assert!(!commands.iter().any(|c| c.contains(" upgrade")));
}

#[tokio::test]
async fn sudo_dnf_upgrade_parses_wrapped_rows_and_reports_held_back_packages() {
    let fixture = fixture_on(DNF_HOST, false, false).await;
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert_eq!(result.distro, "Rocky Linux 9.5");
    assert_eq!(result.package_manager, "dnf");
    assert_eq!(result.upgraded.len(), 1);
    assert_eq!(result.upgraded[0].name, "openssl");
    assert_eq!(result.upgraded[0].current_version, "1:3.0.7-27.el9");
    assert_eq!(result.upgraded[0].candidate_version, "1:3.0.7-28.el9");
    assert_eq!(result.packages.len(), 1);
    assert_eq!(
        result.packages[0].name,
        "kernel-core-with-a-very-long-package-name"
    );
    assert_eq!(result.packages[0].current_version, "5.14.0-427.el9");
    // Update, both listings and the upgrade all go through sudo.
    assert_eq!(fixture.state.pty_count.load(Ordering::SeqCst), 4);
    let commands = fixture.state.commands.lock().unwrap();
    assert_eq!(
        *commands,
        package_manager::package_commands(&fixture.host, Mode::Upgrade)
    );
    assert!(commands[1..].iter().all(|c| !c.contains("apt")));
}

#[tokio::test]
async fn recorded_manager_mismatch_and_unsupported_systems_never_touch_packages() {
    let mut fixture = fixture_on(DNF_HOST, false, false).await;
    fixture.host.package_manager = "apt".into();
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "failed");
    assert!(result.error.contains("測試連線"), "{}", result.error);
    assert_eq!(
        *fixture.state.commands.lock().unwrap(),
        [package_manager::DETECT]
    );
    let mut fixture = fixture_on("OpenBSD\n7.6\n", false, false).await;
    fixture.host.package_manager.clear();
    let result = package_manager::check(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "unsupported");
    assert_eq!(result.distro, "OpenBSD 7.6");
    assert_eq!(
        *fixture.state.commands.lock().unwrap(),
        [package_manager::DETECT]
    );
}

#[tokio::test]
async fn chosen_package_manager_overrides_detection_only_when_installed() {
    // Rocky has dnf and yum; detection prefers dnf, but the user chose yum.
    let mut fixture = fixture_on(DNF_HOST, false, false).await;
    fixture.host.package_manager = "dnf".into();
    fixture.host.manager_choice = "yum".into();
    let result = package_manager::check(&fixture.host, &|_| {}).await;
    let commands = fixture.state.commands.lock().unwrap().clone();
    let expected = package_manager::package_commands(&fixture.host, Mode::Check);
    assert!(expected[1].contains("yum"), "{expected:?}");
    assert_eq!(commands[..2], expected[..2], "{}", result.error);
    // The detected manager is still recorded as detected.
    assert_eq!(result.package_manager, "dnf");
    let mut fixture = fixture_on(DNF_HOST, false, false).await;
    fixture.host.manager_choice = "apt".into();
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "failed");
    assert!(result.error.contains("找不到所選"), "{}", result.error);
    assert_eq!(
        *fixture.state.commands.lock().unwrap(),
        [package_manager::DETECT]
    );
}

/// Saves, trusts and detects a host the way the UI does before batching it.
async fn register(store: &Store, fixture: &Fixture, name: &str) -> String {
    let mut host = fixture.host.clone();
    host.id.clear();
    host.name = name.into();
    let saved = dispatch(json!({"action":"save","host":host}), store, &|_| {})
        .await
        .unwrap();
    let id = saved["id"].as_str().unwrap().to_owned();
    assert!(store.host(&id).unwrap().package_manager.is_empty());
    dispatch(
        json!({"action":"trust","id":id,"fingerprint":fixture.host.fingerprint}),
        store,
        &|_| {},
    )
    .await
    .unwrap();
    dispatch(json!({"action":"test","id":id}), store, &|_| {})
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn batches_reject_mixed_or_undetected_package_managers() {
    let (apt, dnf) = (
        fixture(false, false).await,
        fixture_on(DNF_HOST, false, false).await,
    );
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.db"));
    let debian = register(&store, &apt, "debian").await;
    let rocky = register(&store, &dnf, "rocky").await;
    let saved = store.host(&rocky).unwrap();
    assert_eq!(
        (saved.distro.as_str(), saved.package_manager.as_str()),
        ("Rocky Linux 9.5", "dnf")
    );
    let mut fresh = apt.host.clone();
    fresh.id.clear();
    let fresh = dispatch(json!({"action":"save","host":fresh}), &store, &|_| {})
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for ids in [[&debian, &rocky], [&debian, &fresh]] {
        let error = dispatch(
            json!({"action":"batch","operation":"check","ids":ids}),
            &store,
            &|_| {},
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("同一種"), "{error}");
    }
    assert!(
        !apt.state
            .commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains("update"))
    );
    let response = dispatch(
        json!({"action":"batch","operation":"check","ids":[rocky]}),
        &store,
        &|_| {},
    )
    .await
    .unwrap();
    assert_eq!(response["results"][0]["result"]["status"], "success");
    assert_eq!(
        store
            .host(&rocky)
            .unwrap()
            .last_check
            .unwrap()
            .packages
            .len(),
        2
    );
}

#[tokio::test]
async fn batch_runs_each_host_and_saves_per_host_results() {
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.db"));
    let mut ids = vec![];
    for name in ["first", "second", "changed"] {
        ids.push(register(&store, &fixture, name).await);
    }
    // A key that changed after detection fails only that host.
    dispatch(
        json!({"action":"trust","id":ids[2],"fingerprint":"SHA256:changed-fingerprint-0000000"}),
        &store,
        &|_| {},
    )
    .await
    .unwrap();
    let progress = Mutex::new(vec![]);
    let response = dispatch(
        json!({"action":"batch","operation":"check","ids":[ids[0], ids[1], ids[2], ids[0]]}),
        &store,
        &|message| progress.lock().unwrap().push(message.to_owned()),
    )
    .await
    .unwrap();
    let results = response["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    for (index, id) in ids.iter().enumerate() {
        let saved = store.host(id).unwrap().last_check.unwrap();
        assert_eq!(saved.operation, "check");
        assert_eq!(saved.status, if index < 2 { "success" } else { "failed" });
        assert!(results.iter().any(|entry| entry["id"] == id.as_str()));
    }
    assert_eq!(progress.lock().unwrap().last().unwrap(), "批次作業進度 3/3");
    assert!(
        !fixture
            .state
            .commands
            .lock()
            .unwrap()
            .iter()
            // Only the read-only simulation, which finds phased updates, names one.
            .any(|c| c.contains(" upgrade") && c != apt::PREVIEW)
    );
    for request in [
        json!({"action":"batch","operation":"install","ids":[ids[0]]}),
        json!({"action":"batch","operation":"check","ids":[]}),
        json!({"action":"batch","operation":"check","ids":["missing"]}),
    ] {
        assert!(dispatch(request, &store, &|_| {}).await.is_err());
    }
}

/// Records what a batch reports and answers every failure the same way.
struct Recorder {
    decision: Decision,
    finished: Mutex<Vec<(String, String)>>,
    failed: Mutex<Vec<String>>,
}
impl batch::Observer for Recorder {
    fn finished(&self, id: &str, result: &CheckResult) {
        self.finished
            .lock()
            .unwrap()
            .push((id.into(), result.status.clone()));
    }
    fn failed<'a>(&'a self, id: &str, _: &CheckResult) -> batch::Asking<'a> {
        self.failed.lock().unwrap().push(id.into());
        let decision = self.decision;
        Box::pin(async move { decision })
    }
}

#[tokio::test]
async fn batch_reports_each_host_and_an_abort_skips_hosts_not_started() {
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.db"));
    let mut ids = vec![];
    for index in 0..12 {
        ids.push(register(&store, &fixture, &format!("host-{index}")).await);
    }
    // The first host fails fast on its changed key, while the rest are still checking.
    dispatch(
        json!({"action":"trust","id":ids[0],"fingerprint":"SHA256:changed-fingerprint-0000000"}),
        &store,
        &|_| {},
    )
    .await
    .unwrap();
    let recorder = Recorder {
        decision: Decision::Abort,
        finished: Mutex::new(vec![]),
        failed: Mutex::new(vec![]),
    };
    let response = dispatch_with(
        json!({"action":"batch","operation":"check","ids":ids}),
        &store,
        &|_| {},
        &recorder,
    )
    .await
    .unwrap();
    let results = response["results"].as_array().unwrap();
    let skipped = response["skipped"].as_array().unwrap();
    assert!(!skipped.is_empty());
    assert_eq!(results.len() + skipped.len(), ids.len());
    assert_eq!(*recorder.failed.lock().unwrap(), [ids[0].clone()]);
    // Every reported host was already saved when it was reported; skipped ones never ran.
    let finished = recorder.finished.lock().unwrap();
    assert_eq!(finished.len(), results.len());
    for (id, status) in finished.iter() {
        assert_eq!(store.host(id).unwrap().last_check.unwrap().status, *status);
    }
    for id in skipped {
        assert!(
            store
                .host(id.as_str().unwrap())
                .unwrap()
                .last_check
                .is_none()
        );
    }
}

#[tokio::test]
async fn imported_private_key_authentication_works() {
    let mut fixture = fixture(false, false).await;
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    fixture.host.auth_type = "key".into();
    fixture.host.private_key = key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .unwrap()
        .to_string();
    fixture.host.password.clear();
    fixture.host.validate().unwrap();
    let test = ssh::test_connection(&fixture.host).await;
    assert_eq!(
        (test.status.as_str(), test.kernel.as_str()),
        ("success", "Linux")
    );
    assert_eq!(test.auth_method, "publickey");
    assert_eq!(test.client_key_algorithm, "ssh-ed25519");
    assert!(test.client_key_fingerprint.starts_with("SHA256:"));
}

#[tokio::test]
async fn root_check_executes_apt_without_sudo() {
    let mut fixture = fixture(false, false).await;
    fixture.host.username = "root".into();
    let result = package_manager::check(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    let commands = fixture.state.commands.lock().unwrap();
    assert!(
        commands
            .iter()
            .any(|command| command.starts_with("LC_ALL=C apt-get "))
    );
    assert!(!commands.iter().any(|command| command.contains("sudo")));
    assert!(commands.iter().any(|command| command == "LC_ALL=C apt-get -q=1 -o APT::Update::Error-Mode=any update"));
}

#[tokio::test]
async fn probe_never_submits_credentials_and_changed_key_blocks_authentication() {
    let fixture = fixture(false, false).await;
    assert_eq!(
        ssh::probe(&fixture.host).await.unwrap(),
        fixture.host.fingerprint
    );
    assert_eq!(fixture.state.auth_attempts.load(Ordering::SeqCst), 0);
    let mut changed = fixture.host.clone();
    changed.fingerprint = "SHA256:wrong".into();
    let test = ssh::test_connection(&changed).await;
    assert_eq!(test.status, "failed");
    assert!(test.error.contains("指紋已變更"), "{}", test.error);
    // What the server actually presented is kept, and no sign-in was recorded.
    assert_eq!(test.host_key_fingerprint, fixture.host.fingerprint);
    assert!(test.auth_method.is_empty() && test.distro.is_empty());
    assert_eq!(fixture.state.auth_attempts.load(Ordering::SeqCst), 0);
    changed.fingerprint.clear();
    let test = ssh::test_connection(&changed).await;
    assert!(test.error.contains("先確認"), "{}", test.error);
}

#[tokio::test]
async fn apt_update_failure_never_lists_stale_packages() {
    let fixture = fixture(true, false).await;
    let check = package_manager::check(&fixture.host, &|_| {}).await;
    assert_eq!(check.status, "failed");
    assert!(check.packages.is_empty());
    assert!(check.error.contains("索引失敗"));
    assert!(
        !fixture
            .state
            .commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| c == apt::LIST)
    );
}

#[tokio::test]
async fn repeated_sudo_prompt_fails_without_repeated_password_submission() {
    let fixture = fixture(false, true).await;
    let check = package_manager::check(&fixture.host, &|_| {}).await;
    assert_eq!(check.status, "failed");
    assert!(check.error.contains("sudo 密碼錯誤"), "{}", check.error);
}

#[tokio::test]
async fn failed_authentication_is_reported_without_package_results() {
    let fixture = fixture(false, false).await;
    let mut wrong = fixture.host.clone();
    wrong.password = "wrong".into();
    let test = ssh::test_connection(&wrong).await;
    assert_eq!(test.status, "failed");
    assert!(!test.kex.is_empty() && test.auth_method.is_empty());
    assert!(!test.error.contains("wrong"), "{}", test.error);
    let check = package_manager::check(&wrong, &|_| {}).await;
    assert_eq!(check.status, "failed");
    assert!(check.packages.is_empty() && check.distro.is_empty());
    assert!(fixture.state.commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn persistence_saves_check_and_resets_trust_when_endpoint_changes() {
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.db"));
    let saved = dispatch(
        json!({"action":"save", "host":fixture.host}),
        &store,
        &|_| {},
    )
    .await
    .unwrap();
    let id = saved["id"].as_str().unwrap();
    assert!(store.host(id).unwrap().fingerprint.is_empty());
    dispatch(
        json!({"action":"trust","id":id,"fingerprint":fixture.host.fingerprint}),
        &store,
        &|_| {},
    )
    .await
    .unwrap();
    let result = dispatch(json!({"action":"check","id":id}), &store, &|_| {})
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
    let mut updated = store.host(id).unwrap();
    assert_eq!(updated.last_check.as_ref().unwrap().packages.len(), 2);
    assert_eq!(updated.distro, "Debian 13");
    // The editor cannot overwrite detection results.
    updated.package_manager = "pkg".into();
    dispatch(
        json!({"action":"save","host":updated.clone()}),
        &store,
        &|_| {},
    )
    .await
    .unwrap();
    assert_eq!(store.host(id).unwrap().package_manager, "apt");
    updated.port = 1;
    dispatch(json!({"action":"save","host":updated}), &store, &|_| {})
        .await
        .unwrap();
    assert!(store.host(id).unwrap().fingerprint.is_empty());
    assert!(store.host(id).unwrap().package_manager.is_empty());
    assert!(store.host(id).unwrap().last_check.is_none());
    dispatch(json!({"action":"delete","id":id}), &store, &|_| {})
        .await
        .unwrap();
    assert!(store.read().unwrap().hosts.is_empty());
}

#[test]
fn malformed_store_is_preserved_and_unparseable_apt_output_is_not_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.db");
    std::fs::write(&path, "damaged-data").unwrap();
    let store = Store::new(path.clone());
    assert!(
        store
            .update(|db| {
                db.hosts.clear();
                Ok(())
            })
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "damaged-data");
    assert!(apt::parse_pending("Listing...\n").unwrap().is_empty());
    assert!(apt::parse_pending("unexpected localized output").is_err());
    assert!(apt::parse_pending("package/repo missing-fields").is_err());
}

#[tokio::test]
async fn ssh_apt_results_roundtrip_inside_luksbox_and_remain_cli_compatible() {
    use luksbox_core::{Argon2idParams, CipherSuite};
    use luksbox_format::{Container, UnlockMaterial};
    use luksbox_vfs::Vfs;
    use misf_core::{store::Database, vault::StorageSession};
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.lbx");
    let password = b"integration-test-passphrase";
    {
        let _size =
            luksbox_format::metadata::set_create_metadata_region_size_override(Some(64 * 1024));
        let params = Argon2idParams {
            m_cost_kib: 8,
            t_cost: 1,
            p_cost: 1,
        };
        let mut vfs = Vfs::open(
            Container::create_with_passphrase(
                &path,
                None,
                CipherSuite::Aes256GcmSiv,
                params,
                password,
            )
            .unwrap(),
        )
        .unwrap();
        let file = vfs.create(vfs.root_id(), "hosts.json").unwrap();
        let bytes = serde_json::to_vec(&Database {
            version: 1,
            hosts: vec![fixture.host.clone()],
            ..Default::default()
        })
        .unwrap();
        vfs.write(file, 0, &bytes).unwrap();
        vfs.finish().unwrap();
    }
    let mut session = StorageSession::new(dir.path().join("hosts.db"));
    session
        .handle(
            json!({"action":"vault_unlock","passphrase":std::str::from_utf8(password).unwrap()}),
            &|_| {},
        )
        .await
        .unwrap();
    let listed = session
        .handle(json!({"action":"list"}), &|_| {})
        .await
        .unwrap();
    assert_eq!(
        (listed["format"].clone(), listed["schema_version"].clone()),
        (json!("json"), json!(null))
    );
    let converted = session
        .handle(json!({"action":"convert_storage"}), &|_| {})
        .await
        .unwrap();
    assert_eq!(
        converted,
        json!({"format":"sqlite","schema_version":misf_core::sqlite::SCHEMA_VERSION})
    );
    let listed = session
        .handle(json!({"action":"list"}), &|_| {})
        .await
        .unwrap();
    assert_eq!(listed["hosts"][0]["id"], json!(fixture.host.id));
    let result = session
        .handle(json!({"action":"check","id":fixture.host.id}), &|_| {})
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["packages"].as_array().unwrap().len(), 2);
    let listed = session
        .handle(json!({"action":"list"}), &|_| {})
        .await
        .unwrap();
    assert_eq!(
        (listed["format"].clone(), listed["schema_version"].clone()),
        (json!("sqlite"), json!(misf_core::sqlite::SCHEMA_VERSION))
    );
    session
        .handle(json!({"action":"vault_lock"}), &|_| {})
        .await
        .unwrap();
    assert!(!dir.path().join("hosts.db").exists());
    assert!(!dir.path().join("hosts.json").exists());
    let mut vfs =
        Vfs::open(Container::open(&path, None, UnlockMaterial::Passphrase(password)).unwrap())
            .unwrap();
    // Converting replaced the legacy JSON with a stock SQLite image.
    assert!(vfs.lookup_path("/hosts.json").is_err());
    let file = vfs.lookup_path("/hosts.db").unwrap();
    let mut bytes = vec![0; vfs.stat(file).unwrap().size as usize];
    vfs.read(file, 0, &mut bytes).unwrap();
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), true)
        .unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, misf_core::sqlite::SCHEMA_VERSION as i64);
    let pending: i64 = conn
        .query_row(
            "SELECT count(*) FROM packages WHERE host_id = ?1 AND upgraded = 0",
            [&fixture.host.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 2);
}

#[tokio::test]
async fn hosts_share_a_stored_key_that_cannot_be_deleted_while_in_use() {
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.db"));
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let pem = key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .unwrap()
        .to_string();
    let added = dispatch(
        json!({"action":"key_add","name":"deploy","private_key":pem,"passphrase":""}),
        &store,
        &|_| {},
    )
    .await
    .unwrap();
    let key_id = added["key"]["id"].as_str().unwrap().to_owned();
    assert_eq!(added["key"]["algorithm"], "ssh-ed25519");
    // The same key twice is refused; a broken one never gets stored.
    for request in [
        json!({"action":"key_add","name":"again","private_key":pem}),
        json!({"action":"key_add","name":"bad","private_key":"not a key"}),
        json!({"action":"key_add","name":"","private_key":pem}),
    ] {
        assert!(dispatch(request, &store, &|_| {}).await.is_err());
    }
    let mut host = fixture.host.clone();
    host.id.clear();
    host.auth_type = "key".into();
    host.key_id = "missing".into();
    assert!(
        dispatch(json!({"action":"save","host":host}), &store, &|_| {})
            .await
            .is_err()
    );
    host.key_id = key_id.clone();
    host.private_key = "inline copy that must be dropped".into();
    let saved = dispatch(json!({"action":"save","host":host}), &store, &|_| {})
        .await
        .unwrap();
    let id = saved["id"].as_str().unwrap().to_owned();
    dispatch(
        json!({"action":"trust","id":id,"fingerprint":fixture.host.fingerprint}),
        &store,
        &|_| {},
    )
    .await
    .unwrap();
    // The list carries names and fingerprints only, never the key itself.
    let list = dispatch(json!({"action":"list"}), &store, &|_| {})
        .await
        .unwrap()
        .to_string();
    assert!(!list.contains("OPENSSH PRIVATE KEY") && !list.contains("inline copy"));
    assert!(list.contains("deploy"));
    dispatch(json!({"action":"test","id":id}), &store, &|_| {})
        .await
        .unwrap();
    let error = dispatch(json!({"action":"key_delete","id":key_id}), &store, &|_| {})
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains(&fixture.host.name),
        "{error:#}"
    );
    let mut host = store.read().unwrap().hosts[0].clone();
    host.auth_type = "password".into();
    host.password = "pw".into();
    dispatch(json!({"action":"save","host":host}), &store, &|_| {})
        .await
        .unwrap();
    assert!(store.read().unwrap().hosts[0].key_id.is_empty());
    dispatch(json!({"action":"key_delete","id":key_id}), &store, &|_| {})
        .await
        .unwrap();
    assert!(store.read().unwrap().keys.is_empty());
}

#[tokio::test]
async fn hosts_are_dragged_between_groups_and_survive_group_deletion() {
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.db"));
    let run = |request: serde_json::Value| {
        let store = &store;
        async move { dispatch(request, store, &|_| {}).await }
    };
    let mut ids = Vec::new();
    for name in ["web", "db"] {
        let mut host = fixture.host.clone();
        host.id.clear();
        host.name = name.into();
        host.group_id = "made-up".into();
        let saved = run(json!({"action":"save","host":host})).await.unwrap();
        // A new host only joins a group that exists.
        assert_eq!(saved["group_id"], "");
        ids.push(saved["id"].as_str().unwrap().to_owned());
    }
    let group = run(json!({"action":"group_add","name":"  prod  "}))
        .await
        .unwrap();
    let group_id = group["id"].as_str().unwrap().to_owned();
    assert_eq!(group["name"], "prod");
    assert!(run(json!({"action":"group_add","name":" "})).await.is_err());
    // db moves first, into the group; a stale or partial order is refused whole.
    let arrange =
        |hosts: serde_json::Value| json!({"action":"arrange","groups":[group_id],"hosts":hosts});
    for stale in [
        json!([{"id": ids[1], "group_id": group_id}]),
        json!([{"id": ids[1], "group_id": "nope"}, {"id": ids[0], "group_id": ""}]),
    ] {
        assert!(run(arrange(stale)).await.is_err());
    }
    run(arrange(
        json!([{"id": ids[1], "group_id": group_id}, {"id": ids[0], "group_id": ""}]),
    ))
    .await
    .unwrap();
    run(json!({"action":"group_rename","id":group_id,"name":"production"}))
        .await
        .unwrap();
    run(json!({"action":"group_collapse","id":group_id,"collapsed":true}))
        .await
        .unwrap();
    // Editing a host leaves it in the group it was dragged to.
    let mut edited = store.read().unwrap().hosts[0].clone();
    edited.group_id.clear();
    edited.notes = "edited".into();
    run(json!({"action":"save","host":edited})).await.unwrap();
    let listed = run(json!({"action":"list"})).await.unwrap();
    let names: Vec<_> = listed["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| (h["name"].as_str().unwrap(), h["group_id"].as_str().unwrap()))
        .collect();
    assert_eq!(names, [("db", group_id.as_str()), ("web", "")]);
    assert_eq!(listed["groups"][0]["name"], "production");
    assert_eq!(listed["groups"][0]["collapsed"], true);
    run(json!({"action":"group_delete","id":group_id}))
        .await
        .unwrap();
    let database = store.read().unwrap();
    assert!(database.groups.is_empty());
    assert_eq!(database.hosts[0].name, "db");
    assert_eq!(database.hosts[0].group_id, "");
}
