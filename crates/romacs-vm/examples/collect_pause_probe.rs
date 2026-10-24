//! Full-collection pause against retained heap size, for the D1 receipt.
//!
//! Retains N conses in a declared special, then times five explicit
//! `collect_garbage` calls on that heap. The empty-heap row isolates the fixed root
//! cost (every symbol slot is a root). Output is one JSON line per size so
//! the numbers can be quoted beside load.
//!
//! Example: collect_pause_probe 0 250000 1000000 4000000
use romacs_vm::Vm;
use std::time::Instant;

fn load() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join("/"))
        .unwrap_or_default()
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().expect("size"))
        .collect();
    assert!(!sizes.is_empty(), "usage: collect_pause_probe SIZE...");
    for &n in &sizes {
        let mut vm = Vm::new();
        vm.eval_source("(defvar pause-probe-big nil)").unwrap();
        let build = format!(
            "(setq pause-probe-big (let ((l nil) (i 0)) (while (< i {n}) (setq l (cons i l) i (1+ i))) l))"
        );
        vm.eval_source(&build).unwrap();
        let answer = vm.eval_source("(length pause-probe-big)").unwrap();
        assert_eq!(answer.display().unwrap(), n.to_string());
        let load_before = load();
        let mut pauses_ns = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            vm.collect_garbage();
            pauses_ns.push(t.elapsed().as_nanos() as u64);
        }
        let stats = vm.heap_stats();
        let mut sorted = pauses_ns.clone();
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        let per_object = if stats.live_objects > 0 {
            median as f64 / stats.live_objects as f64
        } else {
            0.0
        };
        println!(
            "{{\"retained_conses\":{n},\"live_objects\":{},\"live_bytes\":{},\"capacity_bytes\":{},\"collections\":{},\"pauses_ns\":{:?},\"median_ns\":{median},\"min_ns\":{},\"max_ns\":{},\"ns_per_live_object\":{per_object:.2},\"load_before\":\"{load_before}\",\"load_after\":\"{}\"}}",
            stats.live_objects,
            stats.live_bytes,
            stats.capacity_bytes,
            stats.collections,
            pauses_ns,
            sorted[0],
            sorted[sorted.len() - 1],
            load()
        );
    }
}
