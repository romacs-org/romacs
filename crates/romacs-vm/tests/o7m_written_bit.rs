//! O7m C4 write-site evidence using the public Lisp primitive.

#[path = "support/o7m_written.rs"]
mod bits;

use bits::{assert_bit, check_write, observed, written};
use romacs_vm::Vm;

macro_rules! write_case {
    ($name:ident, $form:literal, $probe:literal, $expected:literal) => {
        #[test]
        fn $name() {
            check_write(&mut Vm::new(), "o7m-bit-probe", $form, $probe, $expected);
        }
    };
}

write_case!(
    fset_sets_written,
    "(fset 'o7m-bit-probe (lambda () 42))",
    "(o7m-bit-probe)",
    "42"
);
write_case!(
    defalias_sets_written,
    "(defalias 'o7m-bit-probe 'car)",
    "(symbol-function 'o7m-bit-probe)",
    "car"
);
write_case!(
    defun_sets_written,
    "(defun o7m-bit-probe () 42)",
    "(o7m-bit-probe)",
    "42"
);
write_case!(
    defmacro_sets_written,
    "(defmacro o7m-bit-probe () 42)",
    "(o7m-bit-probe)",
    "42"
);
write_case!(
    autoload_sets_written,
    "(autoload 'o7m-bit-probe \"o7m-unloaded-bit-fixture\")",
    "(car (symbol-function 'o7m-bit-probe))",
    "autoload"
);
write_case!(
    set_sets_written,
    "(set 'o7m-bit-probe 42)",
    "o7m-bit-probe",
    "42"
);
write_case!(
    setq_sets_written,
    "(setq o7m-bit-probe 42)",
    "o7m-bit-probe",
    "42"
);
write_case!(
    set_default_sets_written,
    "(set-default 'o7m-bit-probe 42)",
    "(default-value 'o7m-bit-probe)",
    "42"
);
write_case!(
    defvar_sets_written,
    "(defvar o7m-bit-probe 42)",
    "o7m-bit-probe",
    "42"
);
write_case!(
    first_put_sets_written,
    "(put 'o7m-bit-probe 'o7m-key 42)",
    "(symbol-plist 'o7m-bit-probe)",
    "(o7m-key 42)"
);
write_case!(
    setplist_sets_written,
    "(setplist 'o7m-bit-probe '(o7m-key 42))",
    "(symbol-plist 'o7m-bit-probe)",
    "(o7m-key 42)"
);

// The condition projection lives outside symbol-plist.
write_case!(
    put_error_conditions_sets_written,
    "(put 'o7m-bit-probe 'error-conditions '(o7m-bit-probe error))",
    "(list (get 'o7m-bit-probe 'error-conditions) (symbol-plist 'o7m-bit-probe))",
    "((o7m-bit-probe error) nil)"
);
write_case!(
    define_error_sets_written,
    "(define-error 'o7m-bit-probe \"fixture condition\")",
    "(list (get 'o7m-bit-probe 'error-conditions) (symbol-plist 'o7m-bit-probe))",
    "((o7m-bit-probe error) nil)"
);

#[test]
fn defvaralias_sets_written_on_both_symbols() {
    let mut vm = Vm::new();
    let before = |vm: &mut Vm| [written(vm, "o7m-bit-alias"), written(vm, "o7m-bit-base")];
    let initial = before(&mut vm);
    vm.eval_source("(defvaralias 'o7m-bit-alias 'o7m-bit-base)")
        .unwrap();
    assert_eq!(
        observed(&mut vm, "(indirect-variable 'o7m-bit-alias)").as_deref(),
        Ok("o7m-bit-base")
    );
    let final_bits = before(&mut vm);
    assert_eq!(
        (
            initial.each_ref().map(Result::as_deref),
            final_bits.each_ref().map(Result::as_deref)
        ),
        ([Ok("nil"); 2], [Ok("t"); 2]),
        "defvaralias must mark the alias and the base"
    );
}

#[test]
fn lazy_translation_table_plist_sets_written() {
    let mut vm = Vm::new();
    assert_eq!(
        observed(&mut vm, "(intern-soft \"translation-table\")").as_deref(),
        Ok("nil")
    );
    assert_eq!(
        observed(&mut vm, "(symbol-plist (intern \"translation-table\"))").as_deref(),
        Ok("(char-table-extra-slots 2)")
    );
    assert_bit(
        &written(&mut vm, "translation-table"),
        "t",
        "after first intern",
    );
}

#[test]
fn lazy_syntax_table_plist_sets_written() {
    let mut vm = Vm::new();
    assert_eq!(
        observed(&mut vm, "(symbol-plist 'syntax-table)").as_deref(),
        Ok("nil")
    );
    check_write(
        &mut vm,
        "syntax-table",
        "(standard-syntax-table)",
        "(symbol-plist 'syntax-table)",
        "(char-table-extra-slots 0)",
    );
}

#[test]
fn lazy_builtin_function_cell_sets_written() {
    let mut vm = Vm::new();
    assert_eq!(
        observed(&mut vm, "(intern-soft \"kill-region\")").as_deref(),
        Ok("nil")
    );
    assert_eq!(
        observed(&mut vm, "(fboundp (intern \"kill-region\"))").as_deref(),
        Ok("t")
    );
    assert_bit(
        &written(&mut vm, "kill-region"),
        "t",
        "after lazy function install",
    );
}

#[test]
fn load_sets_written() {
    let mut vm = Vm::new();
    let initial = written(&mut vm, "o7m-bit-load");
    let file = std::env::temp_dir().join(format!("o7m-written-load-{}.el", std::process::id()));
    std::fs::write(&file, "(setq o7m-bit-load 42)").unwrap();
    let load = vm.load_file(&file).map(drop);
    std::fs::remove_file(file).unwrap();
    load.unwrap();
    assert_eq!(observed(&mut vm, "o7m-bit-load").as_deref(), Ok("42"));
    assert_eq!(
        (
            initial.as_deref(),
            written(&mut vm, "o7m-bit-load").as_deref()
        ),
        (Ok("nil"), Ok("t"))
    );
}

#[test]
fn collection_does_not_clear_written() {
    let mut vm = Vm::new();
    vm.eval_source("(setq o7m-bit-probe 42)").unwrap();
    let before = written(&mut vm, "o7m-bit-probe");
    vm.collect_garbage();
    assert_eq!(observed(&mut vm, "o7m-bit-probe").as_deref(), Ok("42"));
    assert_eq!(
        (
            before.as_deref(),
            written(&mut vm, "o7m-bit-probe").as_deref()
        ),
        (Ok("t"), Ok("t"))
    );
}
