use romacs_vm::Vm;
use std::time::{SystemTime, UNIX_EPOCH};

// CLOCK_REALTIME, like GNU current_timespec, may be stepped by the OS. The
// ordering/bracketing checks assume no wall-clock adjustment during this test;
// they require no delay and do not claim current-time is a monotonic clock.

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("ERROR: {error}"),
    }
}

fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i128
}

#[test]
fn current_time_callable_shape_and_arity() {
    let mut vm = Vm::new();
    assert_eq!(
        eval(
            &mut vm,
            "(list (fboundp 'current-time) (functionp 'current-time))"
        ),
        "(t t)",
        "CURRENT_TIME_CALLABLE"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(let ((time (funcall 'current-time))) (list (integerp (car time)) (cdr time)))"
        ),
        "(t 1000000000)",
        "CURRENT_TIME_PAIR_SHAPE"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(condition-case e (apply 'current-time '(nil)) (error (car e)))"
        ),
        "wrong-number-of-arguments",
        "CURRENT_TIME_ARITY"
    );
}

#[test]
fn current_time_pair_is_exact_realtime_and_ordered() {
    let mut vm = Vm::new();
    let before = now_nanos();
    let first = eval(&mut vm, "(car (current-time))").parse::<i128>();
    let second = eval(&mut vm, "(car (current-time))").parse::<i128>();
    let after = now_nanos();
    assert!(
        matches!((first, second), (Ok(a), Ok(b)) if before <= a && a <= b && b <= after),
        "CURRENT_TIME_REALTIME_ORDER: expected two exact nanosecond readings within {before}..={after}"
    );
}

#[test]
fn current_time_list_fields_and_other_time_entry_points_agree() {
    let mut vm = Vm::new();
    let before = now_nanos();
    let text = eval(&mut vm, "(let ((current-time-list t)) (current-time))");
    let fields = text
        .trim_matches(['(', ')'])
        .split_whitespace()
        .map(str::parse::<i128>)
        .collect::<Result<Vec<_>, _>>();
    assert!(fields.is_ok(), "CURRENT_TIME_LIST_INTEGERS: {text}");
    let fields = fields.unwrap();
    assert_eq!(fields.len(), 4, "CURRENT_TIME_LIST_LENGTH");
    let (hi, lo, us, ps) = (fields[0], fields[1], fields[2], fields[3]);
    assert!(
        (0..65536).contains(&lo)
            && (0..1000000).contains(&us)
            && (0..1000000).contains(&ps)
            && ps % 1000 == 0,
        "CURRENT_TIME_LIST_RANGES: {text}"
    );
    let ticks = (hi * 65536 + lo) * 1000000000 + us * 1000 + ps / 1000;
    assert!(
        before <= ticks && ticks <= now_nanos(),
        "CURRENT_TIME_LIST_EXACT_REALTIME"
    );
    assert_eq!(
        eval(
            &mut vm,
            &format!("(set-visited-file-modtime '{text}) (visited-file-modtime)")
        ),
        format!("({ticks} . 1000000000)"),
        "CURRENT_TIME_LIST_MTIME_AGREEMENT"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(setq saved-current-time (current-time)) (set-visited-file-modtime saved-current-time) (equal saved-current-time (visited-file-modtime))"
        ),
        "t",
        "CURRENT_TIME_PAIR_MTIME_AGREEMENT"
    );
    assert_eq!(
        eval(&mut vm, "current-time-list"),
        "nil",
        "CURRENT_TIME_DYNAMIC_SETTING_RESTORED"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(let ((current-time-list 'nonboolean)) (length (funcall 'current-time)))"
        ),
        "4",
        "CURRENT_TIME_NONNIL_SELECTS_LIST"
    );
}
