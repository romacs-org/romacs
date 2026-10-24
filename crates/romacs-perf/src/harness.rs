use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufReader, Read};
use std::num::{NonZeroU32, NonZeroU64};
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use linux_perf_data::{PerfFileReader, PerfFileRecord, linux_perf_event_reader::RecordType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    ArtifactFile, ArtifactKind, CorrectnessMismatch, CounterScope, Frontend, MachinePolicy,
    Measurement, MetricName, MetricUnit, PerfCaptureConfiguration, ProfileRejection,
    ProfileReportStyle, ProfileScope, ProfileVerdict, RunArtifact, RunVerdict, ScenarioId,
    profile_gate::ProfileGate, scenario,
};

#[cfg(test)]
use crate::artifact_store::{unix_time_ms, write_json_atomically};

#[cfg(test)]
use crate::{HostProvenance, NativeVideoExecutionIdentity, collect_host_provenance};

#[cfg(test)]
mod passthrough;
pub(crate) mod scenarios;
#[cfg(test)]
pub(crate) use passthrough::passthrough_from;

#[cfg(test)]
pub(crate) const ARTIFACT_SCHEMA_VERSION: u32 = 6;
const SCENARIO_RESULT_SCHEMA_VERSION: u32 = 1;
#[cfg(test)]
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRequest {
    scenario: ScenarioId,
    editor: PathBuf,
    iterations: NonZeroU32,
    frontend: Option<Frontend>,
    timeout: Duration,
    machine: MachinePolicy,
    counters: Option<CounterScope>,
    video_file: Option<PathBuf>,
    journal_file: Option<PathBuf>,
    execution_overrides: crate::ExecutionOverrides,
    inherited_environment: BTreeMap<String, std::ffi::OsString>,
}

impl RunRequest {
    pub fn new(scenario: ScenarioId, editor: impl Into<PathBuf>, iterations: NonZeroU32) -> Self {
        Self {
            scenario,
            editor: editor.into(),
            iterations,
            frontend: None,
            timeout: Duration::from_secs(300),
            machine: MachinePolicy::default(),
            counters: None,
            video_file: None,
            journal_file: None,
            execution_overrides: crate::ExecutionOverrides::default(),
            inherited_environment: BTreeMap::new(),
        }
    }

    pub fn with_frontend(mut self, frontend: Frontend) -> Self {
        self.frontend = Some(frontend);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_machine_policy(mut self, machine: MachinePolicy) -> Self {
        self.machine = machine;
        self
    }

    pub fn with_counters(mut self, counters: Option<CounterScope>) -> Self {
        self.counters = counters;
        self
    }

    pub fn with_video_file(mut self, video_file: Option<PathBuf>) -> Self {
        self.video_file = video_file;
        self
    }

    pub fn with_journal_file(mut self, journal_file: Option<PathBuf>) -> Self {
        self.journal_file = journal_file;
        self
    }

    pub fn with_execution_overrides(mut self, overrides: crate::ExecutionOverrides) -> Self {
        self.execution_overrides = overrides;
        self
    }

    pub fn with_inherited_environment(
        mut self,
        environment: BTreeMap<String, std::ffi::OsString>,
    ) -> Self {
        self.inherited_environment = environment;
        self
    }

    /// Request-owned metadata for externally collected results; no process environment is read.
    pub fn benchmark_environment(&self) -> BTreeMap<String, std::ffi::OsString> {
        let mut environment = self.inherited_environment.clone();
        self.execution_overrides.apply_to(&mut environment);
        environment
    }

    pub const fn scenario(&self) -> ScenarioId {
        self.scenario
    }

    pub fn editor(&self) -> &Path {
        &self.editor
    }

    pub const fn iterations(&self) -> NonZeroU32 {
        self.iterations
    }

    pub fn frontend(&self) -> Frontend {
        self.frontend
            .unwrap_or_else(|| scenario(self.scenario).default_frontend)
    }

    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn machine_policy(&self) -> &MachinePolicy {
        &self.machine
    }

    pub const fn counters(&self) -> Option<CounterScope> {
        self.counters
    }

    pub fn video_file(&self) -> Option<&Path> {
        self.video_file.as_deref()
    }

    pub fn journal_file(&self) -> Option<&Path> {
        self.journal_file.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunReport {
    pub artifact: RunArtifact,
    pub artifact_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum PerfError {
    #[error("failed to create performance artifact directory {path}: {source}")]
    CreateArtifactDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write performance artifact {path}: {source}")]
    WriteArtifact {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("scenario `{scenario}` emitted an invalid result document: {source}")]
    InvalidScenarioResult {
        scenario: ScenarioId,
        source: serde_json::Error,
    },
    #[error("failed to serialize performance artifact: {0}")]
    SerializeArtifact(serde_json::Error),
    #[error("invalid previous performance suite {path}: {message}")]
    InvalidSuiteHistory { path: PathBuf, message: String },
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct PerfHarness {
    pub(crate) workspace_root: PathBuf,
}

#[cfg(test)]
impl PerfHarness {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }

    /// Validate and persist a result produced by a frontend adapter.
    ///
    /// Out-of-process adapters publish the result file before the harness sees
    /// it. Keeping validation and artifact publication together prevents an
    /// adapter from turning a mismatch into a valid sample.
    pub fn record_fixture_result(
        &self,
        request: &RunRequest,
        raw_result: &str,
    ) -> Result<RunReport, PerfError> {
        let result = parse_scenario_result(request.scenario, raw_result).map_err(|source| {
            PerfError::InvalidScenarioResult {
                scenario: request.scenario,
                source,
            }
        })?;
        let context = RunContext::create(&self.workspace_root, request)?;

        let scenario_result_path = context.directory.join("scenario-result.json");
        fs::write(&scenario_result_path, raw_result).map_err(|source| {
            PerfError::WriteArtifact {
                path: scenario_result_path.clone(),
                source,
            }
        })?;

        let verdict = result_verdict(request, &result, u128::from(result.elapsed_us()));
        context.publish(
            u128::from(result.elapsed_us()),
            result.native_video_execution_identity(),
            verdict,
            vec![ArtifactFile {
                kind: ArtifactKind::ScenarioResult,
                path: PathBuf::from("scenario-result.json"),
            }],
        )
    }
}

#[cfg(test)]
struct RunContext {
    request: RunRequest,
    run_id: String,
    started_unix_ms: u128,
    directory: PathBuf,
    host: HostProvenance,
}

#[cfg(test)]
impl RunContext {
    fn create(workspace_root: &Path, request: &RunRequest) -> Result<Self, PerfError> {
        let started_unix_ms = unix_time_ms();
        let run_id = next_run_id(request.scenario, started_unix_ms);
        let directory = workspace_root.join("tmp/perf").join(&run_id);
        fs::create_dir_all(&directory).map_err(|source| PerfError::CreateArtifactDirectory {
            path: directory.clone(),
            source,
        })?;
        Ok(Self {
            request: request.clone(),
            run_id,
            started_unix_ms,
            directory,
            host: collect_host_provenance(request.machine_policy()),
        })
    }

    fn publish(
        &self,
        total_elapsed_us: u128,
        native_video_execution: Option<NativeVideoExecutionIdentity>,
        verdict: RunVerdict,
        files: Vec<ArtifactFile>,
    ) -> Result<RunReport, PerfError> {
        let artifact = RunArtifact {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            run_id: self.run_id.clone(),
            scenario: self.request.scenario,
            frontend: self.request.frontend(),
            editor: self.request.editor.clone(),
            host: self.host.clone(),
            iterations: self.request.iterations.get(),
            started_unix_ms: self.started_unix_ms,
            total_elapsed_us,
            native_video_execution,
            verdict,
            files,
        };
        let artifact_path = self.directory.join("artifact.json");
        write_json_atomically(&artifact_path, &artifact)?;
        Ok(RunReport {
            artifact,
            artifact_path,
        })
    }
}

pub fn profile_verdict(run: &RunReport) -> ProfileVerdict {
    match &run.artifact.verdict {
        RunVerdict::CorrectnessMismatch { .. } => ProfileVerdict::Rejected {
            reason: ProfileRejection::CorrectnessMismatch,
        },
        RunVerdict::InfrastructureFailure { message } => ProfileVerdict::Rejected {
            reason: ProfileRejection::InfrastructureFailure {
                message: message.clone(),
            },
        },
        RunVerdict::Valid { .. } => {
            let Some(data) =
                artifact_path_of_kind(&run.artifact.files, ArtifactKind::NativeProfileData)
            else {
                return ProfileVerdict::Rejected {
                    reason: ProfileRejection::InfrastructureFailure {
                        message: "profile run did not retain perf.data".to_string(),
                    },
                };
            };
            let Some(report) =
                artifact_path_of_kind(&run.artifact.files, ArtifactKind::NativeProfileReport)
            else {
                return ProfileVerdict::Rejected {
                    reason: ProfileRejection::InfrastructureFailure {
                        message: "profile run did not retain its hotspot report".to_string(),
                    },
                };
            };
            let data_on_disk = run
                .artifact_path
                .parent()
                .expect("run artifact has a parent directory")
                .join(data);
            match perf_data_sample_count(&data_on_disk) {
                Ok(sample_count) => ProfileVerdict::Captured {
                    perf_data_path: data.to_path_buf(),
                    hotspot_report_path: report.to_path_buf(),
                    sample_count,
                },
                Err(message) => ProfileVerdict::Rejected {
                    reason: ProfileRejection::InfrastructureFailure { message },
                },
            }
        }
    }
}

fn artifact_path_of_kind(files: &[ArtifactFile], kind: ArtifactKind) -> Option<&Path> {
    files
        .iter()
        .find(|file| file.kind == kind)
        .map(|file| file.path.as_path())
}

pub fn perf_data_sample_count(path: &Path) -> Result<NonZeroU64, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("failed to open native profile {}: {error}", path.display()))?;
    let PerfFileReader {
        mut perf_file,
        mut record_iter,
    } = PerfFileReader::parse_file(BufReader::new(file))
        .map_err(|error| format!("failed to parse native profile {}: {error}", path.display()))?;
    let mut sample_count = 0_u64;
    while let Some(record) = record_iter
        .next_record(&mut perf_file)
        .map_err(|error| format!("failed to read native profile {}: {error}", path.display()))?
    {
        if matches!(
            record,
            PerfFileRecord::EventRecord { record, .. } if record.record_type == RecordType::SAMPLE
        ) {
            sample_count = sample_count
                .checked_add(1)
                .ok_or_else(|| "native profile sample count overflowed u64".to_string())?;
        }
    }
    NonZeroU64::new(sample_count)
        .ok_or_else(|| format!("native profile {} contained no samples", path.display()))
}

pub struct PerfCapture {
    configuration: PerfCaptureConfiguration,
    data: PathBuf,
    report: PathBuf,
    scope: ProfileScope,
    report_style: ProfileReportStyle,
    timeout: Duration,
    gate: Option<ProfileGate>,
}

impl PerfCapture {
    pub fn new(
        directory: &Path,
        configuration: PerfCaptureConfiguration,
        scope: ProfileScope,
        timeout: Duration,
    ) -> Self {
        Self {
            configuration,
            data: directory.join("perf.data"),
            report: directory.join("perf-report.txt"),
            scope,
            report_style: ProfileReportStyle::default(),
            timeout,
            gate: None,
        }
    }

    pub fn with_report_style(mut self, report_style: ProfileReportStyle) -> Self {
        self.report_style = report_style;
        self
    }

    pub fn wrap(
        &mut self,
        mut command: Command,
        route: crate::CaptureRoute,
    ) -> Result<Command, String> {
        if self.scope == ProfileScope::EditLoop {
            self.gate = Some(ProfileGate::start(
                self.data
                    .parent()
                    .expect("profile data has a parent directory"),
                self.timeout,
            )?);
        }
        if let crate::CaptureRoute::Adapter(prefix) = route {
            command.env(format!("{prefix}_PERF_RECORD"), &self.data);
            for (name, value) in self.configuration.adapter_record_environment(prefix) {
                match value {
                    Some(value) => {
                        command.env(name, value);
                    }
                    None => {
                        command.env_remove(name);
                    }
                }
            }
            self.configure_gate_environment(&mut command, Some(prefix));
            return Ok(command);
        }

        let mut profiled = Command::new("perf");
        let control = self.gate.as_ref().map(|gate| {
            let paths = gate.control_paths();
            (paths.command.as_path(), paths.acknowledgement.as_path())
        });
        profiled.args(self.configuration.record_arguments(&self.data, control));
        profiled.arg(command.get_program());
        profiled.args(command.get_args());
        if let Some(directory) = command.get_current_dir() {
            profiled.current_dir(directory);
        }
        profiled.env_clear();
        for (name, value) in command.get_envs() {
            match value {
                Some(value) => {
                    profiled.env(name, value);
                }
                None => {
                    profiled.env_remove(name);
                }
            }
        }
        self.configure_gate_environment(&mut profiled, None);
        Ok(profiled)
    }

    fn configure_gate_environment(&self, command: &mut Command, adapter_prefix: Option<&str>) {
        let Some(gate) = &self.gate else {
            return;
        };
        command.env("ROMACS_PERF_GATE_PORT", gate.endpoint().port().to_string());
        if let Some(prefix) = adapter_prefix {
            let paths = gate.control_paths();
            command.env(
                format!("{prefix}_PERF_CONTROL"),
                format!(
                    "fifo:{},{}",
                    paths.command.display(),
                    paths.acknowledgement.display()
                ),
            );
        }
    }

    pub fn finish_gate(&mut self) -> Result<(), String> {
        match &mut self.gate {
            Some(gate) => gate.finish(),
            None => Ok(()),
        }
    }

    pub fn cancel_gate(&mut self) {
        self.gate.take();
    }

    pub fn collect(&self) -> Result<Vec<ArtifactFile>, ProfileCaptureError> {
        if !self.data.is_file() {
            return Err(ProfileCaptureError {
                message: format!(
                    "perf record did not produce profile data {}",
                    self.data.display()
                ),
                files: Vec::new(),
            });
        }
        let mut files = vec![ArtifactFile {
            kind: ArtifactKind::NativeProfileData,
            path: relative_artifact_path(&self.data),
        }];
        let output = Command::new("perf")
            .args(self.report_style.report_arguments(&self.data))
            .env("LC_ALL", "C")
            .env("PERF_PAGER", "cat")
            .output()
            .map_err(|error| ProfileCaptureError {
                message: format!("failed to launch perf report: {error}"),
                files: files.clone(),
            })?;
        let mut report = output.stdout;
        if !output.stderr.is_empty() {
            report.extend_from_slice(b"\n# perf report stderr\n");
            report.extend_from_slice(&output.stderr);
        }
        fs::write(&self.report, &report).map_err(|error| ProfileCaptureError {
            message: format!(
                "failed to write native hotspot report {}: {error}",
                self.report.display()
            ),
            files: files.clone(),
        })?;
        files.push(ArtifactFile {
            kind: ArtifactKind::NativeProfileReport,
            path: relative_artifact_path(&self.report),
        });
        if !output.status.success() {
            return Err(ProfileCaptureError {
                message: format!("perf report exited with status {}", output.status),
                files,
            });
        }
        perf_data_sample_count(&self.data).map_err(|message| ProfileCaptureError {
            message,
            files: files.clone(),
        })?;
        Ok(files)
    }
}

#[derive(Debug)]
pub struct ProfileCaptureError {
    pub message: String,
    pub files: Vec<ArtifactFile>,
}

fn relative_artifact_path(path: &Path) -> PathBuf {
    path.file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf())
}

enum ScenarioResult {
    MxTabCompletion(scenarios::mx_tab::MxTabCompletionResult),
    MxTabCompletionSteady(scenarios::mx_tab::MxTabCompletionSteadyResult),
    Scrolling(scenarios::scrolling::ScrollingResult),
    BytecodeCallLoop(scenarios::bytecode::BytecodeCallLoopResult),
    VmLoop(scenarios::vm_loop::VmLoopResult),
    BuiltinCall(scenarios::builtin_call::BuiltinCallResult),
    SearchShape(scenarios::search_shape::SearchShapeResult),
    FirstHotLoop(scenarios::vm_loop::first_entry::FirstHotLoopResult),
    BoundedSearch(scenarios::bounded_search::BoundedSearchResult),
    ElispBenchmarks(scenarios::elisp_benchmarks::ElispBenchmarksResult),
    EditorWorkload(scenarios::editor_workload::EditorWorkloadResult),
}

impl ScenarioResult {
    #[cfg(test)]
    fn native_video_execution_identity(&self) -> Option<NativeVideoExecutionIdentity> {
        match self {
            _ => None,
        }
    }

    #[cfg(test)]
    const fn elapsed_us(&self) -> u64 {
        match self {
            Self::MxTabCompletion(result) => result.elapsed_us,
            Self::MxTabCompletionSteady(result) => result.base.elapsed_us,
            Self::Scrolling(result) => result.elapsed_us,
            Self::BytecodeCallLoop(result) => result.elapsed_us,
            Self::VmLoop(result) => result.elapsed_us(),
            Self::BuiltinCall(result) => result.elapsed_us(),
            Self::SearchShape(result) => result.elapsed_us(),
            Self::FirstHotLoop(result) => result.elapsed_us(),
            Self::BoundedSearch(result) => result.elapsed_us(),
            Self::ElispBenchmarks(result) => result.elapsed_us,
            Self::EditorWorkload(result) => result.elapsed_us,
        }
    }
}

fn parse_scenario_result(
    scenario: ScenarioId,
    raw: &str,
) -> Result<ScenarioResult, serde_json::Error> {
    match scenario {
        ScenarioId::MxTabCompletion => {
            serde_json::from_str(raw).map(ScenarioResult::MxTabCompletion)
        }
        ScenarioId::MxTabCompletionSteady => {
            serde_json::from_str(raw).map(ScenarioResult::MxTabCompletionSteady)
        }
        ScenarioId::Scrolling => serde_json::from_str(raw).map(ScenarioResult::Scrolling),
        ScenarioId::BoundedSearchEditSmall
        | ScenarioId::BoundedSearchEditLarge
        | ScenarioId::BoundedSearchEditOnly
        | ScenarioId::BoundedSearchNoEdit => {
            serde_json::from_str(raw).map(ScenarioResult::BoundedSearch)
        }
        ScenarioId::BuiltinCallPoint
        | ScenarioId::BuiltinCallStringBytes
        | ScenarioId::BuiltinCallStringLessp
        | ScenarioId::BuiltinCallGetTextProperty
        | ScenarioId::BuiltinCallMultibyteStringP
        | ScenarioId::BuiltinCallCharOrStringP
        | ScenarioId::BuiltinCallMaxChar => {
            serde_json::from_str(raw).map(ScenarioResult::BuiltinCall)
        }
        ScenarioId::SearchLiteralForward
        | ScenarioId::SearchLiteralBackward
        | ScenarioId::SearchRegexpForward
        | ScenarioId::SearchRegexpBackward
        | ScenarioId::SearchPosixForward
        | ScenarioId::SearchPosixBackward => {
            serde_json::from_str(raw).map(ScenarioResult::SearchShape)
        }
        ScenarioId::FirstHotLoop
        | ScenarioId::FirstHotLoop8K
        | ScenarioId::FirstHotLoop16K
        | ScenarioId::FirstHotLoop32K
        | ScenarioId::FirstBranchLoop64
        | ScenarioId::FirstBranchLoop256 => {
            serde_json::from_str(raw).map(ScenarioResult::FirstHotLoop)
        }
        ScenarioId::LexicalLoop
        | ScenarioId::DynamicBindingLoop
        | ScenarioId::DynamicVariableReadLoop
        | ScenarioId::DynamicAliasReadLoop
        | ScenarioId::BufferLocalReadLoop
        | ScenarioId::DynamicRebindingLoop => serde_json::from_str(raw).map(ScenarioResult::VmLoop),
        ScenarioId::BytecodeCallLoop => {
            serde_json::from_str(raw).map(ScenarioResult::BytecodeCallLoop)
        }
        ScenarioId::ElispBenchmarks => {
            serde_json::from_str(raw).map(ScenarioResult::ElispBenchmarks)
        }
        ScenarioId::EditingSimulation
        | ScenarioId::Startup
        | ScenarioId::SustainedEditing
        | ScenarioId::GuiInputLatency
        | ScenarioId::OrgEditing
        | ScenarioId::OrgEditingHeavy
        | ScenarioId::FileOpen
        | ScenarioId::ProcessOutput
        | ScenarioId::LargeFileEditing
        | ScenarioId::Indentation
        | ScenarioId::LspJsonRpc
        | ScenarioId::RegexSearch => serde_json::from_str(raw).map(ScenarioResult::EditorWorkload),
        _ => Err(<serde_json::Error as serde::de::Error>::custom(
            "retired editor/package workload",
        )),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ScenarioStatus {
    Ok,
    Error,
}

#[derive(Debug, Eq, PartialEq)]
enum ScenarioOutcome {
    Ok,
    Error(String),
}

impl std::fmt::Display for ScenarioOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok => formatter.write_str("ok"),
            Self::Error(message) => write!(formatter, "error: {message}"),
        }
    }
}

fn scenario_outcome(
    status: ScenarioStatus,
    error: Option<String>,
) -> Result<ScenarioOutcome, String> {
    match (status, error) {
        (ScenarioStatus::Ok, None) => Ok(ScenarioOutcome::Ok),
        (ScenarioStatus::Ok, Some(_)) => Err("status `ok` requires a null error".to_string()),
        (ScenarioStatus::Error, Some(message)) if !message.trim().is_empty() => {
            Ok(ScenarioOutcome::Error(message))
        }
        (ScenarioStatus::Error, Some(_)) => {
            Err("status `error` requires a non-empty error".to_string())
        }
        (ScenarioStatus::Error, None) => {
            Err("status `error` requires a non-null error".to_string())
        }
    }
}

pub fn validate_harness_revision(embedded: &str, checkout: &str) -> Result<(), String> {
    if embedded == checkout {
        return Ok(());
    }
    Err(format!(
        "benchmark harness was built from {embedded}, but the checkout is {checkout}; rebuild the harness before collecting acceptance evidence"
    ))
}

pub fn validate_harness_build(
    embedded: &str,
    checkout: &str,
    harness_inputs_dirty_when_built: bool,
) -> Result<(), String> {
    validate_harness_revision(embedded, checkout)?;
    if harness_inputs_dirty_when_built {
        return Err(
            "benchmark harness was built from dirty tracked harness inputs; restore clean inputs and rebuild the harness before collecting acceptance evidence"
                .to_owned(),
        );
    }
    Ok(())
}

fn deserialize_optional_error<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

fn require_positive_phase(mismatches: &mut Vec<CorrectnessMismatch>, invariant: &str, actual: u64) {
    if actual == 0 {
        mismatches.push(CorrectnessMismatch {
            invariant: invariant.to_string(),
            expected: "positive".to_string(),
            actual: actual.to_string(),
        });
    }
}

fn result_verdict(
    request: &RunRequest,
    result: &ScenarioResult,
    process_wall_us: u128,
) -> RunVerdict {
    let mismatches = match result {
        ScenarioResult::BuiltinCall(result) => scenarios::builtin_call::validate(request, result),
        ScenarioResult::SearchShape(result) => scenarios::search_shape::validate(request, result),
        ScenarioResult::FirstHotLoop(result) => {
            scenarios::vm_loop::first_entry::validate(request, result)
        }
        ScenarioResult::VmLoop(result) => {
            scenarios::vm_loop::validate_vm_loop_result(request, result)
        }
        ScenarioResult::BoundedSearch(result) => {
            scenarios::bounded_search::validate_bounded_search_result(request, result)
        }
        ScenarioResult::MxTabCompletion(result) => {
            scenarios::mx_tab::validate_mx_tab_completion_result(request, result)
        }
        ScenarioResult::MxTabCompletionSteady(result) => {
            scenarios::mx_tab::validate_mx_tab_completion_steady_result(request, result)
        }
        ScenarioResult::Scrolling(result) => {
            scenarios::scrolling::validate_scrolling_result(request, result)
        }
        ScenarioResult::BytecodeCallLoop(result) => {
            scenarios::bytecode::validate_bytecode_call_loop_result(request, result)
        }
        ScenarioResult::ElispBenchmarks(result) => {
            scenarios::elisp_benchmarks::validate_elisp_benchmarks_result(request, result)
        }
        ScenarioResult::EditorWorkload(result) => {
            scenarios::editor_workload::validate_editor_workload_result(request, result)
        }
    };
    if mismatches.is_empty() {
        RunVerdict::Valid {
            measurements: valid_measurements(result, process_wall_us),
        }
    } else {
        RunVerdict::CorrectnessMismatch { mismatches }
    }
}

fn mismatch<T>(mismatches: &mut Vec<CorrectnessMismatch>, invariant: &str, expected: T, actual: T)
where
    T: PartialEq + std::fmt::Display,
{
    if expected != actual {
        mismatches.push(CorrectnessMismatch {
            invariant: invariant.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
}

fn valid_measurements(result: &ScenarioResult, wall_elapsed_us: u128) -> Vec<Measurement> {
    match result {
        ScenarioResult::BuiltinCall(result) => {
            scenarios::builtin_call::measurements(result, wall_elapsed_us)
        }
        ScenarioResult::SearchShape(result) => {
            scenarios::search_shape::measurements(result, wall_elapsed_us)
        }
        ScenarioResult::FirstHotLoop(result) => {
            scenarios::vm_loop::first_entry::measurements(result, wall_elapsed_us)
        }
        ScenarioResult::VmLoop(result) => {
            scenarios::vm_loop::valid_vm_loop_measurements(result, wall_elapsed_us)
        }
        ScenarioResult::BoundedSearch(result) => {
            scenarios::bounded_search::valid_bounded_search_measurements(result, wall_elapsed_us)
        }
        ScenarioResult::MxTabCompletion(result) => {
            scenarios::mx_tab::valid_mx_tab_completion_measurements(result, wall_elapsed_us)
        }
        ScenarioResult::MxTabCompletionSteady(result) => {
            scenarios::mx_tab::valid_mx_tab_completion_steady_measurements(result, wall_elapsed_us)
        }
        ScenarioResult::Scrolling(result) => {
            scenarios::scrolling::valid_scrolling_measurements(result, wall_elapsed_us)
        }
        ScenarioResult::BytecodeCallLoop(result) => {
            scenarios::bytecode::valid_bytecode_call_loop_measurements(result, wall_elapsed_us)
        }
        ScenarioResult::ElispBenchmarks(result) => {
            scenarios::elisp_benchmarks::valid_elisp_benchmarks_measurements(
                result,
                wall_elapsed_us,
            )
        }
        ScenarioResult::EditorWorkload(result) => {
            scenarios::editor_workload::valid_editor_workload_measurements(result, wall_elapsed_us)
        }
    }
}

/// The per-keystroke latency budget behind
/// [`MetricName::InputLatencyOverBudgetCount`]: one millisecond, the point at
/// which a keystroke stops feeling immediate and past which every GNU sample
/// in this scenario but a handful lies.
const INPUT_LATENCY_BUDGET_US: u64 = 1_000;

fn nearest_rank(sorted_samples: &[u64], percentile: f64) -> u64 {
    let rank = (percentile * sorted_samples.len() as f64).ceil() as usize;
    sorted_samples[rank.saturating_sub(1).min(sorted_samples.len() - 1)]
}

#[cfg(test)]
fn next_run_id(scenario: ScenarioId, unix_ms: u128) -> String {
    let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{unix_ms}-{}-{sequence}",
        scenario.as_str(),
        std::process::id()
    )
}

#[cfg(test)]
#[path = "harness/tests/harness_test.rs"]
mod tests;

/// Validate an externally collected result against the requested work and observed process wall time.
/// This does not launch an editor or synthesize provenance/timing measurements.
pub fn validate_fixture_result(
    request: &RunRequest,
    raw_result: &str,
    process_wall_us: u128,
) -> Result<RunVerdict, PerfError> {
    let result = parse_scenario_result(request.scenario, raw_result).map_err(|source| {
        PerfError::InvalidScenarioResult {
            scenario: request.scenario,
            source,
        }
    })?;
    Ok(result_verdict(request, &result, process_wall_us))
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(hex)
}
