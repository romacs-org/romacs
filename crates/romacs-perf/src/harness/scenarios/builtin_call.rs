//! Offline validation of recorded workload results; no editor/package preparation.

use crate::harness::*;
#[derive(Debug, Deserialize)]
#[serde(try_from = "BuiltinCallResultWire")]
pub(crate) struct BuiltinCallResult {
    wire: BuiltinCallResultWire,
    outcome: ScenarioOutcome,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuiltinCallResultWire {
    schema_version: u32,
    scenario: ScenarioId,
    status: ScenarioStatus,
    iterations: u32,
    completed_operations: u32,
    elapsed_us: u64,
    elapsed_wall_us: u64,
    result_value: String,
    call_alias: String,
    warmup_iterations: u32,
    warmup_results: Vec<String>,
    bytecode_compiled: bool,
    alias_retained: bool,
    #[serde(deserialize_with = "deserialize_optional_error", rename = "error")]
    error: Option<String>,
}

impl TryFrom<BuiltinCallResultWire> for BuiltinCallResult {
    type Error = String;

    fn try_from(mut wire: BuiltinCallResultWire) -> Result<Self, Self::Error> {
        let outcome = scenario_outcome(wire.status, wire.error.take())?;
        Ok(Self { wire, outcome })
    }
}

impl BuiltinCallResult {
    #[cfg(test)]
    pub(crate) const fn elapsed_us(&self) -> u64 {
        self.wire.elapsed_us
    }
}

fn expected(scenario: ScenarioId) -> (&'static str, &'static str) {
    match scenario {
        ScenarioId::BuiltinCallPoint => ("romacs-perf-builtin-call-zero", "1"),
        ScenarioId::BuiltinCallStringBytes => ("romacs-perf-builtin-call-one", "3"),
        ScenarioId::BuiltinCallStringLessp => ("romacs-perf-builtin-call-two", "t"),
        ScenarioId::BuiltinCallGetTextProperty => ("romacs-perf-builtin-call-three", "nil"),
        ScenarioId::BuiltinCallMultibyteStringP => ("romacs-perf-builtin-call-multibyte", "t"),
        ScenarioId::BuiltinCallCharOrStringP => ("romacs-perf-builtin-call-character", "t"),
        ScenarioId::BuiltinCallMaxChar => ("romacs-perf-builtin-call-vector-control", "1114111"),
        _ => unreachable!("builtin call adapter requires a builtin call scenario"),
    }
}

pub(crate) fn validate(
    request: &RunRequest,
    result: &BuiltinCallResult,
) -> Vec<CorrectnessMismatch> {
    let r = &result.wire;
    let (alias, value) = expected(request.scenario);
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
    mismatch(&mut mismatches, "call-alias", alias, r.call_alias.as_str());
    mismatch(
        &mut mismatches,
        "result-value",
        value,
        r.result_value.as_str(),
    );
    mismatch(
        &mut mismatches,
        "warmup-iterations",
        2000,
        r.warmup_iterations,
    );
    mismatch(&mut mismatches, "warmup-count", 40, r.warmup_results.len());
    for (index, actual) in r.warmup_results.iter().enumerate() {
        mismatch(
            &mut mismatches,
            &format!("warmup-result-{index}"),
            value,
            actual.as_str(),
        );
    }
    mismatch(
        &mut mismatches,
        "bytecode-compiled",
        true,
        r.bytecode_compiled,
    );
    mismatch(&mut mismatches, "alias-retained", true, r.alias_retained);
    crate::harness::require_positive_phase(&mut mismatches, "elapsed-cpu-time", r.elapsed_us);
    crate::harness::require_positive_phase(&mut mismatches, "elapsed-wall-time", r.elapsed_wall_us);
    mismatches
}

pub(crate) fn measurements(result: &BuiltinCallResult, process_us: u128) -> Vec<Measurement> {
    let r = &result.wire;
    call_measurements(
        r.iterations,
        r.completed_operations,
        r.elapsed_us,
        r.elapsed_wall_us,
        process_us,
    )
}

fn call_measurements(
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
#[path = "builtin_call/tests/builtin_call_test.rs"]
mod tests;
