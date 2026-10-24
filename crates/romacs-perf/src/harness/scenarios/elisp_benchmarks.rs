//! Offline validation of recorded workload results; no editor/package preparation.

use crate::harness::*;
const EXPECTED_BENCHMARK_COUNT: u32 = 17;
#[derive(Debug, Deserialize)]
#[serde(try_from = "ElispBenchmarksResultWire")]
pub(crate) struct ElispBenchmarksResult {
    schema_version: u32,
    scenario: ScenarioId,
    outcome: ScenarioOutcome,
    iterations: u32,
    /// Read by the harness's `#[cfg(test)]` elapsed-time helper.
    pub(crate) elapsed_us: u64,
    elapsed_wall_us: u64,
    /// Complete, validated sampling-boundary count/time triples.
    gc_window: [u64; 6],
    benchmark_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ElispBenchmarksResultWire {
    schema_version: u32,
    scenario: ScenarioId,
    status: ScenarioStatus,
    iterations: u32,
    elapsed_us: u64,
    // Required: an old artifact must not silently masquerade as a same-window
    // wall measurement or a zero-collection run.
    elapsed_wall_us: u64,
    gcs_done_start: u64,
    gcs_done_end: u64,
    gcs_done_delta: u64,
    gc_elapsed_us_start: u64,
    gc_elapsed_us_end: u64,
    gc_elapsed_us_delta: u64,
    benchmark_count: u32,
    #[serde(deserialize_with = "deserialize_optional_error", rename = "error")]
    error: Option<String>,
}

impl TryFrom<ElispBenchmarksResultWire> for ElispBenchmarksResult {
    type Error = String;

    fn try_from(wire: ElispBenchmarksResultWire) -> Result<Self, Self::Error> {
        let gc_window = [
            wire.gcs_done_start,
            wire.gcs_done_end,
            wire.gcs_done_delta,
            wire.gc_elapsed_us_start,
            wire.gc_elapsed_us_end,
            wire.gc_elapsed_us_delta,
        ];
        super::validate_gc_window(gc_window.map(Some))?;
        Ok(Self {
            schema_version: wire.schema_version,
            scenario: wire.scenario,
            outcome: scenario_outcome(wire.status, wire.error)?,
            iterations: wire.iterations,
            elapsed_us: wire.elapsed_us,
            elapsed_wall_us: wire.elapsed_wall_us,
            gc_window,
            benchmark_count: wire.benchmark_count,
        })
    }
}

pub(crate) fn validate_elisp_benchmarks_result(
    request: &RunRequest,
    result: &ElispBenchmarksResult,
) -> Vec<CorrectnessMismatch> {
    let mut mismatches = Vec::new();
    mismatch(
        &mut mismatches,
        "scenario-result-schema",
        SCENARIO_RESULT_SCHEMA_VERSION,
        result.schema_version,
    );
    mismatch(
        &mut mismatches,
        "scenario-id",
        request.scenario.workload_str(),
        result.scenario.as_str(),
    );
    mismatch(
        &mut mismatches,
        "scenario-outcome",
        &ScenarioOutcome::Ok,
        &result.outcome,
    );
    mismatch(
        &mut mismatches,
        "iterations",
        request.iterations.get(),
        result.iterations,
    );
    // A suite that quietly lost members would look like a speed-up.
    mismatch(
        &mut mismatches,
        "benchmark-count",
        EXPECTED_BENCHMARK_COUNT,
        result.benchmark_count,
    );
    if result.elapsed_us == 0 {
        mismatches.push(CorrectnessMismatch {
            invariant: "elapsed-time".to_string(),
            expected: "positive".to_string(),
            actual: "0".to_string(),
        });
    }
    crate::harness::require_positive_phase(
        &mut mismatches,
        "elapsed-wall-time",
        result.elapsed_wall_us,
    );
    mismatches
}

pub(crate) fn valid_elisp_benchmarks_measurements(
    result: &ElispBenchmarksResult,
    wall_elapsed_us: u128,
) -> Vec<Measurement> {
    let mut measurements = vec![
        Measurement {
            name: MetricName::ProcessWallTime,
            value: wall_elapsed_us as f64,
            unit: MetricUnit::Microseconds,
        },
        Measurement {
            name: MetricName::WorkloadWallTime,
            value: result.elapsed_wall_us as f64,
            unit: MetricUnit::Microseconds,
        },
        Measurement {
            name: MetricName::PerOperationWallTime,
            value: result.elapsed_wall_us as f64 / u64::from(result.iterations).max(1) as f64,
            unit: MetricUnit::MicrosecondsPerOperation,
        },
        Measurement {
            name: MetricName::WorkloadCpuTime,
            value: result.elapsed_us as f64,
            unit: MetricUnit::Microseconds,
        },
        Measurement {
            name: MetricName::PerOperationCpuTime,
            value: result.elapsed_us as f64 / u64::from(result.iterations).max(1) as f64,
            unit: MetricUnit::MicrosecondsPerOperation,
        },
        Measurement {
            name: MetricName::OperationCount,
            value: f64::from(result.iterations),
            unit: MetricUnit::Count,
        },
        Measurement {
            name: MetricName::Iterations,
            value: f64::from(result.iterations),
            unit: MetricUnit::Count,
        },
    ];
    super::append_gc_window_measurements(&mut measurements, result.gc_window.map(Some));
    measurements
}

#[cfg(test)]
#[path = "elisp_benchmarks/tests/interval_test.rs"]
mod interval_tests;
