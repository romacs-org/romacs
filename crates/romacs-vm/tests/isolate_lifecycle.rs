//! The isolate lifecycle's transition (lisp/vm/isolate-lifecycle.el) as a
//! pure function: the rows of the ownership lane's O1a-2 lifecycle table
//! that no real process can drive apart, each from a state built by hand.
//! The real paths are walked in `owner_lifecycle.rs`.
use romacs_vm::Vm;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm
}

/// `(romacs--isolate-transition STATE EVENT)` printed, STATE a plist.
fn step(vm: &mut Vm, state: &str, event: &str) -> String {
    vm.eval_source(&format!("(romacs--isolate-transition '({state}) '{event})"))
        .and_then(romacs_vm::Local::display)
        .unwrap_or_else(|e| format!("ERROR {e}"))
}

const OWNER: &str = ":role owner :phase live :signal nil :forked t :pending t :lingering nil :child t";

/// Row 11: SIGTERM at the first deadline, SIGKILL at the next, then
/// nothing: SIGKILL is the last there is, whatever a later deadline says.
#[test]
fn a_closing_owners_deadlines_go_15_then_9_then_nothing() {
    let mut vm = vm();
    let closing = |signal: &str| format!(":role owner :phase retired :signal {signal} :forked t :pending nil :child t");
    assert_eq!(step(&mut vm, &closing("nil"), "(deadline)"), "(nil (signal 15) (end 15))");
    assert_eq!(step(&mut vm, &closing("15"), "(deadline)"), "(nil (signal 9) (end 9))");
    assert_eq!(step(&mut vm, &closing("9"), "(deadline)"), "nil");
}

/// Z10 and row 12: a death is delivered once; a second one on a dead
/// isolate changes nothing and settles nothing.
#[test]
fn a_dead_isolate_hears_no_second_death() {
    let mut vm = vm();
    assert_eq!(step(&mut vm, ":role zygote :phase dead :signal nil :forked t", "(death isolate-died nil)"), "nil");
    assert_eq!(step(&mut vm, ":role owner :phase dead :signal nil :forked t :child t", "(death isolate-died nil)"), "nil");
}

/// Row 15 in starting₀: an owner its zygote never reported dies with it;
/// its calls settle (an owner's are never run elsewhere) and it is
/// reported.
#[test]
fn an_unforked_owners_death_settles_its_calls_and_is_reported() {
    let mut vm = vm();
    assert_eq!(
        step(&mut vm, ":role owner :phase live :signal nil :forked nil :pending t :child t", "(death isolate-died nil)"),
        "(dead (hup) (forget) (close) (zygote-gone) (settle isolate-died nil) (hook isolate-died nil t))"
    );
}

/// Z3's branch with no owner left at the ready frame: the forgotten
/// zygote retires, and with nothing to linger for its deadline runs; with
/// a child to linger for, none does.
#[test]
fn a_retired_zygote_has_a_deadline_only_once_nothing_lingers() {
    let mut vm = vm();
    assert_eq!(
        step(&mut vm, ":role zygote :phase live :signal nil :forked t :pending nil :lingering nil", "(end)"),
        "(retired (end nil) (post bye) (retire-children))"
    );
    let retired = |lingering: &str| format!(":role zygote :phase retired :signal nil :forked t :lingering {lingering}");
    assert_eq!(step(&mut vm, &retired("t"), "(deadline)"), "nil");
    assert_eq!(step(&mut vm, &retired("nil"), "(deadline)"), "(nil (signal 15) (end 15))");
    assert_eq!(step(&mut vm, &retired("t"), "(child-gone)"), "(nil (end nil))", "the clock restarts, still no deadline");
}

/// Rows 23 and 24: shutdown kills an owner without waiting, SIGKILL and
/// SIGHUP before any settlement or report, a closing one too; a dead one
/// is left alone; a forked job child is killed and waited for.
#[test]
fn shutdown_signals_before_it_settles_or_reports() {
    let mut vm = vm();
    assert_eq!(
        step(&mut vm, OWNER, "(shutdown)"),
        "(retired (forget-role) (signal 9) (end 9) (hup) (settle owner-killed nil) (hook owner-killed nil nil))"
    );
    assert_eq!(
        step(&mut vm, ":role owner :phase retired :signal 15 :forked nil :child t", "(shutdown)"),
        "(nil (forget-role) (signal 9) (end 9) (hup))"
    );
    // S21: every zygote, retired or live, through one path that waits for nothing.
    assert_eq!(step(&mut vm, ":role zygote :phase retired :signal nil :forked t :lingering t", "(shutdown)"),
               "(nil (forget-role) (signal 9) (end 9) (hup))");
    assert_eq!(step(&mut vm, ":role zygote :phase live :signal nil :forked t :pending t", "(shutdown)"),
               "(retired (forget-role) (signal 9) (end 9) (hup) (settle isolate-killed nil) (hook isolate-killed nil nil))");
    assert_eq!(step(&mut vm, ":role owner :phase dead :child t", "(shutdown)"), "nil");
    assert_eq!(step(&mut vm, ":role job :phase live :forked t :pending t :child t", "(shutdown)"), "(killed (kill-wait shutdown))");
    assert_eq!(step(&mut vm, ":role job :phase live :forked t :pending t :child t", "(kill)"), "(killed (kill-wait nil))");
}

/// I5: no event but an asked kill, end or shutdown signals a live owner;
/// a wedge, a deadline, a reply or a child's end never does.
#[test]
fn only_an_asked_end_signals_a_live_owner() {
    let mut vm = vm();
    for event in ["(retire)", "(deadline)", "(answered result 1 (42 nil))", "(spent)", "(child-gone)", "(memory nil)",
                  "(refused nil)"] {
        let out = step(&mut vm, OWNER, event);
        assert!(!out.contains("(signal"), "{event}: {out}");
    }
    assert_eq!(
        step(&mut vm, OWNER, "(kill)"),
        "(retired (signal 15) (end 15) (settle owner-killed nil) (hook owner-killed nil nil))"
    );
}

/// The edge's order (I6): in every row, what callbacks and hooks hear
/// (settle, resubmit, hook) comes after every effect that changes a
/// process or the lists.
#[test]
fn every_row_puts_what_callbacks_hear_last() {
    let mut vm = vm();
    let roles = ["owner", "job", "zygote", "pool", "fresh"];
    let phases = ["live", "retired", "killed", "dead"];
    let events = ["(kill)", "(end)", "(retire)", "(deadline)", "(answered result 1 (42 t))",
                  "(answered error 1 ((isolate-retired nil nil nil) nil))", "(spent)", "(memory v)", "(orphaned)",
                  "(orphaned shutdown)", "(refused v)", "(refuse r t)", "(hello-refused d)", "(world-refused r w)",
                  "(death isolate-died d)", "(death isolate-killed d shutdown)", "(child-gone)", "(shutdown)"];
    for role in roles {
        for phase in phases {
            for forked in ["t", "nil"] {
                let state = format!(":role {role} :phase {phase} :signal nil :forked {forked} :pending t :spent t :child {}",
                                    if matches!(role, "owner" | "job") { "t" } else { "nil" });
                for event in events {
                    let order = vm.eval_source(&format!(
                        "(let ((seen-late nil) (bad nil))
                           (dolist (effect (cdr (romacs--isolate-transition '({state}) '{event})) bad)
                             (if (memq (car effect) '(settle resubmit hook))
                                 (setq seen-late t)
                               (when seen-late (setq bad effect)))))"))
                        .and_then(romacs_vm::Local::display).unwrap();
                    assert_eq!(order, "nil", "{state} {event}");
                }
            }
        }
    }
}

/// S14: under shutdown, a dead zygote's busy children settle as killed
/// and none runs elsewhere; an ordinary zygote death sends an unforked
/// one's job elsewhere.
#[test]
fn a_zygote_killed_at_shutdown_runs_no_job_elsewhere() {
    let mut vm = vm();
    let zygote = ":role zygote :phase killed :signal nil :forked t :pending t";
    assert_eq!(step(&mut vm, zygote, "(death isolate-killed nil shutdown)"),
               "(nil (hup) (forget) (close) (orphan-children shutdown) (settle isolate-killed nil) (hook isolate-killed nil t))");
    let unforked = ":role job :phase live :signal nil :forked nil :pending t :child t";
    assert_eq!(step(&mut vm, unforked, "(death isolate-killed nil shutdown)"),
               "(dead (hup) (forget) (close) (zygote-gone) (settle isolate-killed nil) (hook isolate-killed nil t))");
    assert_eq!(step(&mut vm, unforked, "(death isolate-died nil)"),
               "(dead (hup) (forget) (close) (zygote-gone) (resubmit) (hook isolate-died nil t))");
    assert_eq!(step(&mut vm, ":role job :phase live", "(orphaned shutdown)"),
               "(retired (signal 9) (end nil) (settle isolate-killed nil) (hook isolate-killed nil nil))");
}

/// S19: a reply resolves its call and begins a spent worker's end in one
/// transition, its callbacks last; an owner's `isolate-retired` is its
/// call's error, another worker's runs the call elsewhere.
#[test]
fn a_reply_ends_a_spent_worker_before_its_callbacks() {
    let mut vm = vm();
    assert_eq!(step(&mut vm, ":role pool :phase live :pending t :spent t", "(answered result 7 (42 nil))"),
               "(nil (reply 7 result 42) (spent-check))");
    assert_eq!(step(&mut vm, ":role pool :phase live :pending nil :spent t", "(spent)"),
               "(retired (end nil) (post bye))");
    assert_eq!(step(&mut vm, ":role job :phase live :pending t :child t", "(answered result 7 (42 t))"),
               "(retired (end nil) (zygote-gone) (taint) (reply 7 result 42))");
    let retired = "(answered error 7 ((isolate-retired nil nil nil) nil))";
    assert_eq!(step(&mut vm, ":role owner :phase live :pending t :child t", retired),
               "(nil (reply 7 error (isolate-retired nil nil nil)))");
    assert_eq!(step(&mut vm, ":role nil :phase live :pending t", retired),
               "(nil (reply 7 requeue (isolate-retired nil nil nil)))");
}

/// S16: a refused hello kills its worker without waiting; its death comes
/// later as any death; a world refused is forgotten first.
#[test]
fn a_refused_hello_waits_for_nothing() {
    let mut vm = vm();
    assert_eq!(step(&mut vm, OWNER, "(hello-refused (why text))"),
               "(retired (forget-role) (signal 9) (end 9) (settle isolate-refused (why text)) (hook isolate-refused (why text) nil))");
    assert_eq!(step(&mut vm, ":role pool :phase live", "(world-refused why w)"),
               "(retired (forget-world why w) (forget-role) (signal 9) (end 9) (settle isolate-world-unavailable (why w)) (hook isolate-world-unavailable (why w) nil))");
    assert_eq!(step(&mut vm, ":role pool :phase retired", "(hello-refused (why text))"), "nil");
    // A zygote of another build fails its queued jobs: any other would be refused too.
    assert_eq!(step(&mut vm, ":role zygote :phase live", "(hello-refused (why text))"),
               "(retired (forget-role) (signal 9) (end 9) (orphan-children (refused why text)) (settle isolate-refused (why text)) (hook isolate-refused (why text) nil))");
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap_or_else(|e| format!("ERROR {e}"))
}

/// P1/P2 and S18: an owner, its zygote counted, ends no worker for room.
#[test]
fn admission_decides_and_an_owner_never_evicts() {
    let mut vm = vm();
    let decide = |vm: &mut Vm, args: &str| eval(vm, &format!("(romacs--isolate-admission {args})"));
    assert_eq!(decide(&mut vm, "'owner 1 2 3 nil nil"), "admit");
    assert_eq!(decide(&mut vm, "'owner 2 2 3 nil nil"), "refuse");
    assert_eq!(decide(&mut vm, "'owner 2 2 3 t nil"), "refuse");
    assert_eq!(decide(&mut vm, "'start 3 1 3 t nil"), "wait");
    assert_eq!(decide(&mut vm, "'start 3 1 3 nil nil"), "evict");
    assert_eq!(decide(&mut vm, "'start 2 1 3 t nil"), "wait", "room coming, memory short");
    assert_eq!(decide(&mut vm, "'start 2 1 3 t t"), "admit");
}

/// §7 and S20: a round takes its calls at its head only; after a wait it
/// only notes what came.
#[test]
fn a_round_takes_calls_once_then_output_before_the_next() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(romacs--isolate-round-plan '(:zygote nil))"), "(interrupts forks calls turns note)");
    assert_eq!(eval(&mut vm, "(romacs--isolate-round-plan '(:zygote t))"), "(interrupts forks calls note)");
    assert_eq!(eval(&mut vm, "(romacs--isolate-wait-plan '(:serving t :queued nil :turned t))"), "(wait note)");
    assert_eq!(eval(&mut vm, "(romacs--isolate-wait-plan '(:serving t :queued nil :turned nil))"), "nil",
               "S23: no wait in a worker round that ran no output");
    assert_eq!(eval(&mut vm, "(romacs--isolate-wait-plan '(:serving t :zygote t))"), "(wait note)");
    assert_eq!(eval(&mut vm, "(romacs--isolate-wait-plan '(:serving t :queued t))"), "nil");
    assert_eq!(eval(&mut vm, "(romacs--isolate-wait-plan '(:serving t :zygote t :forks t))"), "nil");
    assert_eq!(eval(&mut vm, "(romacs--isolate-wait-plan '(:serving nil))"), "nil");
}

/// §8, S12 and S15: a bounded pass keeps every status not yet notified
/// pending whatever callback leaves it; the master keeps GNU's loss.
#[test]
fn a_bounded_status_pass_loses_no_status_to_an_exit() {
    let mut vm = vm();
    let row = |vm: &mut Vm, args: &str| eval(vm, &format!("(romacs--process-status-row {args})"));
    assert_eq!(row(&mut vm, "'draining 'spent t nil t"), "((re-tick))");
    assert_eq!(row(&mut vm, "'draining 'exit t nil t"), "((re-tick))");
    assert_eq!(row(&mut vm, "'draining 'exit t nil nil"), "nil", "S24: a deleted process leaves nothing pending");
    assert_eq!(row(&mut vm, "'draining 'exit t t nil"), "((raise))");
    assert_eq!(row(&mut vm, "'sentinel 'exit t t t"), "((raise))", "the finished one stays acknowledged");
    assert_eq!(row(&mut vm, "'sentinel 'exit t nil t"), "nil", "nothing left: nothing pending");
    assert_eq!(row(&mut vm, "'between 'exit t t nil"), "((raise))");
    assert_eq!(row(&mut vm, "'draining 'exit nil t t"), "nil");
    assert_eq!(row(&mut vm, "'sentinel 'exit nil t t"), "nil");
}
