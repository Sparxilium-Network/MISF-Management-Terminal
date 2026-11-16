//! Shared harness for the package-manager fault tests.
//!
//! Each package manager has its own directory beside `src/`: `apt/` today. It holds a
//! `docker/` build context for a throwaway host and the tests that drive it; this crate
//! holds only what every manager's tests share. Each test gets its own container from
//! that image, and MISF's core checks and upgrades it over SSH exactly as the app does,
//! through a TCP relay the test can cut or freeze to stand in for a lost connection.
//!
//! An image must run sshd on port 22 with an Ed25519 or ECDSA host key, create the three
//! [`Account`]s, and provide a `misf-scenario` command that publishes the named packages'
//! upgrades in a local repository.
//!
//! Needs a `docker` command that can run Linux containers. Images are built once per test
//! run; `MISF_SSHTEST_<MANAGER>_BASE` picks the base image (see [`System`]).

use misf_core::{
    batch::Unattended,
    cancel::Cancel,
    console::Console,
    dispatch, dispatch_with,
    model::{CheckResult, Host, Package},
    ssh,
    store::Store,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpListener, TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::watch,
    time::{Instant, sleep, timeout},
};

/// Every container carries this label, so leftovers from a killed run can be found:
/// `docker ps -aq --filter label=misf-sshtest=1`.
const LABEL: &str = "misf-sshtest=1";

/// How long [`Lab::run`] lets one operation take, in seconds.
pub const PATIENCE: u64 = 5 * 60;

/// One package manager's test host: where its image is built from and how MISF names it.
pub struct System {
    /// MISF's ID for the package manager, which is also the directory holding `docker/`.
    pub manager: &'static str,
    /// The base image when `MISF_SSHTEST_<MANAGER>_BASE` is not set.
    pub default_base: &'static str,
}

/// Debian and Ubuntu hosts, built from `apt/docker`; `MISF_SSHTEST_APT_BASE` picks the base.
pub const APT: System = System {
    manager: "apt",
    default_base: "debian:13",
};

/// Fedora and Enterprise Linux hosts, built from `dnf/docker`; `MISF_SSHTEST_DNF_BASE`
/// picks the base.
pub const DNF: System = System {
    manager: "dnf",
    default_base: "rockylinux/rockylinux:9",
};

/// The accounts every image creates; the passwords are written in its Dockerfile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Account {
    /// Runs the package manager directly.
    Root,
    /// Runs it through sudo, which asks for the password.
    Admin,
    /// Runs it through sudo configured with NOPASSWD.
    Deploy,
}

impl Account {
    pub fn username(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Admin => "misfadmin",
            Self::Deploy => "misfdeploy",
        }
    }

    pub fn password(self) -> &'static str {
        match self {
            Self::Root => "misf-root-pw",
            Self::Admin => "misf-admin-pw",
            Self::Deploy => "misf-deploy-pw",
        }
    }
}

/// Runs `docker` and returns whether it succeeded and what it printed, both streams.
fn docker_now(args: &[String]) -> (bool, String) {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("the docker command must be installed and on PATH");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

/// [`docker_now`] off the async threads, so a waiting test keeps its SSH session alive.
async fn docker(args: &[&str]) -> (bool, String) {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    tokio::task::spawn_blocking(move || docker_now(&args))
        .await
        .unwrap()
}

/// Builds a system's image once per test run; unchanged layers come from Docker's cache.
fn image(system: &System) -> String {
    static IMAGES: Mutex<Option<HashMap<&'static str, String>>> = Mutex::new(None);
    // Held while building, so tests starting together wait for one build.
    let mut images = IMAGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let images = images.get_or_insert_with(HashMap::new);
    if let Some(image) = images.get(system.manager) {
        return image.clone();
    }
    let variable = format!("MISF_SSHTEST_{}_BASE", system.manager.to_uppercase());
    let base = std::env::var(variable).unwrap_or_else(|_| system.default_base.into());
    let tag: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let image = format!("misf-sshtest-{}:{tag}", system.manager);
    let context = format!("{}/{}/docker", env!("CARGO_MANIFEST_DIR"), system.manager);
    let (built, log) = docker_now(&[
        "build".into(),
        "--build-arg".into(),
        format!("BASE={base}"),
        "-t".into(),
        image.clone(),
        context,
    ]);
    assert!(
        built,
        "building the {} test image failed:\n{log}",
        system.manager
    );
    images.insert(system.manager, image.clone());
    image
}

/// One running test host; removed when dropped.
pub struct Container {
    id: String,
    /// The SSH port, published on 127.0.0.1 only.
    pub port: u16,
}

impl Container {
    /// Starts a fresh host whose repository offers the upgrades of `packages`.
    pub async fn start(system: &'static System, packages: &[&str]) -> Self {
        let image = tokio::task::spawn_blocking(move || image(system))
            .await
            .unwrap();
        let (started, id) = docker(&[
            "run",
            "-d",
            "--rm",
            "--init",
            "--label",
            LABEL,
            "-p",
            "127.0.0.1::22",
            &image,
        ])
        .await;
        assert!(started, "starting a container failed:\n{id}");
        let mut container = Self {
            id: id.trim().to_owned(),
            port: 0,
        };
        let (found, mapping) = docker(&["port", &container.id, "22/tcp"]).await;
        assert!(found, "{mapping}");
        container.port = mapping
            .lines()
            .find_map(|line| line.trim().rsplit(':').next()?.parse().ok())
            .unwrap_or_else(|| panic!("no published SSH port in {mapping:?}"));
        container
            .sh(&format!("misf-scenario {}", packages.join(" ")))
            .await;
        container
    }

    /// Runs a shell script in the host as root; returns whether it succeeded and its output.
    pub async fn try_sh(&self, script: &str) -> (bool, String) {
        docker(&["exec", &self.id, "sh", "-c", script]).await
    }

    /// [`Self::try_sh`] for a script that must succeed.
    pub async fn sh(&self, script: &str) -> String {
        let (succeeded, output) = self.try_sh(script).await;
        assert!(succeeded, "`{script}` failed in the container:\n{output}");
        output
    }

    /// Starts a script in the host and returns at once.
    pub async fn spawn(&self, script: &str) {
        let (started, output) = docker(&["exec", "-d", &self.id, "sh", "-c", script]).await;
        assert!(started, "{output}");
    }

    /// Waits until `script` succeeds in the host.
    pub async fn wait_for(&self, script: &str, seconds: u64) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while !self.try_sh(script).await.0 {
            assert!(
                Instant::now() < deadline,
                "`{script}` did not succeed within {seconds} seconds"
            );
            sleep(Duration::from_millis(200)).await;
        }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        docker_now(&["rm".into(), "-f".into(), self.id.clone()]);
    }
}

/// What the relay does with the connection between MISF and the host.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Link {
    Open,
    /// Sockets stay open but nothing passes: a route that silently died.
    Frozen,
    /// Sockets are closed and new connections are turned away.
    Cut,
}

/// A TCP relay in front of the host's SSH port, standing in for the network.
pub struct Relay {
    pub port: u16,
    link: watch::Sender<Link>,
    task: tokio::task::JoinHandle<()>,
}

impl Relay {
    async fn start(target: u16) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (link, state) = watch::channel(Link::Open);
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                if *state.borrow() == Link::Cut {
                    continue;
                }
                let Ok(server) = TcpStream::connect(("127.0.0.1", target)).await else {
                    continue;
                };
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                tokio::spawn(pump(client_read, server_write, state.clone()));
                tokio::spawn(pump(server_read, client_write, state.clone()));
            }
        });
        Self { port, link, task }
    }

    /// Closes every connection at once and refuses new ones: the link went down.
    pub fn cut(&self) {
        self.link.send_replace(Link::Cut);
    }

    /// Stops passing data without closing anything: packets vanish, no one is told.
    pub fn freeze(&self) {
        self.link.send_replace(Link::Frozen);
    }

    /// Lets connections through again after a cut or freeze.
    pub fn restore(&self) {
        self.link.send_replace(Link::Open);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.link.send_replace(Link::Cut);
        self.task.abort();
    }
}

/// Copies one direction until the link stops being open or either side closes.
async fn pump(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, mut link: watch::Receiver<Link>) {
    let mut buffer = vec![0; 16 * 1024];
    loop {
        let read = tokio::select! {
            read = from.read(&mut buffer) => read,
            _ = link.wait_for(|link| *link != Link::Open) => break,
        };
        match read {
            Ok(count) if count > 0 => {
                if to.write_all(&buffer[..count]).await.is_err() {
                    return;
                }
            }
            _ => return,
        }
    }
    // Frozen: hold both sockets open and silent until the link is cut or the test ends.
    let _ = link.wait_for(|link| *link == Link::Cut).await;
}

/// Everything shown live while commands run; commands start with `$ `.
#[derive(Default)]
pub struct Screen(Mutex<String>);

impl Console for Screen {
    fn command(&self, _: &str, command: &str) {
        let mut text = self.0.lock().unwrap();
        text.push_str("$ ");
        text.push_str(command);
        text.push('\n');
    }
    fn output(&self, _: &str, text: &str) {
        self.0.lock().unwrap().push_str(text);
    }
}

impl Screen {
    pub fn text(&self) -> String {
        self.0.lock().unwrap().clone()
    }
}

/// A test host registered in a MISF host store the way the app registers one: saved,
/// its fingerprint trusted, its connection tested.
pub struct Lab {
    pub container: Container,
    pub relay: Relay,
    pub store: Store,
    /// The live terminal output of every operation so far.
    pub screen: Arc<Screen>,
    /// The host's ID in the store.
    pub id: String,
    pub account: Account,
    _directory: tempfile::TempDir,
}

impl Lab {
    /// Starts a `system` host offering the upgrades of `packages` and registers it for
    /// `account`.
    pub async fn start(
        system: &'static System,
        account: Account,
        use_pty: bool,
        packages: &[&str],
    ) -> Self {
        let container = Container::start(system, packages).await;
        let relay = Relay::start(container.port).await;
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().join("hosts.db"));
        let host = Host {
            name: format!("sshtest {}", account.username()),
            address: "127.0.0.1".into(),
            port: relay.port,
            username: account.username().into(),
            os: "Linux".into(),
            auth_type: "password".into(),
            password: account.password().into(),
            use_pty,
            ..Default::default()
        };
        // sshd needs a moment after the container starts.
        let deadline = Instant::now() + Duration::from_secs(60);
        let fingerprint = loop {
            match ssh::probe(&host).await {
                Ok(fingerprint) => break fingerprint,
                Err(error) => {
                    assert!(Instant::now() < deadline, "sshd never answered: {error:#}");
                    sleep(Duration::from_millis(200)).await;
                }
            }
        };
        let saved = dispatch(json!({"action":"save","host":host}), &store, &|_| {})
            .await
            .unwrap();
        let id = saved["id"].as_str().unwrap().to_owned();
        dispatch(
            json!({"action":"trust","id":id,"fingerprint":fingerprint}),
            &store,
            &|_| {},
        )
        .await
        .unwrap();
        let test = dispatch(json!({"action":"test","id":id}), &store, &|_| {})
            .await
            .unwrap();
        assert_eq!(test["package_manager"], system.manager, "{test}");
        Self {
            container,
            relay,
            store,
            screen: Arc::default(),
            id,
            account,
            _directory: directory,
        }
    }

    /// Runs `check`, `upgrade` or `dist_upgrade` as the app does and returns the saved
    /// result. Fails the test after [`PATIENCE`], long before MISF's own 30-minute limit,
    /// so a host that unexpectedly waits for someone cannot stall a CI job.
    pub async fn run(&self, action: &str) -> CheckResult {
        self.run_for(action, PATIENCE)
            .await
            .unwrap_or_else(|| panic!("`{action}` was still running after {PATIENCE} seconds"))
    }

    /// [`Self::run`], given up on after `seconds`. Giving up drops the SSH connection,
    /// which is what the app's Stop button does; `None` means it had to.
    pub async fn run_for(&self, action: &str, seconds: u64) -> Option<CheckResult> {
        let console: Arc<dyn Console> = self.screen.clone();
        let cancel = Cancel::default();
        let progress = |_: &str| {};
        let operation = dispatch_with(
            json!({"action":action,"id":self.id}),
            &self.store,
            &progress,
            &Unattended,
            &console,
            &cancel,
        );
        let result: Value = timeout(Duration::from_secs(seconds), operation)
            .await
            .ok()?
            .unwrap();
        Some(serde_json::from_value(result).unwrap())
    }

    /// The host as stored, with its secrets, to check nothing leaked.
    pub fn host(&self) -> Host {
        self.store.host(&self.id).unwrap()
    }
}

/// The names of `packages`, in order.
pub fn names(packages: &[Package]) -> Vec<&str> {
    packages
        .iter()
        .map(|package| package.name.as_str())
        .collect()
}

/// Saved output and live lines are plain text: no control characters but the newline,
/// and none of the host's secrets.
pub fn assert_plain(lab: &Lab, text: &str) {
    let host = lab.host();
    assert!(
        !text.chars().any(|c| c.is_control() && c != '\n'),
        "control characters left in:\n{text:?}"
    );
    assert!(!text.contains(&host.password), "the password leaked");
    // The live view also shows each command (`$ …`), and sudo commands name the marker.
    assert!(
        !(text.lines())
            .any(|line| !line.starts_with("$ ") && line.contains("__MISF_SUDO_PROMPT__")),
        "the sudo prompt marker was left in the output"
    );
}

/// A failed run names the step and never claims packages.
pub fn assert_failed(result: &CheckResult, error: &str) {
    assert_eq!(result.status, "failed", "{}", result.output);
    assert!(result.error.contains(error), "{}", result.error);
    assert!(result.packages.is_empty() && result.upgraded.is_empty());
    assert!(result.restart.is_none());
}
