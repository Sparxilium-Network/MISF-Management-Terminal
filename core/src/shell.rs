//! Interactive shells: a signed-in SSH session with a PTY and a login shell, kept open
//! after the request that opened it returns. Bytes typed in the app go straight to the
//! shell and its output comes straight back; neither passes through the audit log or
//! the request queue, so typing does not wait behind other work.

use crate::model::Host;
use crate::ssh::{self, Details};
use anyhow::{Context, Result, bail};
use russh::ChannelMsg;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::timeout;

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
    /// Raw output, escape sequences included, for a terminal emulator to draw.
    fn output(&self, shell: u64, data: &[u8]);
    /// The shell is gone: the remote side ended it, it was closed, or the link broke.
    fn ended(&self, shell: u64, error: Option<String>);
}

/// The open shells, shared by the worker that opens them and the app that types into
/// them.
#[derive(Clone, Default)]
pub struct Shells {
    open: Arc<Mutex<HashMap<u64, UnboundedSender<Input>>>>,
    next: Arc<AtomicU64>,
}

impl Shells {
    /// A number for the next shell, never reused.
    pub fn reserve(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Lists a shell as open and returns where its input arrives.
    pub fn register(&self, shell: u64) -> UnboundedReceiver<Input> {
        let (sender, receiver) = unbounded_channel();
        self.list().insert(shell, sender);
        receiver
    }

    /// Sends input to an open shell; false when it is no longer open.
    pub fn send(&self, shell: u64, input: Input) -> bool {
        self.list()
            .get(&shell)
            .is_some_and(|sender| sender.send(input).is_ok())
    }

    /// Ends every open shell, e.g. when the vault locks: the credentials they signed in
    /// with must not outlive it.
    pub fn close_all(&self) {
        for sender in self.list().values() {
            let _ = sender.send(Input::Close);
        }
    }

    pub fn count(&self) -> usize {
        self.list().len()
    }

    fn list(&self) -> std::sync::MutexGuard<'_, HashMap<u64, UnboundedSender<Input>>> {
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
    let receiver = shells.register(shell);
    let shells = shells.clone();
    tokio::spawn(async move {
        if !early.is_empty() {
            screen.output(shell, &early);
        }
        let error = pump(channel, receiver, shell, screen.as_ref()).await.err();
        ssh::disconnect(&session).await;
        shells.list().remove(&shell);
        screen.ended(shell, error.map(|error| format!("{error:#}")));
    });
    Ok(())
}

/// Moves bytes both ways until either side ends the shell.
async fn pump(
    mut channel: russh::Channel<russh::client::Msg>,
    mut input: UnboundedReceiver<Input>,
    shell: u64,
    screen: &dyn Screen,
) -> Result<()> {
    loop {
        tokio::select! {
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    screen.output(shell, &data);
                }
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => return Ok(()),
                Some(_) => {}
            },
            typed = input.recv() => match typed {
                Some(Input::Data(bytes)) => channel.data(&bytes[..]).await?,
                Some(Input::Resize { cols, rows }) => {
                    channel.window_change(cols, rows, 0, 0).await?;
                }
                Some(Input::Close) | None => {
                    let _ = channel.close().await;
                    return Ok(());
                }
            },
        }
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
}
