use misf_core::{
    dispatch,
    model::{Host, parse_packages},
    ssh,
    store::Store,
};
use russh::{
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Server as _, Session, VerifiedPublicKey},
    *,
};
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct State {
    auth_attempts: AtomicUsize,
    commands: Mutex<Vec<String>>,
    pty_count: AtomicUsize,
    fail_update: bool,
    wrong_sudo: bool,
    /// Set once an upgrade ran: later listings report only the held-back package.
    upgraded: AtomicBool,
}

#[derive(Clone)]
struct Mock {
    state: Arc<State>,
    waiting: HashSet<ChannelId>,
    upgrading: HashSet<ChannelId>,
}
impl server::Server for Mock {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        Self {
            state: self.state.clone(),
            waiting: HashSet::new(),
            upgrading: HashSet::new(),
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
        if command == "uname -s" {
            finish(session, channel, "Linux\n", 0)
        } else if command == ssh::DETECT {
            finish(session, channel, "Linux\n__MISF_APT__\n", 0)
        } else if command == ssh::LIST {
            let held = "libc6/stable 2.36-9 amd64 [upgradable from: 2.36-8]\n";
            let listing = if self.state.upgraded.load(Ordering::SeqCst) {
                format!("Listing...\n{held}")
            } else {
                format!(
                    "Listing...\nopenssl/stable-security 3.0.17 amd64 [upgradable from: 3.0.16]\n{held}"
                )
            };
            finish(session, channel, &listing, 0)
        } else if command == ssh::UPGRADE {
            self.state.upgraded.store(true, Ordering::SeqCst);
            finish(session, channel, "Setting up openssl (3.0.17) ...\n", 0)
        } else if command.starts_with("LC_ALL=C sudo -S") {
            self.waiting.insert(channel);
            if command == ssh::SUDO_UPGRADE {
                self.upgrading.insert(channel);
            }
            // Deliberately split the sudo marker across SSH packets.
            session.data(channel, b"__MISF_SUDO_".to_vec())?;
            session.data(channel, b"PROMPT__".to_vec())
        } else if command.starts_with("LC_ALL=C apt-get ") {
            finish(session, channel, "Reading package lists... Done\n", 0)
        } else {
            finish(session, channel, "unexpected command\n", 127)
        }
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.waiting.remove(&channel) {
            assert_eq!(data, b"sudo-secret\n");
            if self.state.wrong_sudo {
                session.data(channel, b"Sorry, try again.\n__MISF_SUDO_PROMPT__".to_vec())
            } else if self.state.fail_update {
                finish(session, channel, "E: repository unavailable\n", 100)
            } else if self.upgrading.remove(&channel) {
                self.state.upgraded.store(true, Ordering::SeqCst);
                finish(session, channel, "Setting up openssl (3.0.17) ...\r\n", 0)
            } else {
                finish(
                    session,
                    channel,
                    "0% [Working]\r\x1b[2K0% [Connecting]\r\x1b[32mHit:1 test-repo\x1b[0m\r\nReading package lists... Done\r\n",
                    0,
                )
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
        fail_update,
        wrong_sudo,
        ..Default::default()
    });
    let mut server = Mock {
        state: state.clone(),
        waiting: HashSet::new(),
        upgrading: HashSet::new(),
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
        ..Default::default()
    };
    Fixture { host, state, task }
}

#[tokio::test]
async fn password_pty_sudo_and_packages_work_end_to_end() {
    let fixture = fixture(false, false).await;
    assert_eq!(ssh::test_connection(&fixture.host).await.unwrap(), "Linux");
    let check = ssh::check(&fixture.host, &|_| {}).await;
    assert_eq!(check.status, "success", "{}", check.error);
    assert_eq!(check.packages.len(), 2);
    assert_eq!(check.packages[0].current_version, "3.0.16");
    assert_eq!(check.packages[0].candidate_version, "3.0.17");
    assert_eq!(fixture.state.pty_count.load(Ordering::SeqCst), 1);
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
    assert!(
        !commands
            .iter()
            .any(|c| c.contains(" upgrade") || c.contains(" install"))
    );
}

#[tokio::test]
async fn sudo_upgrade_reports_installed_and_held_back_packages() {
    let fixture = fixture(false, false).await;
    let result = ssh::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert_eq!(result.operation, "upgrade");
    assert_eq!(result.upgraded.len(), 1);
    assert_eq!(result.upgraded[0].name, "openssl");
    assert_eq!(result.packages.len(), 1);
    assert_eq!(result.packages[0].name, "libc6");
    assert!(result.output.contains("Setting up openssl"));
    assert!(!result.output.contains("sudo-secret"));
    assert_eq!(fixture.state.pty_count.load(Ordering::SeqCst), 2);
    let commands = fixture.state.commands.lock().unwrap();
    assert_eq!(*commands, ssh::apt_commands(&fixture.host, true));
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
    let result = ssh::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "success", "{}", result.error);
    assert_eq!(result.upgraded.len(), 1);
    let commands = fixture.state.commands.lock().unwrap();
    assert!(commands.iter().any(|c| c == ssh::UPGRADE));
    assert!(!commands.iter().any(|c| c.contains("sudo")));
}

#[tokio::test]
async fn failed_index_update_never_starts_an_upgrade() {
    let fixture = fixture(true, false).await;
    let result = ssh::upgrade(&fixture.host, &|_| {}).await;
    assert_eq!(result.status, "failed");
    assert!(result.upgraded.is_empty());
    let commands = fixture.state.commands.lock().unwrap();
    assert!(!commands.iter().any(|c| c.contains(" upgrade")));
}

#[tokio::test]
async fn batch_runs_each_host_and_saves_per_host_results() {
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.json"));
    let mut ids = vec![];
    for name in ["first", "second", "untrusted"] {
        let mut host = fixture.host.clone();
        host.id.clear();
        host.name = name.into();
        let saved = dispatch(json!({"action":"save","host":host}), &store, &|_| {})
            .await
            .unwrap();
        let id = saved["id"].as_str().unwrap().to_owned();
        if name != "untrusted" {
            dispatch(
                json!({"action":"trust","id":id,"fingerprint":fixture.host.fingerprint}),
                &store,
                &|_| {},
            )
            .await
            .unwrap();
        }
        ids.push(id);
    }
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
            .any(|c| c.contains(" upgrade"))
    );
    for request in [
        json!({"action":"batch","operation":"install","ids":[ids[0]]}),
        json!({"action":"batch","operation":"check","ids":[]}),
        json!({"action":"batch","operation":"check","ids":["missing"]}),
    ] {
        assert!(dispatch(request, &store, &|_| {}).await.is_err());
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
    assert_eq!(ssh::test_connection(&fixture.host).await.unwrap(), "Linux");
}

#[tokio::test]
async fn root_check_executes_apt_without_sudo() {
    let mut fixture = fixture(false, false).await;
    fixture.host.username = "root".into();
    let result = ssh::check(&fixture.host, &|_| {}).await;
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
    let error = ssh::test_connection(&changed)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("指紋已變更"), "{error}");
    assert_eq!(fixture.state.auth_attempts.load(Ordering::SeqCst), 0);
    changed.fingerprint.clear();
    assert!(
        ssh::test_connection(&changed)
            .await
            .unwrap_err()
            .to_string()
            .contains("先確認")
    );
}

#[tokio::test]
async fn apt_update_failure_never_lists_stale_packages() {
    let fixture = fixture(true, false).await;
    let check = ssh::check(&fixture.host, &|_| {}).await;
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
            .any(|c| c == ssh::LIST)
    );
}

#[tokio::test]
async fn repeated_sudo_prompt_fails_without_repeated_password_submission() {
    let fixture = fixture(false, true).await;
    let check = ssh::check(&fixture.host, &|_| {}).await;
    assert_eq!(check.status, "failed");
    assert!(check.error.contains("sudo 密碼錯誤"), "{}", check.error);
}

#[tokio::test]
async fn failed_authentication_and_bsd_are_reported_without_package_results() {
    let fixture = fixture(false, false).await;
    let mut wrong = fixture.host.clone();
    wrong.password = "wrong".into();
    assert!(ssh::test_connection(&wrong).await.is_err());
    let before = fixture.state.auth_attempts.load(Ordering::SeqCst);
    wrong.os = "BSD".into();
    let check = ssh::check(&wrong, &|_| {}).await;
    assert_eq!(check.status, "unsupported");
    assert_eq!(fixture.state.auth_attempts.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn persistence_saves_check_and_resets_trust_when_endpoint_changes() {
    let fixture = fixture(false, false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hosts.json"));
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
    updated.port = 1;
    dispatch(json!({"action":"save","host":updated}), &store, &|_| {})
        .await
        .unwrap();
    assert!(store.host(id).unwrap().fingerprint.is_empty());
    assert!(store.host(id).unwrap().last_check.is_none());
    dispatch(json!({"action":"delete","id":id}), &store, &|_| {})
        .await
        .unwrap();
    assert!(store.read().unwrap().hosts.is_empty());
}

#[test]
fn malformed_store_is_preserved_and_unparseable_apt_output_is_not_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.json");
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
    assert!(parse_packages("Listing...\n").unwrap().is_empty());
    assert!(parse_packages("unexpected localized output").is_err());
    assert!(parse_packages("package/repo missing-fields").is_err());
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
        })
        .unwrap();
        vfs.write(file, 0, &bytes).unwrap();
        vfs.finish().unwrap();
    }
    let mut session = StorageSession::new(dir.path().join("hosts.json"));
    session
        .handle(
            json!({"action":"vault_unlock","passphrase":std::str::from_utf8(password).unwrap()}),
            &|_| {},
        )
        .await
        .unwrap();
    let result = session
        .handle(json!({"action":"check","id":fixture.host.id}), &|_| {})
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["packages"].as_array().unwrap().len(), 2);
    session
        .handle(json!({"action":"vault_lock"}), &|_| {})
        .await
        .unwrap();
    assert!(!dir.path().join("hosts.json").exists());
    let mut vfs =
        Vfs::open(Container::open(&path, None, UnlockMaterial::Passphrase(password)).unwrap())
            .unwrap();
    let file = vfs.lookup_path("/hosts.json").unwrap();
    let mut bytes = vec![0; vfs.stat(file).unwrap().size as usize];
    vfs.read(file, 0, &mut bytes).unwrap();
    let database: Database = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        database.hosts[0]
            .last_check
            .as_ref()
            .unwrap()
            .packages
            .len(),
        2
    );
}
