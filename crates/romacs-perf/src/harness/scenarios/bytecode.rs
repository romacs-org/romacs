//! Offline validation of recorded workload results; no editor/package preparation.

use crate::harness::*;
#[derive(Debug, Deserialize)]
#[serde(try_from = "BytecodeCallLoopResultWire")]
pub(crate) struct BytecodeCallLoopResult {
    schema_version: u32,
    scenario: ScenarioId,
    outcome: ScenarioOutcome,
    iterations: u32,
    /// Read by the harness's `#[cfg(test)]` elapsed-time helper.
    pub(crate) elapsed_us: u64,
    bytecode_calls: u64,
    result: i64,
    expected_result: i64,
    bytecode_functions_compiled: bool,
    interpreter_requested: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BytecodeCallLoopResultWire {
    schema_version: u32,
    scenario: ScenarioId,
    status: ScenarioStatus,
    iterations: u32,
    elapsed_us: u64,
    bytecode_calls: u64,
    result: i64,
    expected_result: i64,
    bytecode_functions_compiled: bool,
    interpreter_requested: bool,
    #[serde(deserialize_with = "deserialize_optional_error", rename = "error")]
    error: Option<String>,
}

impl TryFrom<BytecodeCallLoopResultWire> for BytecodeCallLoopResult {
    type Error = String;

    fn try_from(wire: BytecodeCallLoopResultWire) -> Result<Self, Self::Error> {
        let outcome = scenario_outcome(wire.status, wire.error)?;
        Ok(Self {
            schema_version: wire.schema_version,
            scenario: wire.scenario,
            outcome,
            iterations: wire.iterations,
            elapsed_us: wire.elapsed_us,
            bytecode_calls: wire.bytecode_calls,
            result: wire.result,
            expected_result: wire.expected_result,
            bytecode_functions_compiled: wire.bytecode_functions_compiled,
            interpreter_requested: wire.interpreter_requested,
        })
    }
}

pub(crate) fn validate_bytecode_call_loop_result(
    request: &RunRequest,
    result: &BytecodeCallLoopResult,
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
        request.scenario,
        result.scenario,
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
    mismatch(
        &mut mismatches,
        "bytecode-call-count",
        u64::from(request.iterations.get()),
        result.bytecode_calls,
    );
    mismatch(
        &mut mismatches,
        "bytecode-result",
        result.expected_result,
        result.result,
    );
    mismatch(
        &mut mismatches,
        "expected-bytecode-result",
        1,
        result.expected_result,
    );
    mismatch(
        &mut mismatches,
        "bytecode-functions-compiled",
        true,
        result.bytecode_functions_compiled,
    );
    mismatch(
        &mut mismatches,
        "tier-0-interpreter-requested",
        true,
        result.interpreter_requested,
    );
    mismatches
}

pub(crate) fn valid_bytecode_call_loop_measurements(
    result: &BytecodeCallLoopResult,
    wall_elapsed_us: u128,
) -> Vec<Measurement> {
    vec![
        Measurement {
            name: MetricName::ProcessWallTime,
            value: wall_elapsed_us as f64,
            unit: MetricUnit::Microseconds,
        },
        Measurement {
            name: MetricName::WorkloadCpuTime,
            value: result.elapsed_us as f64,
            unit: MetricUnit::Microseconds,
        },
        Measurement {
            name: MetricName::PerBytecodeCallCpuTime,
            value: result.elapsed_us as f64 / result.bytecode_calls.max(1) as f64,
            unit: MetricUnit::MicrosecondsPerBytecodeCall,
        },
        Measurement {
            name: MetricName::BytecodeCalls,
            value: result.bytecode_calls as f64,
            unit: MetricUnit::Count,
        },
        Measurement {
            name: MetricName::Iterations,
            value: f64::from(result.iterations),
            unit: MetricUnit::Count,
        },
    ]
}
