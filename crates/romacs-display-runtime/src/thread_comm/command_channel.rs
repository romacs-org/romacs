//! Capacity-bounded command transport with CPU-only startup extraction.
//! Shutdown is one independent coalescing bit; len includes that pending bit.
//! A paint replaces a paint not yet taken (the newest frame is what draws).
//!
//! Deferred startup already applies window/configuration/clipboard commands
//! ahead of staged GPU work. Extract the same commands without moving or
//! discarding the GPU backlog when staging is full. Each class retains FIFO.

#[cfg(test)]
mod tests;

use super::{LifecycleCommand, RenderCommand};
use crossbeam_channel::{SendError, Sender, TryRecvError, TrySendError, bounded};
#[cfg(test)]
use crossbeam_channel::Receiver;
#[cfg(test)]
use crossbeam_channel::{RecvError, RecvTimeoutError};
#[cfg(test)]
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, Weak};

struct Pending {
    queue: VecDeque<RenderCommand>,
    receiver_alive: bool,
    shutdown: bool,
}
struct Shared {
    pending: Mutex<Pending>,
    space: Condvar,
    capacity: usize,
}
#[derive(Clone)]
pub(crate) struct CommandSender {
    shared: Arc<Shared>,
    wake: Arc<Sender<()>>,
}
pub(crate) struct CommandReceiver {
    shared: Arc<Shared>,
    wake: Weak<Sender<()>>,
    #[cfg(test)]
    available: Receiver<()>,
}

/// Create a finite transport. The wake channel carries no command payloads.
pub(crate) fn command_channel(capacity: usize) -> (CommandSender, CommandReceiver) {
    assert!(capacity > 0);
    let shared = Arc::new(Shared {
        pending: Mutex::new(Pending {
            queue: VecDeque::new(),
            receiver_alive: true,
            shutdown: false,
        }),
        space: Condvar::new(),
        capacity,
    });
    let (wake, available) = bounded(1);
    // Only tests wait on the wake channel; production polls the queue.
    #[cfg(not(test))]
    drop(available);
    let wake = Arc::new(wake);
    (
        CommandSender {
            shared: shared.clone(),
            wake: wake.clone(),
        },
        CommandReceiver {
            shared,
            wake: Arc::downgrade(&wake),
            #[cfg(test)]
            available,
        },
    )
}

impl CommandSender {
    pub fn try_send(&self, command: RenderCommand) -> Result<(), TrySendError<RenderCommand>> {
        self.try_send_within(command, self.shared.capacity)
    }
    /// As `try_send`, but `Full` once LIMIT commands wait: a producer that
    /// must leave the rest of the channel to others.
    #[expect(clippy::result_large_err, reason = "a refused command comes back to its sender whole, as try_send's does")]
    pub fn try_send_within(&self, command: RenderCommand, limit: usize) -> Result<(), TrySendError<RenderCommand>> {
        let mut pending = self.shared.pending.lock().unwrap();
        if !pending.receiver_alive {
            return Err(TrySendError::Disconnected(command));
        }
        if matches!(&command, RenderCommand::Lifecycle(LifecycleCommand::Shutdown)) {
            // Cancellation is one coalescing bit, independent of FIFO capacity.
            pending.shutdown = true;
        } else {
            // A paint is a mailbox per top-level frame: it replaces a paint
            // of its own frame the renderer has not taken, and goes behind
            // what was sent since, so its producer never waits for drawing
            // and the renderer draws each frame's newest.  Another frame's
            // paint is never replaced.
            let superseded = paint_frame(&command)
                .and_then(|frame| pending.queue.iter().position(|queued| paint_frame(queued) == Some(frame)));
            if !exempt_from_capacity(&command)
                && pending.queue.len() - usize::from(superseded.is_some()) >= limit.min(self.shared.capacity)
            {
                return Err(TrySendError::Full(command));
            }
            if let Some(index) = superseded { pending.queue.remove(index); }
            pending.queue.push_back(command);
            coalesce_exempt(&mut pending.queue);
        }
        // Publish and notify under the same lock.
        let _ = self.wake.try_send(());
        Ok(())
    }
    pub fn send(&self, mut command: RenderCommand) -> Result<(), SendError<RenderCommand>> {
        loop {
            match self.try_send(command) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Disconnected(command)) => return Err(SendError(command)),
                Err(TrySendError::Full(full)) => command = full,
            }
            let mut pending = self.shared.pending.lock().unwrap();
            while pending.receiver_alive && pending.queue.len() >= self.shared.capacity {
                pending = self.shared.space.wait(pending).unwrap();
            }
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        let pending = self.shared.pending.lock().unwrap();
        pending.queue.len() + usize::from(pending.shutdown)
    }
}

/// The top-level frame a paint draws, if COMMAND is one.
fn paint_frame(command: &RenderCommand) -> Option<u64> {
    match command {
        RenderCommand::Paint(scene) => Some(scene.root.frame_placement.frame().get()),
        _ => None,
    }
}

/// Commands too small and rare to bound and never to be refused or kept
/// waiting, though they keep their place in order: a frame's opacity
/// setting (each setter counts, GNU's nil keeps the last scalar applied)
/// and a window's destruction.  A producer of them never blocks on a
/// render thread that may itself wait on the producer's input.
fn exempt_from_capacity(command: &RenderCommand) -> bool {
    matches!(
        command,
        RenderCommand::Window(super::WindowCommand::SetFrameAlpha { .. } | super::WindowCommand::DestroyWindow { .. })
    )
}

/// Keep the exempt commands bounded without changing what they do, after
/// the newest was queued: a frame's destruction once, and a frame's alpha
/// setters only those that decide its state: the last, which sets its pair
/// and limit, and the last to set each of its active and inactive alphas,
/// which decide the scalar applied whatever its focus (a negative alpha,
/// GNU's nil, keeps the scalar the setter before applied).  At most three
/// setters and one destruction wait per frame however fast Lisp sets them.
fn coalesce_exempt(queue: &mut VecDeque<RenderCommand>) {
    use super::WindowCommand::{DestroyWindow, SetFrameAlpha};
    let keep: Vec<bool> = (0..queue.len()).map(|index| match &queue[index] {
        RenderCommand::Window(DestroyWindow { frame }) => !queue.iter().skip(index + 1)
            .any(|later| matches!(later, RenderCommand::Window(DestroyWindow { frame: other }) if other == frame)),
        RenderCommand::Window(SetFrameAlpha { frame, alpha, .. }) => {
            let later: Vec<[f32; 2]> = queue.iter().skip(index + 1).filter_map(|later| match later {
                RenderCommand::Window(SetFrameAlpha { frame: other, alpha, .. }) if other == frame => Some(*alpha),
                _ => None,
            }).collect();
            later.is_empty() || (0..2).any(|side| alpha[side] >= 0.0 && later.iter().all(|next| next[side] < 0.0))
        }
        _ => true,
    }).collect();
    if keep.iter().all(|kept| *kept) { return; }
    let mut keep = keep.into_iter();
    queue.retain(|_| keep.next().unwrap_or(true));
}

fn startup_command(command: &RenderCommand, daemon: bool) -> bool {
    matches!(command, RenderCommand::Window(super::WindowCommand::RealizeFrame { .. }))
        || (daemon && matches!(command,
            RenderCommand::Window(_) | RenderCommand::Config(_) | RenderCommand::Clipboard(_)
        ))
}

impl CommandReceiver {
    fn take(
        &self,
        select: impl FnOnce(&VecDeque<RenderCommand>) -> Option<usize>,
    ) -> Result<RenderCommand, TryRecvError> {
        let mut pending = self.shared.pending.lock().unwrap();
        if std::mem::take(&mut pending.shutdown) {
            self.rearm(&pending);
            return Ok(RenderCommand::Lifecycle(LifecycleCommand::Shutdown));
        }
        if let Some(index) = select(&pending.queue) {
            let command = pending.queue.remove(index).unwrap();
            self.shared.space.notify_all();
            self.rearm(&pending);
            return Ok(command);
        }
        // Unserviceable startup work still belongs to this stream.
        if pending.queue.is_empty() && self.wake.strong_count() == 0 {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }
    pub fn try_recv(&self) -> Result<RenderCommand, TryRecvError> {
        self.take(|queue| (!queue.is_empty()).then_some(0))
    }
    fn rearm(&self, pending: &Pending) {
        if (!pending.queue.is_empty() || pending.shutdown)
            && let Some(wake) = self.wake.upgrade()
        {
            let _ = wake.try_send(());
        }
    }
    pub(crate) fn take_shutdown(&self) -> bool {
        let mut pending = self.shared.pending.lock().unwrap();
        let shutdown = std::mem::take(&mut pending.shutdown);
        if shutdown {
            self.rearm(&pending);
        }
        shutdown
    }
    #[cfg(test)]
    pub(crate) fn try_recv_startup(&self) -> Result<RenderCommand, TryRecvError> {
        self.try_recv_startup_for(true, true)
    }
    pub(crate) fn try_recv_startup_for(
        &self,
        staging_full: bool,
        daemon: bool,
    ) -> Result<RenderCommand, TryRecvError> {
        self.take(|queue| {
            if staging_full {
                queue.iter().position(|command| startup_command(command, daemon))
            } else {
                (!queue.is_empty()).then_some(0)
            }
        })
    }
    pub(crate) fn has_startup_command_for(&self, daemon: bool) -> bool {
        let pending = self.shared.pending.lock().unwrap();
        pending.shutdown || pending.queue.iter().any(|command| startup_command(command, daemon))
    }
    #[cfg(test)]
    pub fn recv(&self) -> Result<RenderCommand, RecvError> {
        loop {
            match self.try_recv() {
                Ok(command) => return Ok(command),
                Err(TryRecvError::Disconnected) => return Err(RecvError),
                Err(TryRecvError::Empty) => {}
            }
            // Recheck even on disconnect: a last command may have been admitted.
            let _ = self.available.recv();
        }
    }
    #[cfg(test)]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<RenderCommand, RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.try_recv() {
                Ok(command) => return Ok(command),
                Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
                Err(TryRecvError::Empty) => {}
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(RecvTimeoutError::Timeout);
            };
            match self.available.recv_timeout(remaining) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
                Err(RecvTimeoutError::Timeout) => return Err(RecvTimeoutError::Timeout),
            }
        }
    }
    #[cfg(test)]
    pub fn try_iter(&self) -> impl Iterator<Item = RenderCommand> + '_ {
        std::iter::from_fn(|| self.try_recv().ok())
    }
    pub fn len(&self) -> usize {
        let pending = self.shared.pending.lock().unwrap();
        pending.queue.len() + usize::from(pending.shutdown)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[cfg(test)]
    pub(crate) fn available(&self) -> &Receiver<()> {
        &self.available
    }
}
impl Drop for CommandReceiver {
    fn drop(&mut self) {
        let commands = {
            let mut pending = self.shared.pending.lock().unwrap();
            pending.receiver_alive = false;
            pending.shutdown = false;
            std::mem::take(&mut pending.queue)
        };
        self.shared.space.notify_all();
        // Drop queued replies/resources outside the transport lock.
        drop(commands);
    }
}
