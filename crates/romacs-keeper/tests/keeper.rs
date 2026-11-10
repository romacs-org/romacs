//! The keeper's bars, each against a real keeper process on its own socket.
//! With `ROMACS_KEEPER_TEST_UNIT=1` every keeper runs as a transient
//! systemd user unit, the way a session started as a unit starts it.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use romacs_keeper::proto::Row;
use romacs_keeper::spawn::{End, Spec, Status};
use romacs_keeper::{proto, Client, Event};
use romacs_record::ClassRange;

const KEEPER: &str = env!("CARGO_BIN_EXE_romacs-keeper");

const NEXT: &str = env!("CARGO_BIN_EXE_romacs-keeper-test-next");
const BROKEN: &str = env!("CARGO_BIN_EXE_romacs-keeper-test-broken");
const SESSION: &str = env!("CARGO_BIN_EXE_romacs-keeper-test-session");

/// A keeper on a socket of its own, shut down when dropped.
struct Fixture { socket: PathBuf, dir: PathBuf }

static FIXTURES: AtomicU32 = AtomicU32::new(0);

impl Fixture {
    fn start(idle_ms: u64) -> Self {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = runtime.join(format!("rk{}-{}", std::process::id(), FIXTURES.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("keeper.sock");
        let idle = idle_ms.to_string();
        let status = if std::env::var_os("ROMACS_KEEPER_TEST_UNIT").is_some() {
            Command::new("systemd-run").args(["--user", "--quiet", "--collect", "--"]).arg(KEEPER)
                .args(["--socket", socket.to_str().unwrap(), "--foreground", "--idle-ms", &idle]).status().unwrap()
        } else {
            Command::new(KEEPER).args(["--socket", socket.to_str().unwrap(), "--idle-ms", &idle]).status().unwrap()
        };
        assert!(status.success(), "the keeper started");
        let fixture = Self { socket, dir };
        wait_for("the keeper's socket", || Client::attach(&fixture.socket).is_ok());
        fixture
    }

    fn client(&self) -> Client { Client::attach(&self.socket).unwrap() }

    fn session(&self) -> Session { Session::start(&self.socket) }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(mut client) = Client::attach(&self.socket) { let _ = client.shutdown(); }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Another session process (`tests/bin/session.rs`).
struct Session { child: Child, input: ChildStdin, output: BufReader<ChildStdout>, pid: u32 }

impl Session {
    fn start(socket: &Path) -> Self {
        let mut child = Command::new(SESSION).arg(socket).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let (input, output) = (child.stdin.take().unwrap(), BufReader::new(child.stdout.take().unwrap()));
        let mut session = Self { child, input, output, pid: 0 };
        session.pid = session.answer().strip_prefix("pid ").unwrap().parse().unwrap();
        session
    }

    fn answer(&mut self) -> String {
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        line.trim_end().to_string()
    }

    fn ask(&mut self, command: &str) -> String {
        writeln!(self.input, "{command}").unwrap();
        self.answer()
    }

    /// Spawn `sh -c SCRIPT` with its stdout a pipe; the handle and the pid.
    fn spawn(&mut self, script: &str) -> (romacs_record::Handle, u32) {
        let answer = self.ask(&format!("spawn {script}"));
        let (handle, pid) = answer.split_once(' ').unwrap_or_else(|| panic!("spawn: {answer}"));
        (romacs_record::Handle::from_raw(handle.parse().unwrap()).unwrap(), pid.parse().unwrap())
    }

    fn kill(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn shell(script: &str) -> Spec {
    Spec { argv: vec![b"/bin/sh".to_vec(), b"-c".to_vec(), script.as_bytes().to_vec()], env: vec![b"PATH=/usr/bin:/bin".to_vec()],
           cwd: b"/".to_vec(), umask: 0o022, fds: vec![(0, End::Null), (1, End::Writes), (2, End::Null)], ..Spec::default() }
}

fn read_all(fd: &OwnedFd) -> String {
    let mut text = String::new();
    std::fs::File::from(fd.try_clone().unwrap()).read_to_string(&mut text).unwrap();
    text
}

fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| !stat.rsplit(") ").next().unwrap().starts_with('Z'))
}

fn parent_of(pid: u32) -> u32 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    stat.rsplit(") ").next().unwrap().split(' ').nth(1).unwrap().parse().unwrap()
}

fn row(client: &mut Client, handle: romacs_record::Handle) -> Option<Row> {
    client.list().unwrap().into_iter().find(|row| row.handle == handle)
}

/// Wait for a status event of HANDLE that satisfies WANTED.
fn status_event(client: &mut Client, handle: romacs_record::Handle, wanted: impl Fn(Status) -> bool) -> Status {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    loop {
        for event in client.pump().unwrap() {
            if let Event::Status(row) = event && row.handle == handle {
                if wanted(row.status) { return row.status; }
                seen.push(row.status);
            }
        }
        assert!(Instant::now() < deadline, "no wanted status event of handle {handle}; events {seen:?}, its row {:?}", row(client, handle));
        std::thread::sleep(Duration::from_millis(5));
    }
}

const COUNTER: &str = "i=0; while [ $i -lt 30000 ]; do echo $i; i=$((i+1)); done; exit 7";

fn counter_output() -> String {
    use std::fmt::Write;
    (0..30000).fold(String::new(), |mut text, i| { writeln!(text, "{i}").unwrap(); text })
}

#[test]
fn a_child_starts_with_exactly_what_its_spec_names() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let dir = keeper.dir.to_str().unwrap();
    let spec = Spec { env: vec![b"PATH=/usr/bin:/bin".to_vec(), b"GREETING=hello keeper".to_vec()], cwd: dir.as_bytes().to_vec(), umask: 0o027,
                      sigmask: 1 << (libc::SIGUSR1 - 1), setsid: true,
                      ..shell("umask; pwd; echo \"$GREETING\"; env | grep -c -e ^HOME= -e ^USER= -e ^XDG_; grep SigBlk /proc/self/status; ps -o sid= -p $$; ls /proc/self/fd | wc -l; echo $PPID") };
    let (row, fds) = client.spawn("test", 1, b"meta", false, &spec).unwrap();
    let text = read_all(&fds[0]);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    assert_eq!(lines[0], "0027");
    assert_eq!(lines[1], dir);
    assert_eq!(lines[2], "hello keeper");
    assert_eq!(lines[3], "0", "nothing of the keeper's or the session's environment: {text}");
    assert_eq!(u64::from_str_radix(lines[4].trim_start_matches("SigBlk:").trim(), 16).unwrap(), 1 << (libc::SIGUSR1 - 1));
    assert_eq!(lines[5], row.pid.to_string(), "it leads its own session");
    assert_eq!(lines[6], "4", "only the three named descriptors, and the directory ls reads: {text}");
    assert_eq!((row.tag.as_str(), row.version, row.metadata.as_slice()), ("test", 1, &b"meta"[..]));
    assert_eq!(lines[7], client.keeper.pid.to_string(), "the keeper is the parent");
}

#[test]
fn a_spawn_that_cannot_exec_is_refused_with_the_reason() {
    let keeper = Fixture::start(60_000);
    let spec = Spec { argv: vec![b"/nonexistent/program".to_vec()], ..shell("") };
    let error = keeper.client().spawn("test", 1, b"", false, &spec).unwrap_err();
    assert!(error.contains("/nonexistent/program") && error.contains("No such file"), "{error}");
    assert!(keeper.client().list().unwrap().is_empty());
}

/// The session dies with -9 while its child writes: the child blocks,
/// alive, and a second session reads the rest, in order, then the status.
#[test]
fn a_killed_session_leaves_its_child_blocked_and_a_second_reads_the_rest() {
    let keeper = Fixture::start(60_000);
    let mut first = keeper.session();
    let (handle, pid) = first.spawn(COUNTER);
    let expected = counter_output();
    let consumed = first.ask(&format!("read {handle} 1000")).replace(',', "\n");
    assert_eq!(consumed, expected[..1000]);
    first.kill();
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(pid), "the child outlives its session");
    let mut second = keeper.client();
    let listed = row(&mut second, handle).unwrap();
    assert_eq!((listed.status, listed.owner_alive), (Status::Running, false), "blocked on a full pipe, not dead");
    let (_, fds) = second.claim(handle, 0).unwrap();
    assert_eq!(read_all(&fds[0]), expected[1000..], "every byte the first did not consume, once and in order");
    assert_eq!(status_event(&mut second, handle, Status::ended), Status::Exited(7));
    assert_eq!(row(&mut second, handle).unwrap().status, Status::Exited(7), "the status stays in the row");
}

#[test]
fn two_successors_compete_for_a_handle_and_one_gets_it() {
    let keeper = Fixture::start(60_000);
    let mut owner = keeper.session();
    let (handle, _) = owner.spawn("sleep 600");
    owner.kill();
    let (mut a, mut b) = (keeper.session(), keeper.session());
    let answers = [a.ask(&format!("claim {handle}")), b.ask(&format!("claim {handle}"))];
    assert_eq!(answers[0], "Running");
    assert_eq!(answers[1], format!("error handle {handle} is owned by session {}, which is alive", a.pid));
}

#[test]
fn a_stopped_predecessor_is_killed_before_its_descriptors_are_handed_on() {
    let keeper = Fixture::start(60_000);
    let mut old = keeper.session();
    let (handle, _) = old.spawn("sleep 600");
    // SAFETY: kill takes a pid and a signal; the pid is this test's child.
    unsafe { libc::kill(old.pid.cast_signed(), libc::SIGSTOP) };
    let mut successor = keeper.client();
    let refused = successor.claim(handle, 0).unwrap_err();
    assert!(refused.contains(&format!("owned by session {}", old.pid)), "{refused}");
    successor.claim(handle, old.pid).unwrap();
    assert_eq!(old.child.wait().unwrap().code(), None, "the predecessor was killed");
    assert_eq!(row(&mut successor, handle).unwrap().owner, std::process::id());
}

#[test]
fn a_quit_closes_what_is_not_kept_and_a_crash_leaves_everything() {
    let keeper = Fixture::start(60_000);
    let mut quitting = keeper.session();
    let (closed, plain_pid) = quitting.spawn("sleep 600");
    let (spared, kept_pid) = quitting.spawn("sleep 600");
    assert_eq!(quitting.ask(&format!("kept {spared}")), "ok");
    assert_eq!(quitting.ask("quit"), "ok");
    let mut observer = keeper.client();
    wait_for("the plain child's end", || !alive(plain_pid));
    wait_for("the plain handle's removal", || row(&mut observer, closed).is_none());
    assert!(alive(kept_pid) && row(&mut observer, spared).is_some());

    let mut crashing = keeper.session();
    let (left, left_pid) = crashing.spawn("sleep 600");
    crashing.kill();
    // No timeout of the keeper's closes a handle, however slow the successor.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(alive(left_pid));
    assert!(!row(&mut observer, left).unwrap().owner_alive);
    // The successor's sweep closes what nobody claimed, but not what is kept.
    assert_eq!(observer.close_orphans().unwrap(), 1);
    wait_for("the orphan's end", || !alive(left_pid));
    assert!(alive(kept_pid));
}

#[test]
fn a_deadline_closes_only_a_handle_nobody_owns() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let mut owner = keeper.session();
    let (orphan, orphan_pid) = owner.spawn("sleep 600");
    owner.kill();
    let (mine, _) = client.spawn("test", 1, b"", false, &shell("sleep 600")).unwrap();
    let soon = romacs_keeper::server::now_ms() + 300;
    client.set_deadline(orphan, soon).unwrap();
    client.set_deadline(mine.handle, soon).unwrap();
    wait_for("the deadline", || !alive(orphan_pid));
    assert!(alive(mine.pid));
    assert_eq!(row(&mut client, mine.handle).unwrap().deadline, soon, "under a live owner a deadline waits");
    // A deadline that passed while the owner lived holds when it goes,
    // unless a successor claimed the handle first.
    let mut leaving = keeper.session();
    let (late, late_pid) = leaving.spawn("sleep 600");
    let (claimed, claimed_pid) = leaving.spawn("sleep 600");
    client.set_deadline(late, romacs_keeper::server::now_ms() + 100).unwrap();
    client.set_deadline(claimed, romacs_keeper::server::now_ms() + 100).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert!(alive(late_pid) && alive(claimed_pid));
    client.claim(claimed, leaving.pid).unwrap();
    wait_for("the overdue handle's end", || !alive(late_pid));
    assert!(alive(claimed_pid));
    assert_eq!(row(&mut client, claimed).unwrap().deadline, 0);
}

#[test]
fn signals_go_by_handle_and_one_for_an_exited_child_hits_nothing() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let (row, _fds) = client.spawn("test", 1, b"", false, &shell("sleep 600")).unwrap();
    client.signal(row.handle, libc::SIGSTOP, false).unwrap();
    assert_eq!(status_event(&mut client, row.handle, |_| true), Status::Stopped(libc::SIGSTOP));
    client.signal(row.handle, libc::SIGCONT, false).unwrap();
    assert_eq!(status_event(&mut client, row.handle, |_| true), Status::Continued);
    client.signal(row.handle, libc::SIGTERM, false).unwrap();
    assert_eq!(status_event(&mut client, row.handle, Status::ended), Status::Signaled(libc::SIGTERM));
    let error = client.signal(row.handle, libc::SIGTERM, false).unwrap_err();
    assert_eq!(error, format!("the child of handle {} has exited", row.handle));
}

#[test]
fn closing_a_slot_delivers_end_of_file_once_every_holder_closed() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let spec = Spec { fds: vec![(0, End::Reads), (1, End::Writes), (2, End::Null)], ..shell("cat; echo done") };
    let (row, mut fds) = client.spawn("test", 1, b"", false, &spec).unwrap();
    std::fs::File::from(fds[0].try_clone().unwrap()).write_all(b"through\n").unwrap();
    drop(fds.remove(0));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(self::row(&mut client, row.handle).unwrap().status, Status::Running, "the keeper's copy keeps the pipe open");
    client.close_slot(row.handle, 0).unwrap();
    assert_eq!(read_all(&fds[0]), "through\ndone\n");
    assert_eq!(self::row(&mut client, row.handle).unwrap().open, 0b10);
}

#[test]
fn stored_descriptors_come_back_to_whoever_claims_the_handle() {
    let keeper = Fixture::start(60_000);
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut first = keeper.client();
    let stored = first.store("socket", 3, b"peer", true, &[theirs.as_fd()]).unwrap();
    drop(theirs);
    let (row, fds) = first.claim(stored.handle, 0).unwrap();
    assert_eq!((row.tag.as_str(), row.version, row.kept, row.pid), ("socket", 3, true, 0));
    (&ours).write_all(b"kept").unwrap();
    let mut bytes = [0; 4];
    std::os::unix::net::UnixStream::from(fds.into_iter().next().unwrap()).read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"kept");
}

/// A process that inherited a session's old socket (here a clone, as a
/// forked child would hold) does not take what is meant for the session.
#[test]
fn a_sessions_newest_connection_is_the_one_it_is_told_on() {
    let keeper = Fixture::start(60_000);
    let stale = keeper.client();
    let _held = stale.as_fd().try_clone_to_owned().unwrap();
    drop(stale);
    let mut client = keeper.client();
    let (row, _fds) = client.spawn("test", 1, b"", false, &shell("exit 9")).unwrap();
    assert_eq!(status_event(&mut client, row.handle, Status::ended), Status::Exited(9));
}

#[test]
fn a_keeper_with_no_handle_and_no_session_exits() {
    let keeper = Fixture::start(300);
    let mut client = keeper.client();
    let pid = client.info().unwrap().pid;
    let (row, _fds) = client.spawn("test", 1, b"", false, &shell("sleep 600")).unwrap();
    drop(client);
    std::thread::sleep(Duration::from_millis(900));
    assert!(alive(pid), "a handle keeps the keeper");
    let mut client = keeper.client();
    client.close(row.handle, libc::SIGKILL).unwrap();
    std::thread::sleep(Duration::from_millis(900));
    assert!(alive(pid), "a session keeps the keeper");
    drop(client);
    wait_for("the idle keeper's exit", || !alive(pid));
    assert!(!keeper.socket.exists());
}

#[test]
fn the_keepers_death_is_reported_by_pid() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let pid = client.info().unwrap().pid;
    // SAFETY: kill takes a pid and a signal.
    unsafe { libc::kill(pid.cast_signed(), libc::SIGKILL) };
    assert_eq!(client.list().unwrap_err(), format!("the keeper (pid {pid}) of this session is gone"));
}

#[test]
fn a_second_keeper_for_a_socket_leaves_the_first_in_place() {
    let keeper = Fixture::start(60_000);
    let pid = keeper.client().info().unwrap().pid;
    assert!(Command::new(KEEPER).args(["--socket", keeper.socket.to_str().unwrap()]).status().unwrap().success());
    assert_eq!(keeper.client().info().unwrap().pid, pid);
}

/// A session built with one more version of the keeper class and one more
/// class attaches and uses what both know; a class with no version in
/// common is refused by name.
#[test]
fn a_newer_session_uses_the_classes_both_know() {
    let keeper = Fixture::start(60_000);
    let newer = vec![ClassRange::new(proto::CLASS, proto::NAME, 1..=2), ClassRange::new(9, "future", 1..=1)];
    let mut client = Client::connect(&keeper.socket, newer).unwrap();
    assert_eq!(client.version(proto::CLASS), Ok(1));
    assert_eq!(client.version(9), Err("the peer has no class \"future\"".into()));
    assert!(client.list().unwrap().is_empty());
    let unknown = client.request(999, &[], &[]).unwrap_err();
    assert_eq!(unknown, "this keeper has no request 999", "a kind the keeper lacks is refused, and the connection lives");
    assert!(client.list().is_ok());
    let apart = Client::connect(&keeper.socket, vec![ClassRange::new(proto::CLASS, proto::NAME, 2..=3)]).err().unwrap();
    assert_eq!(apart, "class \"keeper\" has no version in common: ours 2-3, the peer's 1-1");
}

#[test]
fn another_framing_is_refused_with_both_hashes() {
    let keeper = Fixture::start(60_000);
    let mut socket = std::os::unix::net::UnixStream::connect(&keeper.socket).unwrap();
    let mut hello = romacs_record::Hello { epoch: 0, pid: 1, classes: vec![proto::class()] }.record();
    hello.payload[8] ^= 1;
    let mut wire = Vec::new();
    hello.encode_into(&mut wire);
    socket.write_all(&wire).unwrap();
    let mut rest = Vec::new();
    socket.read_to_end(&mut rest).unwrap();
    let mut decoder = romacs_record::Decoder::default();
    decoder.push(&rest);
    assert!(romacs_record::Hello::read(&decoder.pop().unwrap().unwrap()).is_ok(), "the keeper said who it is");
    assert_eq!(decoder.pop().unwrap(), None, "and then hung up on the stranger");
    let error = romacs_record::Hello::read(&hello).unwrap_err().to_string();
    assert!(error.starts_with("protocol framing differs: ours "), "{error}");
}

/// The exec bar: into a build with a newer table, with a session attached,
/// a stopped child, a child reaped but unreported, a deadline, and 100
/// handles; everything is carried and the pid is the same.
#[test]
fn an_exec_in_place_carries_everything() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let before = client.info().unwrap();
    let mut gone = keeper.session();
    let (unreported, _) = gone.spawn("exit 5");
    gone.kill();
    let (running, running_fds) = client.spawn("test", 1, b"running", true, &shell("sleep 600")).unwrap();
    let (counter, counter_fds) = client.spawn("test", 1, b"", false, &shell(COUNTER)).unwrap();
    client.signal(running.handle, libc::SIGSTOP, false).unwrap();
    status_event(&mut client, running.handle, |s| s == Status::Stopped(libc::SIGSTOP));
    let deadline = romacs_keeper::server::now_ms() + 3_600_000;
    client.set_deadline(running.handle, deadline).unwrap();
    for _ in 0..100 { client.spawn("bulk", 1, b"", false, &shell("sleep 600")).unwrap(); }
    wait_for("the unreported exit", || row(&mut client, unreported).unwrap().status == Status::Exited(5));

    let started = Instant::now();
    client.exec(Path::new(NEXT)).unwrap();
    println!("exec in place with 103 handles and reconnect: {:?}", started.elapsed());
    let after = client.info().unwrap();
    assert_eq!(after.pid, before.pid, "the same process");
    assert_ne!(after.executable, before.executable, "another program");
    assert_eq!(after.handles, 103);
    assert_eq!(row(&mut client, unreported).unwrap().status, Status::Exited(5), "a status reaped before the exec");
    let carried = row(&mut client, running.handle).unwrap();
    assert_eq!((carried.status, carried.deadline, carried.kept, carried.metadata.as_slice(), carried.owner, carried.owner_alive),
               (Status::Stopped(libc::SIGSTOP), deadline, true, &b"running"[..], std::process::id(), true));
    assert_eq!(parent_of(running.pid), before.pid, "its children are still its children");

    // The new image sees the old children's changes, and the session's
    // duplicates were never touched.
    client.signal(running.handle, libc::SIGCONT, false).unwrap();
    assert_eq!(status_event(&mut client, running.handle, |_| true), Status::Continued);
    assert_eq!(read_all(&counter_fds[0]), counter_output());
    assert_eq!(status_event(&mut client, counter.handle, Status::ended), Status::Exited(7));
    let (_, again) = client.claim(running.handle, 0).unwrap();
    assert_eq!(again.len(), running_fds.len());
    let (fresh, _) = client.spawn("test", 1, b"", false, &shell("sleep 600")).unwrap();
    assert!(fresh.handle > counter.handle, "handles keep counting up");

    // And back into the build with the older table: it refuses the newer
    // one in its check, and nothing changes.
    let refused = client.exec(Path::new(KEEPER)).unwrap_err();
    assert!(refused.contains("keeper table version 2 is newer than this build reads (1)"), "{refused}");
    assert_eq!(client.info().unwrap(), romacs_keeper::client::Info { handles: 104, ..after });
}

#[test]
fn an_exec_whose_new_binary_cannot_resume_falls_back() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let before = client.info().unwrap();
    let (row, _fds) = client.spawn("test", 1, b"", false, &shell("sleep 600")).unwrap();
    client.exec(Path::new(BROKEN)).unwrap();
    assert_eq!(client.info().unwrap(), romacs_keeper::client::Info { handles: 1, ..before }, "the old program again, same pid");
    assert_eq!(self::row(&mut client, row.handle).unwrap().status, Status::Running);
    assert_eq!(parent_of(row.pid), before.pid);
    let fds = |pid: u32| std::fs::read_dir(format!("/proc/{pid}/fd")).unwrap().count();
    let held = fds(before.pid);
    client.exec(Path::new(BROKEN)).unwrap();
    assert_eq!(fds(before.pid), held, "a fallback leaves no descriptor behind");
    let refused = client.exec(Path::new("/bin/false")).unwrap_err();
    assert!(refused.contains("/bin/false refuses this keeper's table"), "{refused}");
    assert!(client.list().is_ok(), "a refused exec changes nothing, the connection included");
    // A binary that passes the check and is gone at the exec: the keeper
    // goes on, and its sessions reconnect to it unchanged.
    let vanishing = keeper.dir.join("vanishing");
    std::fs::write(&vanishing, "#!/bin/sh\nrm -- \"$0\"\n").unwrap();
    std::fs::set_permissions(&vanishing, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    client.exec(&vanishing).unwrap();
    assert_eq!(client.info().unwrap(), romacs_keeper::client::Info { handles: 1, ..before });
    let empty = Spec { argv: Vec::new(), ..shell("") };
    assert!(client.spawn("test", 1, b"", false, &empty).unwrap_err().starts_with("spawn : "), "an empty argv is refused");
    assert!(client.list().is_ok());
}

/// A child that exits while its session has no connection is reported
/// when the session connects again, once.
#[test]
fn a_status_missed_without_a_connection_is_told_at_the_next() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let (row, _fds) = client.spawn("test", 1, b"", false, &shell("sleep 0.2; exit 7")).unwrap();
    drop(client);
    std::thread::sleep(Duration::from_millis(600));
    let mut client = keeper.client();
    assert_eq!(status_event(&mut client, row.handle, |_| true), Status::Exited(7));
    std::thread::sleep(Duration::from_millis(100));
    assert!(client.pump().unwrap().is_empty(), "once");
}

/// A helper is a child the session talks to directly, over a socketpair
/// whose other end the keeper keeps. The keeper's exec does not touch it:
/// the session's duplicate goes on working, and the helper is still the
/// keeper's child.
#[test]
fn an_exec_leaves_a_helper_and_its_conversation_alone() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let before = client.info().unwrap();
    let spec = Spec { fds: vec![(0, End::Null), (1, End::Null), (2, End::Null), (3, End::Socket)], ..shell("cat <&3 >&3; exit 6") };
    let (helper, fds) = client.spawn("helper", 1, b"", true, &spec).unwrap();
    let (plain, _plain_fds) = client.spawn("test", 1, b"", false, &shell("sleep 600")).unwrap();
    let mut talk = std::os::unix::net::UnixStream::from(fds.into_iter().next().unwrap());
    let say = |talk: &mut std::os::unix::net::UnixStream, line: &[u8]| {
        talk.write_all(line).unwrap();
        let mut back = vec![0; line.len()];
        talk.read_exact(&mut back).unwrap();
        back
    };
    assert_eq!(say(&mut talk, b"before\n"), b"before\n");
    client.exec(Path::new(NEXT)).unwrap();
    assert_eq!(client.info().unwrap().pid, before.pid);
    assert_eq!(say(&mut talk, b"after\n"), b"after\n", "the session's duplicate never noticed");
    assert_eq!((parent_of(helper.pid), parent_of(plain.pid)), (before.pid, before.pid));
    // End of file for the helper needs both holders of the socket to close.
    talk.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(status_event(&mut client, helper.handle, Status::ended), Status::Exited(6), "its exit reaches the new image");
    client.signal(plain.handle, libc::SIGKILL, false).unwrap();
    assert_eq!(status_event(&mut client, plain.handle, Status::ended), Status::Signaled(libc::SIGKILL));
}

/// Spawns sent into the exec are each answered by one keeper or the other:
/// none is lost and none is done twice.
#[test]
fn spawns_in_flight_across_the_exec_happen_once() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let mut other = keeper.session();
    writeln!(other.input, "burst 300").unwrap();
    wait_for("the burst to be under way", || client.info().unwrap().handles > 20);
    client.exec(Path::new(NEXT)).unwrap();
    let mid = client.info().unwrap().handles;
    assert_eq!(other.answer(), "300");
    let bursts = client.list().unwrap().iter().filter(|row| row.tag == "burst").count();
    assert_eq!(bursts, 300, "the exec came with {mid} spawned");
    assert!(mid < 300, "the exec fell inside the burst: {mid}");
}

/// A child that exits while the keeper execs is reported once, and a
/// spawn sent into the exec is answered by one keeper or the other.
#[test]
fn an_exit_during_the_exec_is_reported_once() {
    let keeper = Fixture::start(60_000);
    let mut client = keeper.client();
    let children: Vec<Row> = (0..20).map(|i| client.spawn("test", 1, b"", false, &shell(&format!("sleep 0.0{i}; exit 3"))).unwrap().0).collect();
    client.exec(Path::new(NEXT)).unwrap();
    let (late, _) = client.spawn("test", 1, b"", false, &shell("exit 4")).unwrap();
    let mut seen = std::collections::HashMap::new();
    wait_for("every exit", || {
        for event in client.pump().unwrap() {
            if let Event::Status(row) = event { *seen.entry(row.handle).or_insert(0) += 1; }
        }
        client.list().unwrap().iter().all(|row| row.status.ended())
    });
    std::thread::sleep(Duration::from_millis(200));
    for event in client.pump().unwrap() {
        if let Event::Status(row) = event { *seen.entry(row.handle).or_insert(0) += 1; }
    }
    for child in children.iter().chain([&late]) {
        assert!(seen.get(&child.handle).copied().unwrap_or(0) <= 1, "handle {} reported {:?} times", child.handle, seen.get(&child.handle));
    }
    assert_eq!(row(&mut client, late.handle).unwrap().status, Status::Exited(4));
}
