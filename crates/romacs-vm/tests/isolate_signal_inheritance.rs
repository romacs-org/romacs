#[path = "support/own_test.rs"]
mod own_test;

use romacs_vm::host;
use rustix::process::{Pid, Signal, WaitOptions, getpid, kill_process, waitpid};
use std::process::Command;
use std::time::{Duration, Instant};

const REEXEC: &str = "ROMACS_FORK_USER_SIGNAL_TEST";
const LIMIT: Duration = Duration::from_secs(5);
const ARRIVAL: i32 = 1;
const SIGUSR2_EVENT: i32 = 2;
const OTHER_EVENT: i32 = 4;
const RESET_FAILED: i32 = 8;

#[test]
fn forked_worker_starts_without_its_zygotes_user_signal() {
    if std::env::var_os(REEXEC).is_some() {
        fork_with_pending_user_signal();
        return;
    }

    let test = own_test::name(
        module_path!(),
        "forked_worker_starts_without_its_zygotes_user_signal",
    );
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &test, "--nocapture"])
        .env(REEXEC, "1")
        .status()
        .unwrap();
    assert!(status.success(), "{test} in its own process: {status}");
}

fn fork_with_pending_user_signal() {
    host::catch_user_signals().unwrap();
    assert!(!host::safepoint_pending());
    assert!(!host::user_signal_waiting(1));
    kill_process(getpid(), Signal::USR2).unwrap();

    let start = Instant::now();
    while !host::user_signal_waiting(1) || !host::safepoint_pending() {
        assert!(start.elapsed() < LIMIT, "SIGUSR2 did not reach the zygote");
        std::thread::yield_now();
    }

    let zygote = getpid().as_raw_nonzero().get();
    let pid = host::fork_isolate().unwrap();
    if pid == 0 {
        report_child_signal_state(zygote);
    }

    let code = reap_before_deadline(Pid::from_raw(pid).unwrap());
    assert!(
        host::safepoint_pending(),
        "the zygote still owns its arrival"
    );
    assert_eq!(host::take_user_signal(), Some("sigusr2"));
    assert_ne!(code, RESET_FAILED, "forked_child_reset failed");
    assert_eq!(
        (
            code & ARRIVAL != 0,
            code & SIGUSR2_EVENT != 0,
            code & OTHER_EVENT != 0
        ),
        (false, false, false),
        "forked worker inherited (pending arrival, sigusr2 event, other event)"
    );
}

fn report_child_signal_state(zygote: i32) -> ! {
    // This is the native fork/reset used by both jobs and owners. Sample
    // before any Lisp safepoint has a chance to clear the arrival flag.
    if host::forked_child_reset(zygote).is_err() {
        host::exit_at_once(RESET_FAILED);
    }
    let arrival = if host::safepoint_pending() {
        ARRIVAL
    } else {
        0
    };
    let event = match host::take_user_signal() {
        None => 0,
        Some("sigusr2") => SIGUSR2_EVENT,
        Some(_) => OTHER_EVENT,
    };
    host::exit_at_once(arrival | event);
}

fn reap_before_deadline(pid: Pid) -> i32 {
    let start = Instant::now();
    loop {
        if let Some((_, status)) = waitpid(Some(pid), WaitOptions::NOHANG).unwrap() {
            return status.exit_status().expect("forked worker exited normally");
        }
        if start.elapsed() >= LIMIT {
            let _ = kill_process(pid, Signal::KILL);
            panic!("forked worker did not report its signal state before the deadline");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

const CHILD_SAW_TERM: i32 = 16;
const CHILD_LOST_TERM: i32 = 32;
const SEND_TERM_FAILED: i32 = 64;

#[test]
fn sigterm_sent_before_forked_child_reset_still_ends_child() {
    const TEST: &str = "sigterm_sent_before_forked_child_reset_still_ends_child";
    if std::env::var_os(REEXEC).is_some() {
        fork_with_child_termination();
        return;
    }

    let test = own_test::name(module_path!(), TEST);
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &test, "--nocapture"])
        .env(REEXEC, "1")
        .status()
        .unwrap();
    assert!(status.success(), "{test} in its own process: {status}");
}

fn fork_with_child_termination() {
    host::catch_isolate_signals().unwrap();
    assert!(!host::terminate_pending());
    let zygote = getpid().as_raw_nonzero().get();
    let pid = host::fork_isolate().unwrap();
    if pid == 0 {
        terminate_before_reset(zygote);
    }
    let code = reap_before_deadline(Pid::from_raw(pid).unwrap());
    assert!(
        !host::terminate_pending(),
        "only the child received SIGTERM"
    );
    assert_ne!(code, SEND_TERM_FAILED, "the child could not send SIGTERM");
    assert_ne!(code, RESET_FAILED, "forked_child_reset failed");
    assert_eq!(
        code, CHILD_SAW_TERM,
        "fork reset lost the child's own SIGTERM"
    );
}

fn terminate_before_reset(zygote: i32) -> ! {
    // This is the native fork/reset pair. A self-directed SIGTERM is
    // delivered before kill returns if unmasked, or pending if masked.
    // The worker's run loop exits on this same termination flag.
    if kill_process(getpid(), Signal::TERM).is_err() {
        host::exit_at_once(SEND_TERM_FAILED);
    }
    if host::forked_child_reset(zygote).is_err() {
        host::exit_at_once(RESET_FAILED);
    }
    host::exit_at_once(if host::terminate_pending() {
        CHILD_SAW_TERM
    } else {
        CHILD_LOST_TERM
    });
}
