//! Streaming corpus probe. Each JSON line is a UTF-8 source string. No GNU
//! conversion, compiler, evaluator, or inherited reader participates.
use romacs_vm::Vm;
use std::io::{BufRead, Write};
fn main() {
    let mut vm = Vm::new();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let source: String = serde_json::from_str(&line).unwrap();
        let mut offset = 0;
        let mut count = 0;
        let mut error = None;
        loop {
            match vm.read_source(&source[offset..]) {
                Ok(Some((_, used))) => {
                    offset += used;
                    count += 1;
                }
                Ok(None) => {
                    offset = source.len();
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
            vm.collect_garbage();
        }
        vm.collect_garbage();
        writeln!(
            out,
            "{}",
            serde_json::json!({"forms":count,"bytes":offset,"error":error})
        )
        .unwrap();
        out.flush().unwrap();
    }
}
