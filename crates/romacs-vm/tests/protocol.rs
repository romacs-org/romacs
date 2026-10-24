use romacs_vm::Vm;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
#[test]
fn typed_results_and_batches_preserve_errors_and_order() {
    let mut vm = Vm::new();
    let reply=vm.submit(&json!({"batch":[{"source":"(setq answer 7)"},{"source":"("},{"source":"(+ answer 2.5)"},{"source":"'[a 2]"}]})).unwrap();
    assert_eq!(
        reply["results"][0]["result"],
        json!({"type":"integer","value":7})
    );
    assert!(reply["results"][1]["error"].is_string());
    assert_eq!(reply["results"][2]["result"]["type"], "float");
    assert_eq!(reply["results"][2]["result"]["value"], 9.5);
    assert_eq!(
        reply["results"][3]["result"]["value"][0],
        json!({"type":"symbol","value":"a"})
    );
}
#[test]
fn identity_bearing_anchor_results_are_bare_opaque_tags() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(make-marker)", json!({"type":"marker"})),
        ("(copy-marker 1)", json!({"type":"marker"})),
        ("(make-overlay 1 1)", json!({"type":"overlay"})),
        ("(current-buffer)", json!({"type":"buffer"})),
    ] {
        let reply = vm.submit(&json!({"source":source})).unwrap();
        assert_eq!(reply["result"], expected, "{source}: {reply}");
    }
}
#[test]
fn real_stream_registers_named_calls_invokes_and_exposes_ir() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .arg("--eval-stream")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    fn send(input: &mut impl Write, output: &mut impl BufRead, value: Value) -> Value {
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    assert_eq!(
        send(
            &mut input,
            &mut output,
            json!({"source":"(defun named (x y) (+ x y))"})
        )["value"],
        "named"
    );
    let h = send(
        &mut input,
        &mut output,
        json!({"prepare":"named","arity":2}),
    )["handle"]
        .as_u64()
        .unwrap();
    assert_eq!(
        send(&mut input, &mut output, json!({"invoke":h,"args":[20,22]}))["result"],
        json!({"type":"integer","value":42})
    );
    send(
        &mut input,
        &mut output,
        json!({"source":"(defun named (x y) (- x y))"}),
    );
    assert_eq!(
        send(&mut input, &mut output, json!({"invoke":h,"args":[20,22]}))["result"]["value"],
        -2
    );
    assert!(
        send(&mut input, &mut output, json!({"ir_handle":h}))["ir"]
            .as_str()
            .unwrap()
            .contains("return r")
    );
    assert!(send(&mut input, &mut output, json!({"invoke":999999,"args":[]}))["error"].is_string());
    drop(input);
    assert!(child.wait().unwrap().success());
}
