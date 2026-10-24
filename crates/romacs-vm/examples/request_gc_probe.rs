//! Direct-perf probe: full JSON-to-reply requests, or FIFO-gated collections.
//! Example: perf stat -vv -e instructions:u -- ... full tiny 400000
use romacs_vm::{Form, Vm};
use std::fs::{File, OpenOptions};
use std::hint::black_box;
use std::io::{Read, Write};

struct Control(File, File);
impl Control {
    fn command(&mut self, command: &[u8]) {
        self.0.write_all(command).unwrap();
        let mut ack = [0; 5];
        self.1.read_exact(&mut ack).unwrap();
        assert_eq!(&ack, b"ack\n\0");
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let mode = &args[1];
    assert!(matches!(
        mode.as_str(),
        "full" | "collect-full" | "collect-rerun"
    ));
    let shape = &args[2];
    let n: usize = args[3].parse().unwrap();
    let (text, answer) = match shape.as_str() {
        "tiny" => (r#"["+",1,2]"#, "3"),
        "alloc100" => (
            r#"["let",[["l","nil"],["i",0]],["while",["<","i",100],["setq","l",["cons","i","l"],"i",["1+","i"]]],["length","l"]]"#,
            "100",
        ),
        _ => panic!("unknown workload"),
    };
    let mut control = std::env::var("GC_CONTROL").ok().map(|path| {
        Control(
            OpenOptions::new().write(true).open(path).unwrap(),
            File::open(std::env::var("GC_ACK").unwrap()).unwrap(),
        )
    });
    assert_eq!(control.is_some(), mode != "full");
    let mut vm = Vm::new();
    vm.set_collection_floor(if mode == "full" { 32768 } else { usize::MAX });
    let prepared = if mode == "collect-rerun" {
        Some(
            vm.compile(&Form::from_json(serde_json::from_str(text).unwrap()).unwrap())
                .unwrap(),
        )
    } else {
        None
    };
    let mut digest = 0xcbf2_9ce4_8422_2325u64;
    let mut last = String::new();
    for _ in 0..n {
        let text = black_box(text.to_owned());
        let value = if let Some(id) = prepared {
            vm.execute(id).unwrap()
        } else {
            let form = Form::from_json(serde_json::from_str(&text).unwrap()).unwrap();
            vm.eval_form(&form).unwrap()
        };
        let out = value.display().unwrap();
        assert_eq!(out, answer);
        let reply = serde_json::json!({"value":out}).to_string();
        for &b in reply.as_bytes() {
            digest = (digest ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
        last = reply;
        if mode != "full" && vm.heap_objects() > 32768 {
            let control = control.as_mut().unwrap();
            control.command(b"enable\n");
            vm.collect_garbage();
            control.command(b"disable\n");
        }
    }
    let stats = vm.heap_stats();
    assert!(
        stats.collections > 0,
        "zero collections is not a GC measurement"
    );
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let rss = status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .unwrap();
    println!(
        "{}",
        serde_json::json!({"mode":mode,"shape":shape,"n":n,"last":last,"digest":digest,"collections":stats.collections,"allocations":stats.allocations,"reclaimed":stats.reclaimed,"live_objects":stats.live_objects,"rss":rss})
    );
}
