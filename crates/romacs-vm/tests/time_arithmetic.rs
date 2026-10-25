use romacs_vm::Vm;
use serde_json::Value;

fn cases() -> Vec<Value> {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/time_arithmetic.json")).unwrap();
    fixture["cases"].as_array().unwrap().clone()
}

#[test]
fn time_arithmetic_matches_recorded_gnu() {
    for case in cases() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let source = case["source"].as_str().unwrap();
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(
            actual.as_deref(),
            Ok(case["expected"].as_str().unwrap()),
            "{}: {source}",
            case["name"]
        );
    }
}

#[test]
fn time_arithmetic_retains_bignum_results_across_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.eval_source(
        "(setq time-test-large (* 1000000000 (* 1000000000 1000000000)))
         (setq time-test-pair (time-convert time-test-large 1000000000))
         (setq time-test-list (time-convert time-test-pair 'list))",
    )
    .unwrap();
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    let actual = vm
        .eval_source(
            "(list (time-equal-p time-test-pair time-test-list)
          (time-equal-p (time-add time-test-pair '(-1 . 3))
            (time-subtract time-test-list '(1 . 3)))
          (time-less-p time-test-large (time-add time-test-pair 1)))",
        )
        .and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok("(t t t)"));
    assert!(vm.heap_stats().collections > before);
}

#[test]
fn time_convert_nil_reads_live_clock_in_each_output_form() {
    let nanos = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i128
    };
    for source in [
        "(time-convert nil t)",
        "(time-convert nil 1000000000)",
        "(let ((current-time-list nil)) (time-convert nil nil))",
        "(time-convert (time-convert nil 'list) 1000000000)",
        "(let ((current-time-list t)) (time-convert (time-convert nil nil) 1000000000))",
    ] {
        let mut vm = Vm::new();
        let before = nanos();
        let result = vm.eval_source(source).unwrap().display().unwrap();
        let after = nanos();
        let ticks = result
            .strip_prefix('(')
            .unwrap()
            .strip_suffix(" . 1000000000)")
            .unwrap()
            .parse::<i128>()
            .unwrap();
        assert!(
            before <= ticks && ticks <= after,
            "{source}: {before} <= {ticks} <= {after}; assumes no realtime clock step"
        );
    }
    let mut vm = Vm::new();
    let before = nanos().div_euclid(1_000_000_000);
    let result = vm
        .eval_source("(time-convert nil 'integer)")
        .unwrap()
        .display()
        .unwrap();
    let after = nanos().div_euclid(1_000_000_000);
    let seconds = result.parse::<i128>().unwrap();
    assert!(before <= seconds && seconds <= after);
}
