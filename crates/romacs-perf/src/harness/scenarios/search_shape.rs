//! Offline validation of recorded workload results; no editor/package preparation.

use crate::harness::*;
#[derive(Debug, Deserialize)]
#[serde(try_from = "SearchShapeResultWire")]
pub(crate) struct SearchShapeResult {
    wire: SearchShapeResultWire,
    outcome: ScenarioOutcome,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchShapeResultWire {
    schema_version: u32,
    scenario: ScenarioId,
    status: ScenarioStatus,
    iterations: u32,
    completed_operations: u32,
    elapsed_us: u64,
    elapsed_wall_us: u64,
    result_value: u32,
    point: u32,
    match_data: String,
    buffer_contents: String,
    buffer_size: u32,
    buffer_bytes: u32,
    multibyte: bool,
    warmup_single_calls: u32,
    warmup_batch_calls: u32,
    warmup_batch_iterations: u32,
    warmup_validations: u32,
    bytecode_compiled: bool,
    #[serde(deserialize_with = "deserialize_optional_error", rename = "error")]
    error: Option<String>,
}

impl TryFrom<SearchShapeResultWire> for SearchShapeResult {
    type Error = String;

    fn try_from(mut wire: SearchShapeResultWire) -> Result<Self, Self::Error> {
        let outcome = scenario_outcome(wire.status, wire.error.take())?;
        Ok(Self { wire, outcome })
    }
}

impl SearchShapeResult {
    #[cfg(test)]
    pub(crate) const fn elapsed_us(&self) -> u64 {
        self.wire.elapsed_us
    }
}

fn expected(scenario: ScenarioId) -> (u32, &'static str) {
    match scenario {
        ScenarioId::SearchLiteralForward => (4, "(2 4 t)"),
        ScenarioId::SearchLiteralBackward => (5, "(5 7 t)"),
        ScenarioId::SearchRegexpForward | ScenarioId::SearchPosixForward => (4, "(2 4 2 4 t)"),
        ScenarioId::SearchRegexpBackward | ScenarioId::SearchPosixBackward => (5, "(5 7 5 7 t)"),
        _ => unreachable!("search shape adapter requires a search shape scenario"),
    }
}

pub(crate) fn validate(
    request: &RunRequest,
    result: &SearchShapeResult,
) -> Vec<CorrectnessMismatch> {
    let r = &result.wire;
    let (value, match_data) = expected(request.scenario);
    let mut mismatches = Vec::new();
    mismatch(
        &mut mismatches,
        "scenario-result-schema",
        SCENARIO_RESULT_SCHEMA_VERSION,
        r.schema_version,
    );
    mismatch(&mut mismatches, "scenario-id", request.scenario, r.scenario);
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
        r.iterations,
    );
    mismatch(
        &mut mismatches,
        "completed-operations",
        request.iterations.get(),
        r.completed_operations,
    );
    mismatch(&mut mismatches, "result-value", value, r.result_value);
    mismatch(&mut mismatches, "point", value, r.point);
    mismatch(
        &mut mismatches,
        "match-data",
        match_data,
        r.match_data.as_str(),
    );
    mismatch(
        &mut mismatches,
        "buffer-contents",
        "éab abc",
        r.buffer_contents.as_str(),
    );
    mismatch(&mut mismatches, "buffer-size", 7, r.buffer_size);
    mismatch(&mut mismatches, "buffer-bytes", 8, r.buffer_bytes);
    mismatch(&mut mismatches, "multibyte", true, r.multibyte);
    mismatch(
        &mut mismatches,
        "warmup-single-calls",
        2000,
        r.warmup_single_calls,
    );
    mismatch(
        &mut mismatches,
        "warmup-batch-calls",
        20,
        r.warmup_batch_calls,
    );
    mismatch(
        &mut mismatches,
        "warmup-batch-iterations",
        100,
        r.warmup_batch_iterations,
    );
    mismatch(
        &mut mismatches,
        "warmup-validations",
        2020,
        r.warmup_validations,
    );
    mismatch(
        &mut mismatches,
        "bytecode-compiled",
        true,
        r.bytecode_compiled,
    );
    crate::harness::require_positive_phase(&mut mismatches, "elapsed-cpu-time", r.elapsed_us);
    crate::harness::require_positive_phase(&mut mismatches, "elapsed-wall-time", r.elapsed_wall_us);
    mismatches
}

pub(crate) fn measurements(result: &SearchShapeResult, process_us: u128) -> Vec<Measurement> {
    let r = &result.wire;
    search_measurements(
        r.iterations,
        r.completed_operations,
        r.elapsed_us,
        r.elapsed_wall_us,
        process_us,
    )
}

fn search_measurements(
    iterations: u32,
    completed: u32,
    cpu_us: u64,
    wall_us: u64,
    process_us: u128,
) -> Vec<Measurement> {
    [
        (MetricName::ProcessWallTime, process_us as f64),
        (MetricName::WorkloadCpuTime, cpu_us as f64),
        (MetricName::WorkloadWallTime, wall_us as f64),
        (
            MetricName::PerOperationCpuTime,
            cpu_us as f64 / f64::from(iterations),
        ),
        (
            MetricName::PerOperationWallTime,
            wall_us as f64 / f64::from(iterations),
        ),
        (MetricName::OperationCount, f64::from(completed)),
        (MetricName::Iterations, f64::from(iterations)),
    ]
    .into_iter()
    .map(|(name, value)| Measurement {
        name,
        value,
        unit: name.canonical_unit(),
    })
    .collect()
}

#[cfg(test)]
#[path = "search_shape/tests/search_shape_test.rs"]
mod tests;
