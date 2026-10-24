//! Offline validation of recorded workload results; no editor/package preparation.

use crate::harness::*;
const MX_TAB_COMPLETION_CANDIDATE_COUNT: u64 = 1024;
#[derive(Debug, Deserialize)]
#[serde(try_from = "MxTabCompletionResultWire")]
pub(crate) struct MxTabCompletionResult {
    schema_version: u32,
    scenario: ScenarioId,
    outcome: ScenarioOutcome,
    iterations: u32,
    /// Read by the harness's `#[cfg(test)]` elapsed-time helper.
    pub(crate) elapsed_us: u64,
    completion_help_calls: u32,
    /// Zero for the cold row, always: a nonzero value means the fixture
    /// warmed up inside a run the catalog still calls cold.
    cold_warmup_completion_help_calls: u32,
    completion_visible: bool,
    completion_mode_correct: bool,
    known_commands_present: bool,
    completion_candidate_count: u64,
    candidate_count_stable: bool,
    completion_hidden_after_exit: bool,
    minibuffer_depth_restored: bool,
    selected_buffer_restored: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MxTabCompletionResultWire {
    schema_version: u32,
    scenario: ScenarioId,
    status: ScenarioStatus,
    iterations: u32,
    elapsed_us: u64,
    completion_help_calls: u32,
    /// One fixture serves both rows; the cold row must report zero, or the
    /// two time series silently collapsed into each other.
    #[serde(default)]
    warmup_completion_help_calls: u32,
    completion_visible: bool,
    completion_mode_correct: bool,
    known_commands_present: bool,
    completion_candidate_count: u64,
    candidate_count_stable: bool,
    completion_hidden_after_exit: bool,
    minibuffer_depth_restored: bool,
    selected_buffer_restored: bool,
    #[serde(deserialize_with = "deserialize_optional_error", rename = "error")]
    error: Option<String>,
}

impl TryFrom<MxTabCompletionResultWire> for MxTabCompletionResult {
    type Error = String;

    fn try_from(wire: MxTabCompletionResultWire) -> Result<Self, Self::Error> {
        let outcome = scenario_outcome(wire.status, wire.error)?;
        Ok(Self {
            schema_version: wire.schema_version,
            scenario: wire.scenario,
            outcome,
            iterations: wire.iterations,
            elapsed_us: wire.elapsed_us,
            completion_help_calls: wire.completion_help_calls,
            cold_warmup_completion_help_calls: wire.warmup_completion_help_calls,
            completion_visible: wire.completion_visible,
            completion_mode_correct: wire.completion_mode_correct,
            known_commands_present: wire.known_commands_present,
            completion_candidate_count: wire.completion_candidate_count,
            candidate_count_stable: wire.candidate_count_stable,
            completion_hidden_after_exit: wire.completion_hidden_after_exit,
            minibuffer_depth_restored: wire.minibuffer_depth_restored,
            selected_buffer_restored: wire.selected_buffer_restored,
        })
    }
}

pub(crate) fn validate_mx_tab_completion_result(
    request: &RunRequest,
    result: &MxTabCompletionResult,
) -> Vec<CorrectnessMismatch> {
    let mut mismatches = validate_shared_mx_tab_invariants(request, result);
    // The cold row's own warm-up guard: the field is carried by the wire
    // and must be zero here, or the fixture warmed up inside a run the
    // catalog still calls cold and the two time series collapse.
    mismatch(
        &mut mismatches,
        "cold-row-ran-no-warmup",
        0_u32,
        result.cold_warmup_completion_help_calls,
    );
    mismatches
}

/// Invariants both mx-tab rows share: one lifecycle, one candidate
/// namespace, full restoration. Row-specific warm-up guards live in the
/// row's own validator.
fn validate_shared_mx_tab_invariants(
    request: &RunRequest,
    result: &MxTabCompletionResult,
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
        "completion-help-calls",
        request.iterations.get(),
        result.completion_help_calls,
    );
    mismatch(
        &mut mismatches,
        "completion-window-visible",
        true,
        result.completion_visible,
    );
    mismatch(
        &mut mismatches,
        "completion-buffer-mode",
        true,
        result.completion_mode_correct,
    );
    mismatch(
        &mut mismatches,
        "known-command-candidates",
        true,
        result.known_commands_present,
    );
    mismatch(
        &mut mismatches,
        "completion-candidate-count-stable",
        true,
        result.candidate_count_stable,
    );
    mismatch(
        &mut mismatches,
        "completion-candidate-count",
        MX_TAB_COMPLETION_CANDIDATE_COUNT,
        result.completion_candidate_count,
    );
    mismatch(
        &mut mismatches,
        "completion-window-hidden-after-exit",
        true,
        result.completion_hidden_after_exit,
    );
    mismatch(
        &mut mismatches,
        "minibuffer-depth-restored",
        true,
        result.minibuffer_depth_restored,
    );
    mismatch(
        &mut mismatches,
        "selected-buffer-restored",
        true,
        result.selected_buffer_restored,
    );
    mismatches
}

pub(crate) fn valid_mx_tab_completion_measurements(
    result: &MxTabCompletionResult,
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
            name: MetricName::PerCompletionCpuTime,
            value: result.elapsed_us as f64 / f64::from(result.completion_help_calls.max(1)),
            unit: MetricUnit::MicrosecondsPerCompletion,
        },
        Measurement {
            name: MetricName::CompletionHelpCalls,
            value: f64::from(result.completion_help_calls),
            unit: MetricUnit::Count,
        },
        Measurement {
            name: MetricName::CompletionCandidateCount,
            value: result.completion_candidate_count as f64,
            unit: MetricUnit::Count,
        },
        Measurement {
            name: MetricName::Iterations,
            value: f64::from(result.iterations),
            unit: MetricUnit::Count,
        },
    ]
}

/// Result of the steady row: the cold row's full invariant set plus proof
/// the untimed warm-up pass actually ran.
#[derive(Debug, Deserialize)]
#[serde(try_from = "MxTabCompletionSteadyResultWire")]
pub(crate) struct MxTabCompletionSteadyResult {
    pub(crate) base: MxTabCompletionResult,
    warmup_completion_help_calls: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MxTabCompletionSteadyResultWire {
    schema_version: u32,
    scenario: ScenarioId,
    status: ScenarioStatus,
    iterations: u32,
    elapsed_us: u64,
    completion_help_calls: u32,
    warmup_completion_help_calls: u32,
    completion_visible: bool,
    completion_mode_correct: bool,
    known_commands_present: bool,
    completion_candidate_count: u64,
    candidate_count_stable: bool,
    completion_hidden_after_exit: bool,
    minibuffer_depth_restored: bool,
    selected_buffer_restored: bool,
    #[serde(deserialize_with = "deserialize_optional_error", rename = "error")]
    error: Option<String>,
}

impl TryFrom<MxTabCompletionSteadyResultWire> for MxTabCompletionSteadyResult {
    type Error = String;

    fn try_from(wire: MxTabCompletionSteadyResultWire) -> Result<Self, Self::Error> {
        let outcome = scenario_outcome(wire.status, wire.error)?;
        Ok(Self {
            warmup_completion_help_calls: wire.warmup_completion_help_calls,
            base: MxTabCompletionResult {
                schema_version: wire.schema_version,
                scenario: wire.scenario,
                outcome,
                iterations: wire.iterations,
                elapsed_us: wire.elapsed_us,
                completion_help_calls: wire.completion_help_calls,
                cold_warmup_completion_help_calls: wire.warmup_completion_help_calls,
                completion_visible: wire.completion_visible,
                completion_mode_correct: wire.completion_mode_correct,
                known_commands_present: wire.known_commands_present,
                completion_candidate_count: wire.completion_candidate_count,
                candidate_count_stable: wire.candidate_count_stable,
                completion_hidden_after_exit: wire.completion_hidden_after_exit,
                minibuffer_depth_restored: wire.minibuffer_depth_restored,
                selected_buffer_restored: wire.selected_buffer_restored,
            },
        })
    }
}

pub(crate) fn validate_mx_tab_completion_steady_result(
    request: &RunRequest,
    result: &MxTabCompletionSteadyResult,
) -> Vec<CorrectnessMismatch> {
    let mut mismatches = validate_shared_mx_tab_invariants(request, &result.base);
    // The row's identity is the warm-up: without it, this is just a slow
    // cold row and the two time series silently collapse into each other.
    mismatch(
        &mut mismatches,
        "warmup-completions-ran",
        true,
        result.warmup_completion_help_calls > 0,
    );
    mismatches
}

pub(crate) fn valid_mx_tab_completion_steady_measurements(
    result: &MxTabCompletionSteadyResult,
    wall_elapsed_us: u128,
) -> Vec<Measurement> {
    let mut measurements = valid_mx_tab_completion_measurements(&result.base, wall_elapsed_us);
    measurements.push(Measurement {
        name: MetricName::WarmupCompletionHelpCalls,
        value: f64::from(result.warmup_completion_help_calls),
        unit: MetricUnit::Count,
    });
    measurements
}
