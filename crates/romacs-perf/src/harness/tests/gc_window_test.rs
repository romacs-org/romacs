//! Request-owned GC sidecar environment contract. These tests do not mutate
//! the process environment, launch editors, or run a performance workload.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::PathBuf;

use crate::harness::{RunRequest, passthrough_from};
use crate::{Frontend, ScenarioId};

#[test]
fn benchmark_environment_forwards_exact_acknowledged_gc_window_path() {
    let path = OsString::from("/owned/run A/actual-gc-window.json");
    let forwarded: BTreeMap<_, _> = passthrough_from([
        (OsString::from("ROMACS_PERF_GC_WINDOW_FILE"), path.clone()),
        (
            OsString::from("ROMACS_PERF_GC_WINDOW_FILE_BACKUP"),
            OsString::from("ignored"),
        ),
        (
            OsString::from("ROMACS_PERF_GATE_PORT"),
            OsString::from("discard-inherited-port"),
        ),
        (OsString::from("RUST_LOG"), OsString::from("debug")),
    ])
    .into_iter()
    .collect();

    assert_eq!(
        forwarded,
        BTreeMap::from([("ROMACS_PERF_GC_WINDOW_FILE".to_owned(), path)]),
        "the exact sidecar path must reach the editor; the gate port remains harness-owned"
    );
}

#[test]
fn benchmark_environment_without_gc_window_preserves_existing_controls() {
    let variables = [
        ("PATH", "/bin"),
        ("ROMACS_REGEX_CACHE_DIAGNOSTIC", "/owned/profile.csv"),
        ("ROMACS_PERF_LATENCY_TRACE_FILE", "/owned/latency.txt"),
    ];
    let forwarded: BTreeMap<_, _> = passthrough_from(
        variables.map(|(name, value)| (OsString::from(name), OsString::from(value))),
    )
    .into_iter()
    .collect();

    assert_eq!(
        forwarded,
        BTreeMap::from(variables.map(|(name, value)| (name.to_owned(), OsString::from(value))))
    );
    assert!(!forwarded.contains_key("ROMACS_PERF_GC_WINDOW_FILE"));
}

#[test]
fn acknowledged_gc_window_path_is_captured_per_request_and_arm() {
    let request = |name: &str| {
        let environment = passthrough_from([
            (
                OsString::from("ROMACS_PERF_GC_WINDOW_FILE"),
                OsString::from(name),
            ),
            (OsString::from("ROMACS_REGEX_DFA"), OsString::from("off")),
        ])
        .into_iter()
        .collect();
        RunRequest::new(
            ScenarioId::MagitStatus,
            PathBuf::from("/owned/romacs"),
            NonZeroU32::new(10).expect("nonzero workload iterations"),
        )
        .with_frontend(Frontend::Batch)
        .with_inherited_environment(environment)
    };
    let a = request("/owned/A-gc-window.json");
    let b = request("/owned/B-gc-window.json");
    assert_eq!(
        a.benchmark_environment().get("ROMACS_PERF_GC_WINDOW_FILE"),
        Some(&OsString::from("/owned/A-gc-window.json"))
    );
    assert_eq!(
        b.benchmark_environment().get("ROMACS_PERF_GC_WINDOW_FILE"),
        Some(&OsString::from("/owned/B-gc-window.json"))
    );
    assert_eq!(
        a.benchmark_environment().get("ROMACS_REGEX_DFA"),
        Some(&OsString::from("off"))
    );
}
