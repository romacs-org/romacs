//! O7m C4 tests requiring a real image with pre-existing, unwritten slots.

#[path = "o7m_written.rs"]
mod bits;

use super::{ROLE, Vm, child_of, scratch};
use bits::{assert_bit, check_write, observed, written};
use romacs_vm::world::store::{Launch, World};
use std::path::PathBuf;

fn restored_case(name: &str, initial: &str, test: impl FnOnce(&mut Vm)) {
    if std::env::var(ROLE).as_deref() == Ok("written-bit") {
        let store = PathBuf::from(std::env::var_os("ROMACS_WORLD_IMAGE_TEST_STORE").unwrap());
        let world = World::new(store, vec![format!("consumer written-bit {name}")]).unwrap();
        match world.launch::<u64>(|| std::env::args_os().skip(1).collect(), true, |_| {}) {
            Launch::Build => {
                world.begin_build().unwrap();
                world
                    .build(|| {
                        let mut vm = Vm::new();
                        vm.world_begin();
                        vm.eval_source(initial)?;
                        Ok((vm, 42_u64, serde_json::json!({})))
                    })
                    .unwrap();
                std::process::exit(0);
            }
            Launch::Restored { mut vm, carry, .. } => {
                assert_eq!(carry, 42);
                vm.world_reinit();
                println!("WRITTEN-BIT RESTORED");
                test(&mut vm);
            }
            Launch::Chain { reason } => panic!("fixture must restore its image: {reason}"),
        }
    } else {
        let directory = scratch(name);
        let output = child_of(name, "written-bit", &directory.join("worlds"))
            .output()
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains("WRITTEN-BIT RESTORED"),
            "{stdout}\n{stderr}"
        );
        assert!(output.status.success(), "{stdout}\n{stderr}");
    }
}

macro_rules! restored_case {
    ($name:ident, $initial:literal, $test:expr) => {
        #[test]
        fn $name() {
            restored_case(
                concat!("written_bits::", stringify!($name)),
                $initial,
                $test,
            );
        }
    };
}

restored_case!(
    untouched_image_symbol_is_clear,
    "(defvar o7m-bit-probe 42)",
    |vm| {
        assert_eq!(observed(vm, "o7m-bit-probe").as_deref(), Ok("42"));
        assert_bit(
            &written(vm, "o7m-bit-probe"),
            "nil",
            "after actual image restore",
        );
    }
);

restored_case!(
    fmakunbound_sets_written,
    "(defun o7m-bit-probe () 42)",
    |vm| {
        assert_eq!(observed(vm, "(o7m-bit-probe)").as_deref(), Ok("42"));
        check_write(
            vm,
            "o7m-bit-probe",
            "(fmakunbound 'o7m-bit-probe)",
            "(fboundp 'o7m-bit-probe)",
            "nil",
        );
    }
);

restored_case!(makunbound_sets_written, "(defvar o7m-bit-probe 42)", |vm| {
    assert_eq!(observed(vm, "o7m-bit-probe").as_deref(), Ok("42"));
    check_write(
        vm,
        "o7m-bit-probe",
        "(makunbound 'o7m-bit-probe)",
        "(boundp 'o7m-bit-probe)",
        "nil",
    );
});

restored_case!(
    let_of_special_stays_written_after_unwind,
    "(defvar o7m-bit-probe 42)",
    |vm| {
        let before = written(vm, "o7m-bit-probe");
        let during = observed(
            vm,
            "(let ((o7m-bit-probe 73))
                       (list (symbol-value 'o7m-bit-probe)
                             (condition-case err (romacs--written-p 'o7m-bit-probe)
                               (void-function err))))",
        );
        assert_eq!(observed(vm, "o7m-bit-probe").as_deref(), Ok("42"));
        assert_eq!(
            (
                before.as_deref(),
                during.as_deref(),
                written(vm, "o7m-bit-probe").as_deref()
            ),
            (Ok("nil"), Ok("(73 t)"), Ok("t")),
            "a special binding must mark the slot and keep the bit after unwinding"
        );
    }
);

restored_case!(
    put_replace_sets_written,
    "(setplist 'o7m-bit-probe '(o7m-key 1))",
    |vm| {
        check_write(
            vm,
            "o7m-bit-probe",
            "(put 'o7m-bit-probe 'o7m-key 42)",
            "(symbol-plist 'o7m-bit-probe)",
            "(o7m-key 42)",
        );
    }
);

restored_case!(
    put_append_sets_written,
    "(setplist 'o7m-bit-probe '(o7m-key 1))",
    |vm| {
        check_write(
            vm,
            "o7m-bit-probe",
            "(put 'o7m-bit-probe 'o7m-other 42)",
            "(symbol-plist 'o7m-bit-probe)",
            "(o7m-key 1 o7m-other 42)",
        );
    }
);

restored_case!(
    failed_put_keeps_bit_clear,
    "(setplist 'o7m-bit-probe '(old 1 . improper))",
    |vm| {
        let before = written(vm, "o7m-bit-probe");
        assert_eq!(
            observed(
                vm,
                "(condition-case err (put 'o7m-bit-probe 'new 42)
                         (wrong-type-argument (nth 1 err)))"
            )
            .as_deref(),
            Ok("plistp")
        );
        assert_eq!(
            observed(vm, "(symbol-plist 'o7m-bit-probe)").as_deref(),
            Ok("(old 1 . improper)")
        );
        assert_eq!(
            (before.as_deref(), written(vm, "o7m-bit-probe").as_deref()),
            (Ok("nil"), Ok("nil")),
            "a put that signals before storing must leave the bit clear"
        );
    }
);
