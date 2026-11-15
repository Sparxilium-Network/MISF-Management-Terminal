//! Interactive shells: a signed-in SSH session with a PTY and a login shell, kept open
//! after the request that opened it returns. Bytes typed in the app go straight to the
//! shell and its output comes straight back; neither passes through the audit log or
//! the request queue, so typing does not wait behind other work.

use crate::model::Host;
use crate::ssh::{self, Details};
use anyhow::{Context, Result, bail};
use russh::ChannelMsg;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{
    Notify,
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};
use tokio::time::timeout;

/// Output handed to the screen but not yet drawn, per shell. Past this the shell stops
/// reading its channel, so SSH flow control makes the remote side wait.
pub const WINDOW: usize = 1024 * 1024;

/// What the app sends an open shell.
pub enum Input {
    /// Typed keys, already turned into the bytes a terminal sends.
    Data(Vec<u8>),
    /// The terminal's new size in character cells.
    Resize { cols: u32, rows: u32 },
    /// Ends the shell.
    Close,
}

/// Where a shell's output goes.
pub trait Screen: Send + Sync {
    /// Raw output, escape sequences included, for a terminal emulator to draw. Each
    /// call must be answered with [`Shells::drawn`] once it is drawn or dropped.
    fn output(&self, shell: u64, data: &[u8]);
    /// The shell is gone: the remote side ended it, it was closed, or the link broke.
    fn ended(&self, shell: u64, error: Option<String>);
}

/// How far a shell's output is ahead of the screen.
#[derive(Clone, Default)]
struct Flow {
    pending: Arc<AtomicUsize>,
    drained: Arc<Notify>,
}

struct Entry {
    input: UnboundedSender<Input>,
    flow: Flow,
}

/// The open shells, shared by the worker that opens them and the app that types into
/// them.
#[derive(Clone, Default)]
pub struct Shells {
    open: Arc<Mutex<HashMap<u64, Entry>>>,
    next: Arc<AtomicU64>,
}

impl Shells {
    /// A number for the next shell, never reused.
    pub fn reserve(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Lists a shell as open and returns where its input arrives.
    pub fn register(&self, shell: u64) -> UnboundedReceiver<Input> {
        self.enter(shell).0
    }

    fn enter(&self, shell: u64) -> (UnboundedReceiver<Input>, Flow) {
        let (input, receiver) = unbounded_channel();
        let flow = Flow::default();
        self.list().insert(
            shell,
            Entry {
                input,
                flow: flow.clone(),
            },
        );
        (receiver, flow)
    }

    /// Sends input to an open shell; false when it is no longer open.
    pub fn send(&self, shell: u64, input: Input) -> bool {
        self.list()
            .get(&shell)
            .is_some_and(|entry| entry.input.send(input).is_ok())
    }

    /// The screen has drawn (or dropped) `bytes` of a shell's output, so it may read on.
    pub fn drawn(&self, shell: u64, bytes: usize) {
        if let Some(entry) = self.list().get(&shell) {
            let _ = (entry.flow.pending).try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                Some(n.saturating_sub(bytes))
            });
            entry.flow.drained.notify_one();
        }
    }

    /// Ends every open shell, e.g. when the vault locks: the credentials they signed in
    /// with must not outlive it.
    pub fn close_all(&self) {
        for entry in self.list().values() {
            let _ = entry.input.send(Input::Close);
        }
    }

    pub fn count(&self) -> usize {
        self.list().len()
    }

    fn list(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Entry>> {
        self.open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Signs in to `host`, starts a login shell on a PTY of `cols` × `rows`, and leaves it
/// running on the current Tokio runtime. Returns once the shell has started; its output
/// and end go to `screen`.
pub async fn open(
    host: &Host,
    shell: u64,
    cols: u32,
    rows: u32,
    shells: &Shells,
    screen: Arc<dyn Screen>,
) -> Result<()> {
    let session = ssh::authenticate(host, &Details::default()).await?;
    let started = timeout(Duration::from_secs(20), async {
        let mut channel = session.channel_open_session().await?;
        channel
            .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
            .await?;
        channel.request_shell(true).await?;
        // The replies come back on the channel: one for the PTY, one for the shell.
        // Anything the shell prints meanwhile is kept for the screen.
        let mut early = Vec::new();
        let mut replies = 0;
        while replies < 2 {
            match channel.wait().await {
                Some(ChannelMsg::Success) => replies += 1,
                Some(ChannelMsg::Failure) => bail!("遠端拒絕開啟 PTY 或 shell"),
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    early.extend_from_slice(&data);
                }
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => {
                    bail!("遠端在開啟 shell 時關閉了連線")
                }
                Some(_) => {}
            }
        }
        anyhow::Ok((channel, early))
    })
    .await
    .context("開啟遠端 shell 逾時")
    .and_then(|started| started);
    let (channel, early) = match started {
        Ok(started) => started,
        Err(error) => {
            ssh::disconnect(&session).await;
            return Err(error);
        }
    };
    let (receiver, flow) = shells.enter(shell);
    let shells = shells.clone();
    tokio::spawn(async move {
        if !early.is_empty() {
            flow.pending.fetch_add(early.len(), Ordering::AcqRel);
            screen.output(shell, &early);
        }
        let error = pump(channel, receiver, shell, screen.as_ref(), &flow)
            .await
            .err();
        ssh::disconnect(&session).await;
        shells.list().remove(&shell);
        screen.ended(shell, error.map(|error| format!("{error:#}")));
    });
    Ok(())
}

type Channel = russh::Channel<russh::client::Msg>;

/// Moves bytes both ways until either side ends the shell.
async fn pump(
    mut channel: Channel,
    mut input: UnboundedReceiver<Input>,
    shell: u64,
    screen: &dyn Screen,
    flow: &Flow,
) -> Result<()> {
    // Input that arrived while a write waited for the remote side to read.
    let mut queued = VecDeque::new();
    loop {
        while let Some(typed) = queued.pop_front() {
            if !apply(&channel, typed, &mut input, &mut queued).await? {
                let _ = channel.close().await;
                return Ok(());
            }
        }
        // The screen is far behind: wait for it before reading more.
        let paused = flow.pending.load(Ordering::Acquire) >= WINDOW;
        tokio::select! {
            message = channel.wait(), if !paused => match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    flow.pending.fetch_add(data.len(), Ordering::AcqRel);
                    screen.output(shell, &data);
                }
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => return Ok(()),
                Some(_) => {}
            },
            () = flow.drained.notified(), if paused => {}
            typed = input.recv() => {
                let Some(typed) = typed else {
                    let _ = channel.close().await;
                    return Ok(());
                };
                if !apply(&channel, typed, &mut input, &mut queued).await? {
                    let _ = channel.close().await;
                    return Ok(());
                }
            }
        }
    }
}

/// Carries out one input; false when it ends the shell. A write the remote side does
/// not read can still be interrupted by a close.
async fn apply(
    channel: &Channel,
    typed: Input,
    input: &mut UnboundedReceiver<Input>,
    queued: &mut VecDeque<Input>,
) -> Result<bool> {
    match typed {
        Input::Data(bytes) => {
            let send = channel.data(&bytes[..]);
            tokio::pin!(send);
            loop {
                tokio::select! {
                    sent = &mut send => {
                        sent?;
                        return Ok(true);
                    }
                    next = input.recv() => match next {
                        Some(Input::Close) | None => return Ok(false),
                        Some(other) => queued.push_back(other),
                    },
                }
            }
        }
        Input::Resize { cols, rows } => {
            channel.window_change(cols, rows, 0, 0).await?;
            Ok(true)
        }
        Input::Close => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_all_reaches_every_open_shell_and_numbers_are_not_reused() {
        let shells = Shells::default();
        let (a, b) = (shells.reserve(), shells.reserve());
        assert_ne!(a, b);
        let mut first_rx = shells.register(a);
        let mut second_rx = shells.register(b);
        assert_eq!(shells.count(), 2);
        assert!(shells.send(a, Input::Data(b"ls\r".to_vec())));
        assert!(!shells.send(99, Input::Data(vec![])));
        shells.close_all();
        assert!(matches!(first_rx.try_recv(), Ok(Input::Data(_))));
        assert!(matches!(first_rx.try_recv(), Ok(Input::Close)));
        assert!(matches!(second_rx.try_recv(), Ok(Input::Close)));
    }

    #[test]
    fn drawn_output_is_taken_off_the_shells_backlog() {
        let shells = Shells::default();
        let (_input, flow) = shells.enter(1);
        flow.pending.store(WINDOW + 10, Ordering::Release);
        shells.drawn(1, 10);
        assert_eq!(flow.pending.load(Ordering::Acquire), WINDOW);
        // More than is pending never wraps around.
        shells.drawn(1, 2 * WINDOW);
        assert_eq!(flow.pending.load(Ordering::Acquire), 0);
        // An unknown shell is ignored.
        shells.drawn(9, 1);
    }
}
