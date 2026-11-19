//! Stopping a running remote operation at the user's request.
//!
//! Stopping drops the operation's SSH connections. A command still running on the host is
//! cut off with them (with a PTY it gets SIGHUP), so an interrupted upgrade may leave
//! apt/dpkg half-way; callers record such hosts as unknown.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;

/// The requests a [`Cancel`] can stop. Reboots are left out: the reboot is scheduled the
/// moment its command runs, and stopping cannot take that back.
pub const CANCELLABLE: [&str; 6] = [
    "check",
    "upgrade",
    "dist_upgrade",
    "batch",
    "test",
    "apt_diagnose",
];

/// A stop request shared by the UI and the worker; reset before each request.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Inner>);

#[derive(Default)]
struct Inner {
    requested: AtomicBool,
    notify: Notify,
}

impl Cancel {
    /// Asks the running request to stop.
    pub fn request(&self) {
        self.0.requested.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    /// Forgets a request meant for an earlier operation.
    pub fn reset(&self) {
        self.0.requested.store(false, Ordering::SeqCst);
    }

    pub fn is_requested(&self) -> bool {
        self.0.requested.load(Ordering::SeqCst)
    }

    /// Resolves once a stop is requested; at once if one already is.
    pub async fn requested(&self) {
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            // Registered before the check, so a request in between is not missed.
            notified.as_mut().enable();
            if self.is_requested() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_request_wakes_a_waiter_and_a_reset_forgets_it() {
        let cancel = Cancel::default();
        let waiter = cancel.clone();
        let waiting = tokio::spawn(async move { waiter.requested().await });
        tokio::task::yield_now().await;
        cancel.request();
        waiting.await.unwrap();
        // Already requested: resolves at once.
        cancel.requested().await;
        cancel.reset();
        assert!(!cancel.is_requested());
        let pending =
            tokio::time::timeout(std::time::Duration::from_millis(20), cancel.requested()).await;
        assert!(pending.is_err());
    }
}
