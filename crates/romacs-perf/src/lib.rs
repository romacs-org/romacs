mod artifact;
#[cfg(test)]
mod artifact_store;
mod capture;
mod catalog;
mod comparison;
mod counters;
mod execution_policy;
mod harness;
mod host;
mod input_latency;
mod native_video;
mod portable_dump;
mod profile;
mod profile_gate;
mod suite;

pub use artifact::{
    ArtifactFile, ArtifactKind, CorrectnessMismatch, EditorCapabilities, EditorKind,
    EditorProvenance, Measurement, MetricName, MetricUnit, PortableDumpProvenance, RunArtifact,
    RunVerdict,
};
pub use capture::CaptureRoute;
pub use catalog::{
    CrossEditorParityMetric, Frontend, ScenarioId, ScenarioSpec, scenario, scenarios,
};
pub use comparison::{
    COMPARISON_ARTIFACT_SCHEMA_VERSION, ComparisonObservation, evaluate_comparison,
};
pub use comparison::{
    ComparisonArtifact, ComparisonInput, ComparisonMetricSummary, ComparisonPairedSample,
    ComparisonRejection, ComparisonReport, ComparisonRequest, ComparisonRun, ComparisonRunOutcome,
    ComparisonRunRole, ComparisonSampleCount, ComparisonVerdict, comparison_schedule,
};
pub use counters::{CounterScope, PerfStatCapture, parse_perf_stat_csv};
pub use execution_policy::{ExecutionOverride, ExecutionOverrides};
#[cfg(test)]
pub(crate) use harness::ARTIFACT_SCHEMA_VERSION;
#[cfg(test)]
pub(crate) use harness::PerfHarness;
pub use harness::{PerfCapture, ProfileCaptureError, perf_data_sample_count, profile_verdict};
pub use harness::{
    PerfError, RunReport, RunRequest, validate_fixture_result, validate_harness_build,
    validate_harness_revision,
};
#[cfg(test)]
pub(crate) use host::cpu_list_contains;
pub use host::{HostProvenance, MachinePolicy, collect_host_provenance, validate_machine_policy};
pub use native_video::{
    InvalidPresentationTarget, NativeVideoComparisonIdentity, NativeVideoDecoderKind,
    NativeVideoExecutionIdentity, NativeVideoFrameFormat, NativeVideoFrameRate,
    NativeVideoGpuTimingStatus, NativeVideoGraphicsBackend, NativeVideoMediaMetadata,
    NativeVideoPresentationTarget,
};
pub use profile::{
    NativeProfiler, PerfCallGraph, PerfCaptureConfiguration, PerfSamplingEvent, PerfSamplingRate,
    ProfileArtifact, ProfileRejection, ProfileReport, ProfileReportStyle, ProfileRequest,
    ProfileScope, ProfileVerdict,
};
#[cfg(test)]
pub(crate) use profile_gate::ProfileGate;
pub use suite::{SUITE_ARTIFACT_SCHEMA_VERSION, evaluate_suite, read_history_link};
pub use suite::{
    SuiteArtifact, SuiteHistoryLink, SuiteId, SuiteRegression, SuiteReport, SuiteRequest,
    SuiteScenario, SuiteScenarioResult, SuiteVerdict,
};

/// Artifact/test scratch root: nextest's runtime override, otherwise this crate.
pub fn workspace_root() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_WORKSPACE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

#[cfg(test)]
mod tests;

/// Analyze already captured compositor-confirmed input latency records.
pub fn report_input_latency(text: &str, budget_us: u64) -> Result<serde_json::Value, String> {
    input_latency::report(text, budget_us)
}

pub use portable_dump::{
    parse_status as parse_portable_dump_status, verify_run as verify_portable_dump_run,
};

pub use portable_dump::configure_capture as configure_portable_dump_capture;
