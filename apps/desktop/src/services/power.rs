use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use super_paste_core::{Error, Result};
use super::Services;

const WAKE_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
struct State { generation: u64, sleeping: Option<bool>, wake_at: Instant }

pub(super) struct PowerLifecycle {
    state: watch::Sender<State>,
    stopped: CancellationToken,
}

impl PowerLifecycle {
    pub(super) fn new() -> Self {
        let (state, _) = watch::channel(State { generation: 0, sleeping: None, wake_at: Instant::now() });
        Self { state, stopped: CancellationToken::new() }
    }
    pub(super) fn stop(&self) { self.stopped.cancel(); }
}

impl Services {
    pub fn power_event(&self, sleeping: bool) -> Result<()> {
        if self.power.stopped.is_cancelled() { return Err(Error::Cancelled); }
        let mut result = Ok(());
        // The watch write lock also serializes immediate suspension against
        // resuming an older generation. Native work never runs under this lock.
        self.power.state.send_if_modified(|state| {
            // Ignore duplicate notifications without extending wake grace or
            // restarting capture. None still admits a first wake during boot.
            if state.sleeping == Some(sleeping) { return false; }
            state.generation = state.generation.wrapping_add(1);
            state.sleeping = Some(sleeping);
            state.wake_at = Instant::now() + WAKE_GRACE;
            self.ocr.set_suspended(true);
            result = self.access.set_suspended(true);
            true
        });
        result
    }

    pub(super) async fn power_lifecycle(self: Arc<Self>) {
        let mut changes = self.power.state.subscribe();
        let mut generation = 0;
        loop {
            let event = tokio::select! { biased;
                _ = self.power.stopped.cancelled() => return,
                state = changes.wait_for(|state| state.generation != generation) => match state {
                    Ok(state) => *state,
                    Err(_) => return,
                },
            };
            generation = event.generation;
            // Retain the actual stop through callback and capture drain. A
            // subsequent event replaces the target, not this owned teardown.
            if let Err(error) = self.access.reconcile_power().await {
                if !self.power.stopped.is_cancelled() { tracing::error!(%error, "休眠时停止剪贴板监听失败"); }
            }
            if event.sleeping == Some(true) { continue; }
            tokio::select! { biased;
                _ = self.power.stopped.cancelled() => return,
                _ = changes.changed() => continue,
                // Count from the native wake event, including time spent
                // draining a previous capture, rather than adding another delay.
                _ = tokio::time::sleep_until(event.wake_at) => {},
            }
            let mut resume = Ok(false);
            self.power.state.send_if_modified(|current| {
                if current.generation == generation && current.sleeping == Some(false) && !self.power.stopped.is_cancelled() {
                    resume = self.access.set_suspended(false).map(|()| {
                        self.ocr.set_suspended(false);
                        true
                    });
                }
                false
            });
            match resume {
                Ok(true) => if let Err(error) = self.access.reconcile_power().await {
                    if !self.power.stopped.is_cancelled() { tracing::error!(%error, "唤醒后恢复剪贴板监听失败"); }
                },
                Ok(false) => {},
                Err(error) => tracing::error!(%error, "唤醒后恢复系统接入状态失败"),
            }
        }
    }
}
