use std::io::{self, Read as _};
use std::os::fd::{AsFd as _, AsRawFd as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// One observation made by the continuously draining PTY transport.
///
/// A closed enum makes consumers handle output, orderly closure, and read
/// failure separately instead of treating every zero-byte wakeup as silence.
pub(crate) enum PtyOutputEvent {
    Data {
        observed_at: Instant,
        bytes: Box<[u8]>,
    },
    Closed,
    Failed(io::Error),
}

/// Continuously drains a PTY master independently of test-driver pacing.
///
/// GNU Emacs can make its shared slave open-file description nonblocking while
/// checking terminal input (`src/keyboard.c:8256`). If the master is drained
/// only when a test explicitly calls `read`, a full output queue makes GNU's
/// terminal `fwrite` lose bytes. A real terminal always has an independent
/// reader, so the harness must provide one too.
pub(crate) struct PtyOutputPump {
    receiver: Receiver<PtyOutputEvent>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PtyOutputPump {
    pub(crate) fn start(pty: &pty_process::blocking::Pty, name: &str) -> io::Result<Self> {
        let reader_fd = pty.as_fd().try_clone_to_owned()?;
        // SAFETY: `reader_fd` is an owned duplicate of the live PTY master and
        // is transferred exactly once into the blocking Pty wrapper.
        let mut reader = unsafe { pty_process::blocking::Pty::from_fd(reader_fd) };
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name(format!("tui-pty-output-{name}"))
            .spawn(move || {
                const POLL_SLICE_MS: i32 = 50;
                let mut buffer = [0_u8; 65_536];
                while !worker_stop.load(Ordering::Relaxed) {
                    let mut descriptor = libc::pollfd {
                        fd: reader.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    let ready = unsafe { libc::poll(&mut descriptor, 1, POLL_SLICE_MS) };
                    if ready < 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() == io::ErrorKind::Interrupted {
                            continue;
                        }
                        let _ = sender.send(PtyOutputEvent::Failed(error));
                        break;
                    }
                    if ready == 0 {
                        continue;
                    }
                    if descriptor.revents & libc::POLLIN != 0 {
                        match reader.read(&mut buffer) {
                            Ok(0) => {
                                let _ = sender.send(PtyOutputEvent::Closed);
                                break;
                            }
                            Ok(length) => {
                                if sender
                                    .send(PtyOutputEvent::Data {
                                        observed_at: Instant::now(),
                                        bytes: buffer[..length].into(),
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                            Err(error) => {
                                let _ = sender.send(PtyOutputEvent::Failed(error));
                                break;
                            }
                        }
                    } else if descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)
                        != 0
                    {
                        let _ = sender.send(PtyOutputEvent::Closed);
                        break;
                    }
                }
            })?;

        Ok(Self {
            receiver,
            stop,
            worker: Some(worker),
        })
    }

    pub(crate) fn try_recv(&self) -> Result<PtyOutputEvent, TryRecvError> {
        self.receiver.try_recv()
    }

    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<PtyOutputEvent, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }

    pub(crate) fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for PtyOutputPump {
    fn drop(&mut self) {
        self.shutdown();
    }
}
