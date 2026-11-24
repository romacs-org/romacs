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
        "(retired (signal 9) (end 9) (hup) (settle owner-killed nil) (hook owner-killed nil nil))"
    );
    assert_eq!(
        step(&mut vm, ":role owner :phase retired :signal 15 :forked nil :child t", "(shutdown)"),
        "(nil (signal 9) (end 9) (hup))"
    );
    assert_eq!(step(&mut vm, ":role owner :phase dead :child t", "(shutdown)"), "nil");
    assert_eq!(step(&mut vm, ":role job :phase live :forked t :pending t :child t", "(shutdown)"), "(killed (kill-wait))");
}

/// I5: no event but an asked kill, end or shutdown signals a live owner;
/// a wedge, a deadline, a reply or a child's end never does.
#[test]
fn only_an_asked_end_signals_a_live_owner() {
    let mut vm = vm();
    for event in ["(retire)", "(deadline)", "(answered)", "(child-gone)", "(memory nil)", "(refused nil)"] {
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
    let roles = ["owner", "job", "zygote", "pool"];
    let phases = ["live", "retired", "killed", "dead"];
    let events = ["(kill)", "(end)", "(retire)", "(deadline)", "(answered)", "(memory v)", "(orphaned)", "(refused v)",
                  "(refuse r t)", "(death isolate-died d)", "(child-gone)", "(shutdown)"];
    for role in roles {
        for phase in phases {
            for forked in ["t", "nil"] {
                let state = format!(":role {role} :phase {phase} :signal nil :forked {forked} :pending t :child {}",
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
