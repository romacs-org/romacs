use romacs_vm::Vm;

#[test]
fn readable_ir_is_reachable_for_source_submissions() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .args(["--source", "--dump-ir"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"(+ 1 2)").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "source IR request failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ir = String::from_utf8(output.stdout).unwrap();
    assert!(ir.starts_with("entry f0\n"));
    assert!(ir.contains("return r"));
}

#[test]
fn numeric_looking_symbols_remain_symbols_and_integer_dot_is_integer() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("'nan", "nan"),
        ("'inf", "inf"),
        ("'NaN", "NaN"),
        ("'infinity", "infinity"),
        ("(+ 1. 2.)", "3"),
    ] {
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(actual, Ok(expected.to_owned()), "{source}");
    }
}

#[test]
fn reader_values_survive_real_compaction() {
    let mut vm = Vm::new();
    vm.read_source("(unreachable [1 2 3])").unwrap();
    let (value, used) = vm
        .read_source("[λ (1 . 2) \"text\"] trailing")
        .unwrap()
        .unwrap();
    assert_eq!(used, "[λ (1 . 2) \"text\"]".len());
    let before = value.display().unwrap();
    let value = value.root();
    vm.collect_garbage();
    assert_eq!(value.get(&vm).unwrap().display().unwrap(), before);
    assert!(vm.heap_stats().reclaimed > 0);
}

#[test]
fn source_literals_survive_executor_collections() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let value = vm
        .eval_source(
            r#"
      (defun source-literal () '["keep" (λ . 9)])
      (let ((i 0))
        (while (< i 50000) (cons i nil) (setq i (1+ i))))
      (source-literal)
    "#,
        )
        .unwrap()
        .display();
    assert!(vm.heap_stats().collections > 0);
    assert_eq!(value.unwrap(), r#"["keep" (λ . 9)]"#);
}

#[test]
fn symbols_at_obarray_scale_have_stable_identity_and_constant_time_names() {
    let mut vm = Vm::new();
    for i in 0..100000 {
        vm.read_source(&format!("symbol-{i}")).unwrap();
    }
    let first = format!("{:?}", vm.read_source("symbol-99999").unwrap().unwrap().0);
    vm.collect_garbage();
    let second = vm.read_source("symbol-99999").unwrap().unwrap().0;
    assert_eq!(first, format!("{second:?}"));
    assert_eq!(second.display().unwrap(), "symbol-99999");
    assert_eq!(vm.read_source("nil").unwrap().unwrap().0, romacs_vm::Fixed::NIL);
    assert_eq!(vm.read_source("t").unwrap().unwrap().0, romacs_vm::Fixed::T);
}

#[test]
fn malformed_and_unsupported_source_is_not_silently_reinterpreted() {
    let mut vm = Vm::new();
    for text in [
        "(",
        ")",
        "[1)",
        "(a . b c)",
        "(. a)",
        "[a . b]",
        "\"oops",
        "'",
        "?ab",
        "?\\C_a",
        "#x1g",
    ] {
        assert!(vm.read_source(text).is_err(), "unexpectedly read {text}");
    }
    assert!(vm.read_source(&"(".repeat(1000)).is_err());
    assert!(vm.read_source("; only comments\n  ").unwrap().is_none());
    assert!(vm.eval_source("(+ 1 2) )").is_err());
    let value = vm.eval_source("(+ 2 3)").unwrap();
    assert_eq!(value.as_integer().unwrap(), 5);
}

#[test]
fn raw_source_cli_loads_a_real_el_file() {
    let source = include_str!("../examples/source-buffer.el");
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .arg("--source")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "2");
}
use std::io::{BufRead, BufReader, Write};

#[test]
fn source_stream_flushes_persists_and_recovers_without_eof() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .arg("--eval-stream")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    for (source, expected) in [
        ("(defun persistent (x) (+ x 2))", Some("persistent")),
        ("(persistent 5)", Some("7")),
        ("(", None),
        ("(persistent 8)", Some("10")),
    ] {
        writeln!(input, "{}", serde_json::json!({"source":source})).unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let reply: serde_json::Value = serde_json::from_str(&line).unwrap();
        if let Some(expected) = expected {
            assert_eq!(reply["value"], expected);
        } else {
            assert!(reply["error"].is_string());
        }
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn productivity_primitives_from_source() {
    let mut vm = Vm::new();
    let value = vm
        .eval_source(
            r#"(format "%s:%d:%S:%%" (car (process-lines "/usr/bin/printf" "hello\n")) 7 'ok)"#,
        )
        .unwrap();
    assert_eq!(value.display().unwrap(), r#""hello:7:ok:%""#);
}

use std::process::{Command, Stdio};

#[test]
fn source_stream_is_a_real_agent_boundary() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .arg("--eval-stream")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        serde_json::json!({"source":"(+ 1 2)"})
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["value"], "3");
}
