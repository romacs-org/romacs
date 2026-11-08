use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

const MAX_JOBS: usize = 16;
const MAX_INPUT: usize = 1024 * 1024;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

pub struct Request {
    pub argv: Vec<String>,
    pub input: Vec<u8>,
    pub directory: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug)]
pub struct Output {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub error: Option<String>,
}

struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Arc<Output>>,
    output: Option<Arc<Output>>,
    notified: bool,
}

/// Called from a worker thread once a job completes.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct Jobs {
    jobs: BTreeMap<u64, Job>,
    next_id: u64,
    completions: Arc<AtomicUsize>,
    workers: Vec<std::thread::JoinHandle<()>>,
    wake: Option<Wake>,
}

impl Jobs {
    /// No job and no worker: nothing of this process's threads.
    pub(crate) fn is_idle(&self) -> bool {
        self.jobs.is_empty() && self.workers.is_empty() && self.wake.is_none()
    }

    /// Wake the embedder's wait when a job completes, so it need not poll.
    pub fn set_wake(&mut self, wake: Wake) { self.wake = Some(wake); }
    /// The embedder's wake, for other work that ends a wait.
    pub(crate) fn wake(&self) -> Option<Wake> { self.wake.clone() }

    pub fn start(&mut self, request: Request) -> Result<u64, String> {
        if !cfg!(target_os = "linux") {
            return Err("asynchronous CLI jobs require Linux process-group supervision".into());
        }
        if self.jobs.len() >= MAX_JOBS {
            return Err("too many retained CLI jobs (maximum 16)".into());
        }
        if request.argv.first().is_none_or(|arg| arg.is_empty()) {
            return Err("CLI job requires a program".into());
        }
        if !request.directory.is_absolute() {
            return Err("CLI job directory must be absolute".into());
        }
        if request.input.len() > MAX_INPUT {
            return Err("CLI job input exceeds 1 MiB".into());
        }
        if request.timeout.is_zero() || request.timeout > Duration::from_secs(120) {
            return Err("CLI job timeout must be nonzero and at most 120 seconds".into());
        }
        let id = self.next_id.checked_add(1).ok_or("CLI job IDs exhausted")?;
        // Made on the submitting thread, whose host mode refuses it in a
        // world build: the worker's is its own.
        let command = crate::host::command(&request.argv[0]).map_err(|error| error.to_string())?;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let completions = self.completions.clone();
        let wake = self.wake.clone();
        let (sender, receiver) = mpsc::channel();
        self.workers.retain(|worker| !worker.is_finished());
        let worker = std::thread::Builder::new()
            .name(format!("cli-job-{id}"))
            .spawn(move || {
                let output = run(request, command, worker_cancel);
                if sender.send(Arc::new(output)).is_ok() {
                    completions.fetch_add(1, Ordering::Release);
                    if let Some(wake) = wake { wake(); }
                }
            })
            .map_err(|error| error.to_string())?;
        self.workers.push(worker);
        self.next_id = id;
        self.jobs.insert(
            id,
            Job {
                cancel,
                receiver,
                output: None,
                notified: false,
            },
        );
        Ok(id)
    }

    pub fn result(&mut self, id: u64) -> Result<Option<Arc<Output>>, String> {
        let job = self.jobs.get_mut(&id).ok_or("unknown CLI job")?;
        refresh(job);
        Ok(job.output.clone())
    }

    pub fn ready(&mut self) -> Vec<u64> {
        self.completions.swap(0, Ordering::AcqRel);
        self.jobs
            .iter_mut()
            .filter_map(|(&id, job)| {
                refresh(job);
                if job.output.is_some() && !job.notified {
                    job.notified = true;
                    Some(id)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        for job in self.jobs.values() {
            job.cancel.store(true, Ordering::Release);
        }
        let deadline = crate::host::monotonic() + Duration::from_secs(1);
        loop {
            self.workers.retain(|worker| !worker.is_finished());
            if self.workers.is_empty() { return Ok(()); }
            if crate::host::monotonic() >= deadline {
                return Err("CLI job cleanup did not settle within one second".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn has_ready(&self) -> bool {
        self.completions.load(Ordering::Acquire) != 0
    }

    pub fn forget(&mut self, id: u64) -> Result<(), String> {
        let job = self.jobs.remove(&id).ok_or("unknown CLI job")?;
        job.cancel.store(true, Ordering::Release);
        Ok(())
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn refresh(job: &mut Job) {
    if job.output.is_none()
        && let Ok(output) = job.receiver.try_recv()
    {
        job.output = Some(output);
    }
}

#[cfg(not(target_os = "linux"))]
fn run(_: Request, _: std::process::Command, _: Arc<AtomicBool>) -> Output {
    Output {
        status: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        error: Some("unsupported platform".into()),
    }
}

#[cfg(target_os = "linux")]
fn run(request: Request, mut command: std::process::Command, cancel: Arc<AtomicBool>) -> Output {
    use std::io::{Read, Write};
    use std::os::fd::AsFd;
    use std::os::unix::process::CommandExt;

    fn drain(reader: &mut impl Read, bytes: &mut Vec<u8>) -> Result<(), String> {
        let mut buffer = [0; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    let remaining = MAX_OUTPUT - bytes.len();
                    bytes.extend_from_slice(&buffer[..count.min(remaining)]);
                    if count > remaining {
                        return Err("CLI job output exceeds 4 MiB per stream".into());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    let mut output = Output {
        status: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        error: None,
    };
    if cancel.load(Ordering::Acquire) {
        output.error = Some("CLI job cancelled".into());
        return output;
    }
    let started = crate::host::monotonic();
    let child = command
        .args(&request.argv[1..])
        .current_dir(&request.directory)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            output.error = Some(error.to_string());
            return output;
        }
    };
    let pid = child.id() as libc::pid_t;
    crate::isolate::child_spawned(pid);
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let setup = crate::host::set_nonblocking(stdout.as_fd())
        .and_then(|()| crate::host::set_nonblocking(stderr.as_fd()))
        .and_then(|()| crate::host::set_nonblocking(stdin.as_ref().unwrap().as_fd()));
    let mut written = 0;
    if let Err(error) = &setup {
        output.error = Some(error.to_string());
    }
    while output.error.is_none() {
        if cancel.load(Ordering::Acquire) {
            output.error = Some("CLI job cancelled".into());
            break;
        }
        if crate::host::monotonic().saturating_duration_since(started) >= request.timeout {
            output.error = Some("CLI job timed out".into());
            break;
        }
        if let Err(error) = drain(&mut stdout, &mut output.stdout)
            .and_then(|()| drain(&mut stderr, &mut output.stderr))
        {
            output.error = Some(error);
            break;
        }
        if written == request.input.len() {
            stdin.take();
        } else if let Some(pipe) = stdin.as_mut() {
            match pipe.write(&request.input[written..]) {
                Ok(count) => written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                    stdin.take();
                }
                Err(error) => {
                    output.error = Some(error.to_string());
                    break;
                }
            }
        }
        match crate::subprocess::child_exited(pid) {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) => {
                output.error = Some(error.to_string());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Keep the leader unreaped until its group is killed, pinning the numeric PID.
    let _ = crate::host::kill(-pid, libc::SIGKILL);
    let _ = crate::host::kill(pid, libc::SIGKILL);
    stdin.take();
    let waited = child.wait();
    crate::isolate::child_reaped(pid);
    match waited {
        Ok(status) => output.status = status.code(),
        Err(error) => {
            output.error.get_or_insert_with(|| error.to_string());
        }
    }
    // Never await EOF: a descendant may have escaped the group with inherited pipes.
    if setup.is_ok()
        && let Err(error) = drain(&mut stdout, &mut output.stdout)
            .and_then(|()| drain(&mut stderr, &mut output.stderr))
    {
        output.error.get_or_insert(error);
    }
    output
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::Instant;

    fn request(script: &str) -> Request {
        Request {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            input: Vec::new(),
            directory: PathBuf::from("/"),
            timeout: Duration::from_secs(2),
        }
    }

    fn wait(jobs: &mut Jobs, id: u64) -> Arc<Output> {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if let Some(output) = jobs.result(id).unwrap() {
                return output;
            }
            assert!(Instant::now() < deadline, "job did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn stdin_cwd_status_and_repeatable_notifications() {
        let mut jobs = Jobs::default();
        let mut req = request("cat; pwd; printf error >&2; exit 7");
        req.input = "Ελληνικά 🦀\n".as_bytes().to_vec();
        let id = jobs.start(req).unwrap();
        let output = wait(&mut jobs, id);
        assert_eq!(output.stdout, "Ελληνικά 🦀\n/\n".as_bytes());
        assert_eq!(output.stderr, b"error");
        assert_eq!(output.status, Some(7));
        assert!(output.error.is_none());
        assert!(Arc::ptr_eq(&output, &jobs.result(id).unwrap().unwrap()));
        assert_eq!(jobs.ready(), vec![id]);
        assert!(jobs.ready().is_empty());
        jobs.forget(id).unwrap();
        assert!(jobs.result(id).is_err());
        assert_eq!(output.status, Some(7));
        let next = jobs.start(request("exit 0")).unwrap();
        assert!(next > id);
        wait(&mut jobs, next);
    }

    #[test]
    fn timeout_and_both_output_caps() {
        let mut jobs = Jobs::default();
        let mut req = request("sleep 30 & wait");
        req.timeout = Duration::from_millis(40);
        let id = jobs.start(req).unwrap();
        assert!(
            wait(&mut jobs, id)
                .error
                .as_ref()
                .unwrap()
                .contains("timed out")
        );
        for script in ["yes x", "yes x >&2"] {
            let id = jobs.start(request(script)).unwrap();
            let output = wait(&mut jobs, id);
            assert!(output.error.as_ref().unwrap().contains("4 MiB"));
            assert!(output.stdout.len() <= MAX_OUTPUT && output.stderr.len() <= MAX_OUTPUT);
        }
    }

    #[test]
    fn descendant_pipes_and_independent_jobs() {
        let mut jobs = Jobs::default();
        let slow = jobs.start(request("sleep 30")).unwrap();
        let quick = jobs.start(request("sleep 30 & printf done")).unwrap();
        assert_eq!(wait(&mut jobs, quick).stdout, b"done");
        assert!(jobs.result(slow).unwrap().is_none());
        assert_eq!(jobs.ready(), vec![quick]);
        jobs.forget(slow).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(jobs.ready().is_empty());
        assert!(!jobs.has_ready() || jobs.ready().is_empty());
    }

    #[test]
    fn validation_and_retention_bound() {
        let mut jobs = Jobs::default();
        let mut req = request("true");
        req.timeout = Duration::ZERO;
        assert!(jobs.start(req).is_err());
        let mut req = request("true");
        req.input = vec![0; MAX_INPUT + 1];
        assert!(jobs.start(req).is_err());
        for _ in 0..MAX_JOBS {
            jobs.start(request("true")).unwrap();
        }
        assert!(jobs.start(request("true")).is_err());
    }

    #[test]
    fn forgotten_and_dropped_processes_are_reaped() {
        for drop_owner in [false, true] {
            let directory = std::env::temp_dir().join(format!(
                "process-jobs-test-{}-{drop_owner}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("pids");
            let _ = std::fs::remove_file(&path);
            let mut jobs = Jobs::default();
            let mut req = request("echo $$ > pids; sleep 30 & echo $! >> pids; wait");
            req.directory = directory.clone();
            let id = jobs.start(req).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            let pids = loop {
                let pids: Vec<i32> = std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .split_whitespace()
                    .filter_map(|pid| pid.parse().ok())
                    .collect();
                if pids.len() == 2 {
                    break pids;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            };
            let before = Instant::now();
            if drop_owner {
                drop(jobs);
                let reaped = !std::path::Path::new(&format!("/proc/{}", pids[0])).exists();
                if !reaped {
                    // Settle the owned fixture before failing; no leaked probe child.
                    while std::path::Path::new(&format!("/proc/{}", pids[0])).exists()
                        && Instant::now() < deadline
                    {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                assert!(reaped, "PROCESS_OWNER_DROP_ACKNOWLEDGES_REAP_BEFORE_RETURN");
            } else {
                jobs.forget(id).unwrap();
            }
            assert!(before.elapsed() < Duration::from_millis(100));
            loop {
                let leader_reaped = !std::path::Path::new(&format!("/proc/{}", pids[0])).exists();
                let descendant_dead = std::fs::read_to_string(format!("/proc/{}/stat", pids[1]))
                    .map(|stat| stat.split_once(") ").unwrap().1.starts_with('Z'))
                    .unwrap_or(true);
                if leader_reaped && descendant_dead {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "owned leader not reaped or group descendant alive"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn full_duplex_and_complete_notification_set() {
        let mut jobs = Jobs::default();
        let mut req = request("head -c 1048576 /dev/zero; head -c 1048576 /dev/zero >&2; cat");
        req.input = vec![b'x'; MAX_INPUT];
        let id = jobs.start(req).unwrap();
        let output = wait(&mut jobs, id);
        assert!(output.error.is_none(), "{:?}", output.error);
        assert_eq!(output.stdout.len(), 2 * MAX_INPUT);
        assert_eq!(&output.stdout[MAX_INPUT..], vec![b'x'; MAX_INPUT]);
        assert_eq!(output.stderr.len(), MAX_INPUT);
        jobs.forget(id).unwrap();
        jobs.ready();
        let ids: Vec<_> = (0..MAX_JOBS)
            .map(|_| jobs.start(request("true")).unwrap())
            .collect();
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while seen.len() < ids.len() {
            if jobs.has_ready() {
                seen.extend(jobs.ready());
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        seen.sort_unstable();
        assert_eq!(seen, ids);
        assert!(jobs.ready().is_empty());
        assert!(!jobs.has_ready() || jobs.ready().is_empty());
        assert_eq!(output.stdout.len(), 2 * MAX_INPUT);
    }
}
