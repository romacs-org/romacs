//! Offline validation of recorded workload results; no editor/package preparation.

use crate::harness::*;
pub(crate) mod first_entry;
#[derive(Debug, Deserialize)]
#[serde(try_from = "VmLoopResultWire")]
pub(crate) struct VmLoopResult {
    wire: VmLoopResultWire,
    outcome: ScenarioOutcome,
}

impl VmLoopResult {
    #[cfg(test)]
    pub(crate) const fn elapsed_us(&self) -> u64 {
        self.wire.elapsed_us
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VmLoopResultWire {
    schema_version: u32,
    scenario: ScenarioId,
    status: ScenarioStatus,
    iterations: u32,
    elapsed_us: u64,
    elapsed_wall_us: u64,
    completed_operations: u32,
    result_sum: i64,
    held_value: i64,
    outer_value_before: i64,
    outer_value_after: i64,
    warmup_result: [i64; 3],
    warmup_outer_value: i64,
    // Old loop artifacts predate these checks. New alias/local cases require
    // them; supplied values are checked for every scenario.
    global_value_after: Option<i64>,
    warmup_global_value: Option<i64>,
    buffer_local: Option<bool>,
    dynamic_binding: bool,
    bytecode_compiled: bool,
    #[serde(deserialize_with = "deserialize_optional_error", rename = "error")]
    error: Option<String>,
}

impl TryFrom<VmLoopResultWire> for VmLoopResult {
    type Error = String;

    fn try_from(mut wire: VmLoopResultWire) -> Result<Self, Self::Error> {
        let outcome = scenario_outcome(wire.status, wire.error.take())?;
        Ok(Self { wire, outcome })
    }
}

fn expected_sum(iterations: u32) -> i64 {
    let n = u64::from(iterations);
    // Even u32::MAX * (u32::MAX - 1) fits u64; the halved result fits i64.
    (n * n.saturating_sub(1) / 2) as i64
}

fn expected_loop_sum(scenario: ScenarioId, iterations: u32) -> i64 {
    if matches!(
        scenario,
        ScenarioId::DynamicVariableReadLoop
            | ScenarioId::DynamicAliasReadLoop
            | ScenarioId::BufferLocalReadLoop
    ) {
        i64::from(iterations) * 7
    } else {
        expected_sum(iterations)
    }
}

pub(crate) fn validate_vm_loop_result(
    request: &RunRequest,
    result: &VmLoopResult,
) -> Vec<CorrectnessMismatch> {
    let mut mismatches = Vec::new();
    let r = &result.wire;
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
    mismatch(
        &mut mismatches,
        "sum",
        expected_loop_sum(request.scenario, request.iterations.get()),
        r.result_sum,
    );
    mismatch(&mut mismatches, "held-value", 7, r.held_value);
    mismatch(
        &mut mismatches,
        "outer-value-before",
        17,
        r.outer_value_before,
    );
    mismatch(
        &mut mismatches,
        "outer-value-after",
        17,
        r.outer_value_after,
    );
    mismatch(
        &mut mismatches,
        "warmup-operations",
        100,
        r.warmup_result[0],
    );
    mismatch(
        &mut mismatches,
        "warmup-sum",
        expected_loop_sum(request.scenario, 100),
        r.warmup_result[1],
    );
    mismatch(&mut mismatches, "warmup-held-value", 7, r.warmup_result[2]);
    mismatch(
        &mut mismatches,
        "warmup-outer-value",
        17,
        r.warmup_outer_value,
    );
    mismatch(
        &mut mismatches,
        "dynamic-binding",
        matches!(
            request.scenario,
            ScenarioId::DynamicBindingLoop
                | ScenarioId::DynamicVariableReadLoop
                | ScenarioId::DynamicRebindingLoop
                | ScenarioId::DynamicAliasReadLoop
                | ScenarioId::BufferLocalReadLoop
        ),
        r.dynamic_binding,
    );
    let require_context = matches!(
        request.scenario,
        ScenarioId::DynamicAliasReadLoop | ScenarioId::BufferLocalReadLoop
    );
    validate_context_field(
        &mut mismatches,
        "global-value-after",
        17,
        r.global_value_after,
        require_context,
    );
    validate_context_field(
        &mut mismatches,
        "warmup-global-value",
        17,
        r.warmup_global_value,
        require_context,
    );
    validate_context_field(
        &mut mismatches,
        "buffer-local",
        request.scenario == ScenarioId::BufferLocalReadLoop,
        r.buffer_local,
        require_context,
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

fn validate_context_field<T: PartialEq + std::fmt::Display>(
    mismatches: &mut Vec<CorrectnessMismatch>,
    invariant: &str,
    expected: T,
    actual: Option<T>,
    required: bool,
) {
    match actual {
        Some(actual) => mismatch(mismatches, invariant, expected, actual),
        None if required => mismatches.push(CorrectnessMismatch {
            invariant: invariant.to_string(),
            expected: expected.to_string(),
            actual: "missing".to_string(),
        }),
        None => {}
    }
}

pub(crate) fn valid_vm_loop_measurements(
    result: &VmLoopResult,
    wall_elapsed_us: u128,
) -> Vec<Measurement> {
    let r = &result.wire;
    loop_measurements(
        r.iterations,
        r.completed_operations,
        r.elapsed_us,
        r.elapsed_wall_us,
        wall_elapsed_us,
    )
}

fn loop_measurements(
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
#[path = "vm_loop/tests/vm_loop_test.rs"]
mod tests;
