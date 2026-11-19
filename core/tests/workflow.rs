use misf_core::{
    batch::{self, Decision},
    console::{Console, Echo},
    dispatch, dispatch_with,
    model::{CheckResult, Host},
    package_manager::{self, Manager, Mode, apk, apt, dnf, pacman},
    reboot, shell, ssh,
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
const DNF_HOST: &str =
    "Linux\n5.14.0\n__MISF_NAME__Rocky Linux\n__MISF_VERSION__9.5\n__MISF_PM__dnf\n";
const PACMAN_HOST: &str = "Linux\n6.17.2-arch1-1\n__MISF_NAME__Arch Linux\n__MISF_VERSION__\n__MISF_PRETTY__Arch Linux\n__MISF_PM__pacman\n";
/// Alpine with a Debian-style apt wrapper would never happen; this host has both so a
/// choice in the editor has something to pick.
const APK_HOST: &str = "Linux\n6.12.0\n__MISF_NAME__Alpine Linux\n__MISF_VERSION__3.24.4\n__MISF_PM__apt\n__MISF_PM__apk\n";

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
    /// Interactive shells asked for: the terminal type and size of each PTY.
    shell_ptys: Mutex<Vec<(String, u32, u32)>>,
    /// Window size changes, as (columns, rows).
    resizes: Mutex<Vec<(u32, u32)>>,
    /// Upgrades never finish once `free_upgrades` have: a host stuck mid-upgrade.
    hang_upgrades: AtomicBool,
    free_upgrades: AtomicUsize,
    /// Index updates never finish: a check stuck on a slow mirror.
    hang_updates: AtomicBool,
    /// Upgrade output prints the sudo prompt marker, as a package script could.
    print_marker: AtomicBool,
    /// Input a command channel got that no sudo prompt asked for.
    stray_input: Mutex<Vec<u8>>,
}

impl State {
    /// Whether this command is left running forever.
    fn hangs(&self, command: &str) -> bool {
        if self.hang_updates.load(Ordering::SeqCst)
            && [apt::UPDATE, apt::SUDO_UPDATE].contains(&command)
        {
            return true;
        }
        self.hang_upgrades.load(Ordering::SeqCst)
            && [apt::UPGRADE, apt::SUDO_UPGRADE].contains(&command)
            && self
                .free_upgrades
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_err()
    }

    fn reply(&self, command: &str) -> (String, u32) {
        let upgraded = self.upgraded.load(Ordering::SeqCst);
        if command == package_manager::DETECT {
            (self.detect.clone(), 0)
        } else if command == misf_core::apt_lock::SNAPSHOT {
            // An idle host: no apt or dpkg running, nothing half-configured.
            (
                "__MISF_PS__
__MISF_JOBS__
__MISF_AUDIT__
"
                .into(),
                0,
            )
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
            // A long name wraps onto its own line, as dnf does on narrow terminals.
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
            let marker = if self.print_marker.load(Ordering::SeqCst) {
                "postinst says __MISF_SUDO_PROMPT__\r\n"
            } else {
                ""
            };
            // libc6 needs a new package, so a plain upgrade keeps it back.
            (
                format!(
                    "{marker}Setting up openssl (3.0.17) ...\r\nThe following packages have been kept back:\r\n  libc6\r\n1 upgraded, 0 newly installed, 0 to remove and 1 not upgraded.\r\n"
                ),
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
        } else if command == pacman::UPDATE {
            if self.fail_update {
                ("==> ERROR: Cannot fetch updates\n".into(), 1)
            } else {
                (String::new(), 0)
            }
        } else if command == pacman::LIST {
            // linux is pinned with IgnorePkg, so the upgrade leaves it.
            let held = "linux 6.17.1.arch1-1 -> 6.17.2.arch1-1 [ignored]\n";
            let fresh = if upgraded {
                ""
            } else {
                "openssl 3.6.0-1 -> 3.6.0-2\n"
            };
            (format!("{fresh}{held}"), 0)
        } else if [pacman::UPGRADE, pacman::SUDO_UPGRADE].contains(&command) {
            self.upgraded.store(true, Ordering::SeqCst);
            (
                ":: Synchronizing package databases...\r\n:: Starting full system upgrade...\r\nwarning: linux: ignoring package upgrade (6.17.1.arch1-1 => 6.17.2.arch1-1)\r\n(1/1) upgrading openssl\r\n".into(),
                0,
            )
        } else if [apk::UPDATE, apk::SUDO_UPDATE].contains(&command) {
            if self.fail_update {
                // apk only warns about an index it cannot fetch, and exits 0.
                (
                    "WARNING: fetching https://dl-cdn.alpinelinux.org/alpine/v3.24/main: temporary error (try again later)\nOK: 0 distinct packages available\n".into(),
                    0,
                )
            } else {
                ("OK: 28671 distinct packages available\r\n".into(), 0)
            }
        } else if command == apk::LIST {
            let held = "musl-1.2.6-r1 x86_64 {musl} (MIT) [upgradable from: musl-1.2.6-r0]\n";
            let fresh = if upgraded {
                ""
            } else {
                "openssl-3.6.0-r2 x86_64 {openssl} (Apache-2.0) [upgradable from: openssl-3.6.0-r1]\n"
            };
            (format!("{fresh}{held}"), 0)
        } else if [apk::UPGRADE, apk::SUDO_UPGRADE].contains(&command) {
            self.upgraded.store(true, Ordering::SeqCst);
            (
                "(1/1) Upgrading openssl (3.6.0-r1 -> 3.6.0-r2)\r\nOK: 12 MiB in 30 packages\r\n"
                    .into(),
                0,
            )
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
    /// Channels running an interactive shell, which echoes what it gets.
    shells: Vec<ChannelId>,
}
impl server::Server for Mock {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        Self {
            state: self.state.clone(),
            waiting: HashMap::new(),
            shells: Vec::new(),
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
        term: &str,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if term == "dumb" {
            // Commands run without echo, so a sudo password is never printed back.
            assert!(modes.contains(&(Pty::ECHO, 0)));
            self.state.pty_count.fetch_add(1, Ordering::SeqCst);
        } else {
            (self.state.shell_ptys.lock().unwrap()).push((term.into(), cols, rows));
        }
        session.channel_success(channel)
    }
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shells.push(channel);
        session.channel_success(channel)?;
        session.data(channel, b"\x1b[1mwelcome\x1b[0m\r\n$ ".to_vec())
    }
    async fn window_change_request(
        &mut self,
        _: ChannelId,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.state.resizes.lock().unwrap().push((cols, rows));
        Ok(())
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
        } else if self.state.hangs(&command) {
            Ok(())
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
        if self.shells.contains(&channel) {
            // `exit` ends the shell, as a login shell would.
            if data == b"exit\r" {
                session.eof(channel)?;
                return session.close(channel);
            }
            // Far more output than the client lets through unread.
            if data == b"flood\r" {
                for _ in 0..48 {
                    session.data(channel, vec![b'y'; 64 * 1024])?;
                }
                return Ok(());
            }
            return session.data(channel, data.to_vec());
        }
        if let Some(command) = self.waiting.remove(&channel) {
            assert_eq!(data, b"sudo-secret\n");
            if self.state.wrong_sudo {
                session.data(channel, b"Sorry, try again.\n__MISF_SUDO_PROMPT__".to_vec())
            } else if self.state.hangs(&command) {
                Ok(())
            } else {
                let (text, code) = self.state.reply(&command);
                finish(session, channel, &text, code)
            }
        } else {
            self.state
                .stray_input
                .lock()
                .unwrap()
                .extend_from_slice(data);
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
        shells: Vec::new(),
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
            misf_core::apt_lock::SNAPSHOT,
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
    reboot::reboot(&root, &|_| {}, &Echo::silent())
        .await
        .unwrap();
    assert_eq!(
        fixture.state.commands.lock().unwrap().last().unwrap(),
        reboot::ROOT
    );

    let rocky = fixture_on(DNF_HOST, false, false).await;
    let error = reboot::reboot(&rocky.host, &|_| {}, &Echo::silent())
        .await
        .unwrap_err();
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
    // CentOS 7 has only yum, which MISF no longer drives.
    let mut fixture = fixture_on(
        "Linux\n3.10.0\n__MISF_NAME__CentOS Linux\n__MISF_VERSION__7\n__MISF_PM__yum\n",
        false,
        false,
    )
    .await;
    fixture.host.package_manager.clear();
    let result = package_manager::check(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "unsupported");
    assert_eq!(result.distro, "CentOS Linux 7");
    assert!(result.error.contains("pacman 或 apk"), "{}", result.error);
    assert_eq!(
        *fixture.state.commands.lock().unwrap(),
        [package_manager::DETECT]
    );

    assert_eq!(
        *fixture.state.commands.lock().unwrap(),
        [package_manager::DETECT]
    );
}

#[tokio::test]
async fn chosen_package_manager_overrides_detection_only_when_installed() {
    // This host has apt and apk; detection prefers apt, but the user chose apk.
    let mut fixture = fixture_on(APK_HOST, false, false).await;
    fixture.host.package_manager = "apt".into();
    fixture.host.manager_choice = "apk".into();
    let result = package_manager::check(&fixture.host, &|_| {}).await;
    let commands = fixture.state.commands.lock().unwrap().clone();
    let expected = package_manager::package_commands(&fixture.host, Mode::Check);
    assert!(expected[1].contains("apk update"), "{expected:?}");
    assert_eq!(commands, expected, "{}", result.error);
    assert_eq!(result.status, "success", "{}", result.error);
    // The detected manager is still recorded as detected.
    assert_eq!(result.package_manager, "apt");
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

/// What each host's commands showed live, as `(host ID, text)`; commands start with `$ `.
#[derive(Default)]
struct Screen(Mutex<Vec<(String, String)>>);
impl Console for Screen {
    fn command(&self, host: &str, command: &str) {
        let line = format!("$ {command}\n");
        self.0.lock().unwrap().push((host.into(), line));
    }
    fn output(&self, host: &str, text: &str) {
        self.0.lock().unwrap().push((host.into(), text.into()));
    }
}
impl Screen {
    fn of(&self, host: &str) -> String {
        (self.0.lock().unwrap().iter())
            .filter(|(id, _)| id == host)
            .map(|(_, text)| text.as_str())
            .collect()
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
    let screen = Arc::new(Screen::default());
    let console: Arc<dyn Console> = screen.clone();
    let response = dispatch_with(
        json!({"action":"batch","operation":"check","ids":ids}),
        &store,
        &|_| {},
        &recorder,
        &console,
        &misf_core::cancel::Cancel::default(),
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
    // Each checked host showed its own commands and output live, cleaned and redacted.
    for (id, status) in finished.iter().filter(|(_, status)| status == "success") {
        let shown = screen.of(id);
        assert!(
            shown.contains(&format!("$ {}\n", apt::LIST)),
            "{status}: {shown}"
        );
        assert!(
            shown.contains("Listing...\nopenssl/stable-security"),
            "{shown}"
        );
        assert!(!shown.contains("sudo-secret"));
        // The prompt marker appears only in the commands that set it, never in output.
        let output: String = (shown.lines())
            .filter(|line| !line.starts_with("$ "))
            .collect();
        assert!(!output.contains("__MISF_SUDO_PROMPT__"), "{shown}");
        assert!(!shown.contains(['\r', '\x1b']));
    }
    for id in skipped {
        assert!(screen.of(id.as_str().unwrap()).is_empty());
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
    updated.package_manager = "dnf".into();
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

/// Collects a shell's output and its end.
#[derive(Default)]
struct ShellScreen {
    output: Mutex<Vec<u8>>,
    ended: Mutex<Option<Option<String>>>,
}

impl shell::Screen for ShellScreen {
    fn output(&self, _: u64, data: &[u8]) {
        self.output.lock().unwrap().extend_from_slice(data);
    }
    fn ended(&self, _: u64, error: Option<String>) {
        *self.ended.lock().unwrap() = Some(error);
    }
}

/// Waits up to five seconds for `done`.
async fn until(done: impl Fn() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interactive_shell_echoes_keys_follows_resizes_and_ends() {
    let fixture = fixture(false, false).await;
    let shells = shell::Shells::default();
    let screen = Arc::new(ShellScreen::default());
    let id = shells.reserve();
    shell::open(&fixture.host, id, 100, 30, &shells, screen.clone())
        .await
        .unwrap();
    assert_eq!(
        *fixture.state.shell_ptys.lock().unwrap(),
        [("xterm-256color".to_owned(), 100, 30)]
    );
    // Output arrives raw, escape sequences and all, for the emulator to draw.
    let output = || String::from_utf8_lossy(&screen.output.lock().unwrap()).into_owned();
    until(|| output().contains("$ ")).await;
    assert!(output().starts_with("\x1b[1mwelcome"));
    assert_eq!(shells.count(), 1);

    assert!(shells.send(id, shell::Input::Data(b"nano\x18".to_vec())));
    until(|| output().ends_with("nano\x18")).await;
    assert!(shells.send(id, shell::Input::Resize { cols: 80, rows: 20 }));
    until(|| !fixture.state.resizes.lock().unwrap().is_empty()).await;
    assert_eq!(*fixture.state.resizes.lock().unwrap(), [(80, 20)]);

    // The remote side ends it: the screen hears, and it is no longer open.
    assert!(shells.send(id, shell::Input::Data(b"exit\r".to_vec())));
    until(|| screen.ended.lock().unwrap().is_some()).await;
    assert_eq!(*screen.ended.lock().unwrap(), Some(None));
    assert_eq!(shells.count(), 0);
    assert!(!shells.send(id, shell::Input::Data(b"x".to_vec())));

    // Closing from this side, e.g. when the vault locks, ends it too.
    let screen = Arc::new(ShellScreen::default());
    let id = shells.reserve();
    shell::open(&fixture.host, id, 100, 30, &shells, screen.clone())
        .await
        .unwrap();
    shells.close_all();
    until(|| screen.ended.lock().unwrap().is_some()).await;
    assert_eq!(shells.count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_stops_reading_while_the_screen_is_behind() {
    let fixture = fixture(false, false).await;
    let shells = shell::Shells::default();
    let screen = Arc::new(ShellScreen::default());
    let id = shells.reserve();
    shell::open(&fixture.host, id, 100, 30, &shells, screen.clone())
        .await
        .unwrap();
    let received = || screen.output.lock().unwrap().len();
    until(|| received() > 0).await;
    let welcome = received();
    screen.output.lock().unwrap().clear();
    // 3 MiB is sent; the screen draws nothing, so reading stops near the window.
    assert!(shells.send(id, shell::Input::Data(b"flood\r".to_vec())));
    until(|| received() >= shell::WINDOW).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let stalled = received();
    assert!(stalled < 3 * 1024 * 1024, "{stalled}");
    // Once it is drawn, the rest comes through.
    let mut acknowledged = 0;
    for _ in 0..500 {
        let now = welcome + received();
        shells.drawn(id, now - acknowledged);
        acknowledged = now;
        if received() >= 3 * 1024 * 1024 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(received(), 3 * 1024 * 1024);
    shells.close_all();
}

#[tokio::test]
async fn root_output_printing_the_sudo_marker_gets_no_password_and_no_abort() {
    let mut fixture = fixture(false, false).await;
    fixture.host.username = "root".into();
    fixture.state.print_marker.store(true, Ordering::SeqCst);
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert!(fixture.state.stray_input.lock().unwrap().is_empty());
}

/// An unlocked session over a vault holding `hosts`, whose audit log is in `dir/logs`.
async fn session_with(
    dir: &std::path::Path,
    hosts: Vec<Host>,
    cancel: &misf_core::cancel::Cancel,
) -> misf_core::session::Session {
    use luksbox_core::{Argon2idParams, CipherSuite};
    use luksbox_format::Container;
    use luksbox_vfs::Vfs;
    use misf_core::store::Database;
    let password = "integration-test-passphrase";
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
                &dir.join("hosts.lbx"),
                None,
                CipherSuite::Aes256GcmSiv,
                params,
                password.as_bytes(),
            )
            .unwrap(),
        )
        .unwrap();
        let file = vfs.create(vfs.root_id(), "hosts.json").unwrap();
        let bytes = serde_json::to_vec(&Database {
            version: 1,
            hosts,
            ..Default::default()
        })
        .unwrap();
        vfs.write(file, 0, &bytes).unwrap();
        vfs.finish().unwrap();
    }
    let paths = misf_core::runtime::RuntimePaths {
        store: dir.join("hosts.db"),
        debug: false,
        debug_allow_passkey: false,
    };
    let mut session =
        misf_core::session::Session::new(&paths, misf_core::audit::AuditLog::new(dir).unwrap());
    session.set_cancel(cancel.clone());
    session
        .handle(
            json!({"action":"vault_unlock","passphrase":password}),
            &|_| {},
        )
        .await
        .unwrap();
    session
}

fn audit_log(dir: &std::path::Path) -> String {
    std::fs::read_dir(dir.join("logs"))
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect()
}

/// Requests a stop once `ready` holds.
fn stop_when(
    cancel: &misf_core::cancel::Cancel,
    ready: impl Fn() -> bool + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let cancel = cancel.clone();
    tokio::spawn(async move {
        until(ready).await;
        cancel.request();
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_upgrade_is_saved_as_unknown_and_audited_as_cancelled() {
    let fixture = fixture(false, false).await;
    fixture.state.hang_upgrades.store(true, Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    let cancel = misf_core::cancel::Cancel::default();
    let mut session = session_with(dir.path(), vec![fixture.host.clone()], &cancel).await;
    let state = fixture.state.clone();
    let stopper = stop_when(&cancel, move || {
        (state.commands.lock().unwrap().iter()).any(|c| c == apt::SUDO_UPGRADE)
    });
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        session.handle(json!({"action":"upgrade","id":"fixture"}), &|_| {}),
    )
    .await
    .expect("the stop ends the upgrade")
    .unwrap();
    stopper.await.unwrap();
    assert_eq!(result["status"], "failed");
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("dpkg --configure -a")
    );
    // Saved, so the host shows the unknown result after a restart too.
    let listed = session
        .handle(json!({"action":"list"}), &|_| {})
        .await
        .unwrap();
    assert_eq!(listed["hosts"][0]["last_check"]["error"], result["error"]);
    let log = audit_log(dir.path());
    assert!(
        log.lines()
            .any(|line| line.contains("\"action\":\"upgrade\"") && line.contains("cancelled")),
        "{log}"
    );

    // A stopped check changes nothing and keeps the last result.
    cancel.reset();
    let state = fixture.state.clone();
    let before = state.commands.lock().unwrap().len();
    fixture.state.hang_updates.store(true, Ordering::SeqCst);
    let stopper = stop_when(&cancel, move || {
        state.commands.lock().unwrap()[before..]
            .iter()
            .any(|c| c == apt::SUDO_UPDATE)
    });
    let error = session
        .handle(json!({"action":"check","id":"fixture"}), &|_| {})
        .await
        .unwrap_err();
    stopper.await.unwrap();
    assert!(format!("{error:#}").contains("已中斷檢查"), "{error:#}");
    let listed = session
        .handle(json!({"action":"list"}), &|_| {})
        .await
        .unwrap();
    assert_eq!(listed["hosts"][0]["last_check"]["error"], result["error"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_batch_keeps_finished_hosts_and_marks_running_upgrades_unknown() {
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.db"));
    let mut ids = vec![];
    for index in 0..3 {
        ids.push(register(&store, &fixture, &format!("host-{index}")).await);
    }
    // One upgrade finishes; the other two stay stuck until stopped.
    fixture.state.hang_upgrades.store(true, Ordering::SeqCst);
    fixture.state.free_upgrades.store(1, Ordering::SeqCst);
    let recorder = Arc::new(Recorder {
        decision: Decision::Continue,
        finished: Mutex::new(vec![]),
        failed: Mutex::new(vec![]),
    });
    let cancel = misf_core::cancel::Cancel::default();
    let (state, seen) = (fixture.state.clone(), recorder.clone());
    let stopper = stop_when(&cancel, move || {
        let started = (state.commands.lock().unwrap().iter())
            .filter(|c| *c == apt::SUDO_UPGRADE)
            .count();
        started == 3 && seen.finished.lock().unwrap().len() == 1
    });
    let console: Arc<dyn Console> = Arc::new(Screen::default());
    let response = tokio::time::timeout(
        Duration::from_secs(20),
        dispatch_with(
            json!({"action":"batch","operation":"upgrade","ids":ids}),
            &store,
            &|_| {},
            recorder.as_ref(),
            &console,
            &cancel,
        ),
    )
    .await
    .expect("the stop ends the batch")
    .unwrap();
    stopper.await.unwrap();
    let cancelled: Vec<String> = serde_json::from_value(response["cancelled"].clone()).unwrap();
    assert_eq!(cancelled.len(), 2);
    assert!(response["skipped"].as_array().unwrap().is_empty());
    assert_eq!(response["results"].as_array().unwrap().len(), 3);
    // Every host was reported once, and the reports match what was saved.
    let finished = recorder.finished.lock().unwrap();
    assert_eq!(finished.len(), 3);
    for id in &ids {
        let saved = store.host(id).unwrap().last_check.unwrap();
        if cancelled.contains(id) {
            assert_eq!(saved.status, "failed");
            assert!(saved.error.contains("dpkg --configure -a"));
        } else {
            assert_eq!(saved.status, "success", "{}", saved.error);
        }
    }
}

#[tokio::test]
async fn pacman_checks_without_sudo_or_a_system_sync_and_upgrades_through_sudo() {
    let fixture = fixture_on(PACMAN_HOST, false, false).await;
    let check = package_manager::check(&fixture.host, &|_| {}).await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert_eq!(check.distro, "Arch Linux");
    assert_eq!(check.package_manager, "pacman");
    assert_eq!(check.packages.len(), 2);
    assert_eq!(check.packages[0].name, "openssl");
    assert_eq!(check.packages[0].current_version, "3.6.0-1");
    assert_eq!(check.packages[0].candidate_version, "3.6.0-2");
    // checkupdates syncs a private copy: no sudo and no `pacman -Sy`. The refresh still
    // gets the host's PTY setting, as every manager's refresh does; the listing does not.
    assert_eq!(fixture.state.pty_count.load(Ordering::SeqCst), 1);
    {
        let commands = fixture.state.commands.lock().unwrap();
        assert_eq!(
            *commands,
            [package_manager::DETECT, pacman::UPDATE, pacman::LIST]
        );
        assert!(
            commands
                .iter()
                .all(|c| !c.contains("sudo") && !c.contains("-Sy"))
        );
    }

    let upgrade = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(upgrade.status, "success", "{}", upgrade.error);
    assert_eq!(upgrade.upgraded.len(), 1);
    assert_eq!(upgrade.upgraded[0].name, "openssl");
    assert_eq!(upgrade.packages.len(), 1);
    assert_eq!(upgrade.packages[0].name, "linux");
    // The refresh again, then the upgrade through sudo with the password.
    assert_eq!(fixture.state.pty_count.load(Ordering::SeqCst), 3);
    let commands = fixture.state.commands.lock().unwrap();
    assert_eq!(
        commands[3..],
        package_manager::package_commands(&fixture.host, Mode::Upgrade)
    );
    assert!(commands.contains(&pacman::SUDO_UPGRADE.to_owned()));
}

#[tokio::test]
async fn a_failed_pacman_sync_says_what_the_host_needs_and_lists_nothing() {
    let fixture = fixture_on(PACMAN_HOST, true, false).await;
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "failed");
    assert!(result.error.contains("pacman-contrib"), "{}", result.error);
    assert!(result.packages.is_empty());
    assert_eq!(
        *fixture.state.commands.lock().unwrap(),
        [package_manager::DETECT, pacman::UPDATE]
    );
}

#[tokio::test]
async fn apk_upgrades_as_root_and_an_unfetched_index_fails_despite_exit_zero() {
    let mut fixture = fixture_on(APK_HOST, false, false).await;
    fixture.host.username = "root".into();
    fixture.host.manager_choice = "apk".into();
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert_eq!(result.upgraded.len(), 1);
    assert_eq!(result.upgraded[0].name, "openssl");
    assert_eq!(result.upgraded[0].current_version, "3.6.0-r1");
    assert_eq!(result.upgraded[0].architecture, "x86_64");
    assert_eq!(result.packages.len(), 1);
    assert_eq!(result.packages[0].name, "musl");
    let commands = fixture.state.commands.lock().unwrap().clone();
    assert_eq!(
        commands,
        [
            package_manager::DETECT,
            apk::UPDATE,
            apk::LIST,
            apk::UPGRADE,
            apk::LIST
        ]
    );

    let mut fixture = fixture_on(APK_HOST, true, false).await;
    fixture.host.manager_choice = "apk".into();
    let result = package_manager::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "failed");
    assert!(
        result.error.contains("更新套件索引失敗"),
        "{}",
        result.error
    );
    assert_eq!(
        *fixture.state.commands.lock().unwrap(),
        [package_manager::DETECT, apk::SUDO_UPDATE]
    );
}

#[tokio::test]
async fn hosts_saved_for_bsd_or_yum_come_back_linux_and_detected_automatically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.db");
    // A host list from before SQLite, the way an older MISF wrote it.
    std::fs::write(
        dir.path().join("hosts.json"),
        serde_json::to_vec(&json!({
            "version": 1,
            "hosts": [
                {"id":"bsd","name":"bsd","address":"10.0.0.1","port":22,"username":"root",
                 "os":"BSD","auth_type":"password","password":"pw","package_manager":"pkg",
                 "manager_choice":"pkg"},
                {"id":"centos","name":"centos","address":"10.0.0.2","port":22,"username":"root",
                 "os":"Linux","auth_type":"password","password":"pw","package_manager":"yum",
                 "manager_choice":"yum"},
                {"id":"debian","name":"debian","address":"10.0.0.3","port":22,"username":"root",
                 "os":"Linux","auth_type":"password","password":"pw","package_manager":"apt"},
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    let store = Store::new(path);
    let db = store.read().unwrap();
    let found: Vec<_> = db
        .hosts
        .iter()
        .map(|h| {
            (
                h.id.as_str(),
                h.os.as_str(),
                h.package_manager.as_str(),
                h.manager_choice.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("bsd", "Linux", "", ""),
            ("centos", "Linux", "", ""),
            ("debian", "Linux", "apt", ""),
        ]
    );
    // Each one can be saved again as it is now.
    for host in db.hosts {
        dispatch(json!({"action":"save","host":host}), &store, &|_| {})
            .await
            .unwrap();
    }
}
