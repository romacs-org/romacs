use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Render thread state.
pub(crate) struct RenderThread {
    handle: Option<JoinHandle<()>>,
}

impl RenderThread {
    fn finish_spawn(
        handle: JoinHandle<()>,
        startup_rx: mpsc::Receiver<Result<(), String>>,
        startup_timeout: Duration,
    ) -> Result<Self, String> {
        match startup_rx.recv_timeout(startup_timeout) {
            Ok(Ok(())) => Ok(Self {
                handle: Some(handle),
            }),
            Ok(Err(err)) => {
                let _ = handle.join();
                Err(err)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
                "Timed out waiting {:?} for render thread startup",
                startup_timeout
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = handle.join();
                Err("Render thread terminated before reporting startup status".to_string())
            }
        }
    }

    #[allow(dead_code)] // spawn helpers exercised by the thread_handle tests
    fn spawn_with_state_timeout<T, S, R>(
        startup: S,
        runner: R,
        startup_timeout: Duration,
    ) -> Result<Self, String>
    where
        T: Send + 'static,
        S: FnOnce() -> Result<T, String> + Send + 'static,
        R: FnOnce(T) -> Result<(), String> + Send + 'static,
    {
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || match startup() {
            Ok(state) => {
                let _ = startup_tx.send(Ok(()));
                if let Err(err) = runner(state) {
                    tracing::error!("Render thread exited with error: {}", err);
                }
            }
            Err(err) => {
                let _ = startup_tx.send(Err(err));
            }
        });

        Self::finish_spawn(handle, startup_rx, startup_timeout)
    }

    #[allow(dead_code)] // exercised by the thread_handle tests
    fn spawn_with_state<T, S, R>(startup: S, runner: R) -> Result<Self, String>
    where
        T: Send + 'static,
        S: FnOnce() -> Result<T, String> + Send + 'static,
        R: FnOnce(T) -> Result<(), String> + Send + 'static,
    {
        Self::spawn_with_state_timeout(startup, runner, Duration::from_secs(10))
    }

    #[cfg(test)]
    /// Wait for render thread to finish.
    pub fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
#[path = "thread_handle/tests/thread_handle_test.rs"]
mod tests;
