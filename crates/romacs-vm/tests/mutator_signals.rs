//! A process's OS signals are its own mutator's (`Mutator::process_signals`):
//! a second VM made without them never takes a pending `sigusr1' from the
//! process's mailbox, and a `C-g' pressed through one VM's word quits that
//! VM alone. It raises a real signal, so it runs again in a child of its own.
#[path = "support/own_test.rs"]
mod own_test;
use romacs_vm::Vm;

const CHILD: &str = "ROMACS_MUTATOR_SIGNALS_CHILD";
const LOOP: &str = "(progn (dotimes (_ 100000)) 'ran)";

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap()
}

#[test]
fn a_second_mutator_leaves_the_process_signals_and_quits_by_its_own_word() {
    let own = own_test::name(module_path!(), "a_second_mutator_leaves_the_process_signals_and_quits_by_its_own_word");
    if std::env::var_os(CHILD).is_none() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &own, "--nocapture"]).env(CHILD, "1").output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && text.contains("1 passed"), "{text}");
        return;
    }
    romacs_vm::host::catch_user_signals().unwrap();
    let (mut first, mut second) = (Vm::new(), Vm::new());
    second.set_process_signals(false);
    rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::USR1).unwrap();
    // Any thread may run the handler: both reads come after it has, so the
    // second VM's nil is its gate, not a signal still in flight.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let sigusr1 = 1 << romacs_vm::host::USER_SIGNAL_NAMES.iter().position(|&name| name == "sigusr1").unwrap();
    while !romacs_vm::host::user_signal_waiting(sigusr1) {
        assert!(std::time::Instant::now() < deadline, "the signal reached the mailbox");
        std::thread::yield_now();
    }
    assert_eq!(eval(&mut second, "(romacs--take-user-signal)"), "nil", "a second mutator leaves the signal");
    assert_eq!(eval(&mut first, "(romacs--take-user-signal)"), "sigusr1", "the process's own mutator takes it");
    assert!(second.quit_signal().interrupt(), "a busy evaluator's word takes the press");
    assert_eq!(eval(&mut first, LOOP), "ran", "a press on another VM's word quits nothing here");
    let quit = second.eval_source(LOOP).and_then(romacs_vm::Local::display);
    assert_eq!(quit, Err("(quit)".into()), "its own word's press quits it");
}
