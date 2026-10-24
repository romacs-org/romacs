//! Offline validation of recorded workload results; no editor/package preparation.

use crate::harness::*;
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Settings {
    pub(crate) buffer_size: usize,
    pub(crate) edit: bool,
    pub(crate) search: bool,
}

pub(crate) const fn settings(scenario: ScenarioId) -> Settings {
    match scenario {
        ScenarioId::BoundedSearchEditSmall => Settings {
            buffer_size: 1024,
            edit: true,
            search: true,
        },
        ScenarioId::BoundedSearchEditLarge => Settings {
            buffer_size: 8 * 1024 * 1024,
            edit: true,
            search: true,
        },
        ScenarioId::BoundedSearchEditOnly => Settings {
            buffer_size: 8 * 1024 * 1024,
            edit: true,
            search: false,
        },
        ScenarioId::BoundedSearchNoEdit => Settings {
            buffer_size: 8 * 1024 * 1024,
            edit: false,
            search: true,
        },
        _ => unreachable!(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "BoundedSearchResultWire")]
pub(crate) struct BoundedSearchResult {
    wire: BoundedSearchResultWire,
    outcome: ScenarioOutcome,
}

impl BoundedSearchResult {
    #[cfg(test)]
    pub(crate) const fn elapsed_us(&self) -> u64 {
        self.wire.elapsed_us
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundedSearchResultWire {
    schema_version: u32,
    scenario: ScenarioId,
    status: ScenarioStatus,
    iterations: u32,
    elapsed_us: u64,
    elapsed_wall_us: u64,
    buffer_size: usize,
    buffer_bytes: usize,
    multibyte: bool,
    edit: bool,
    search: bool,
    completed_operations: u32,
    failed_searches: u32,
    point: u32,
    match_data: Vec<u32>,
    initial_checksum: String,
    final_checksum: String,
    bytecode_compiled: bool,
    #[serde(deserialize_with = "deserialize_optional_error", rename = "error")]
    error: Option<String>,
}

impl TryFrom<BoundedSearchResultWire> for BoundedSearchResult {
    type Error = String;

    fn try_from(mut wire: BoundedSearchResultWire) -> Result<Self, Self::Error> {
        let outcome = scenario_outcome(wire.status, wire.error.take())?;
        Ok(Self { wire, outcome })
    }
}

fn expected_checksum(size: usize) -> String {
    use sha2::{Digest, Sha256};
    // The Lisp fixture inserts eight ASCII characters, one three-byte
    // character immediately after the bound, then ASCII to SIZE characters.
    let mut hash = Sha256::new();
    hash.update("aaaaaaaa中".as_bytes());
    let chunk = [b'a'; 4096];
    for _ in 0..(size - 9) / chunk.len() {
        hash.update(chunk);
    }
    hash.update(&chunk[..(size - 9) % chunk.len()]);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn validate_bounded_search_result(
    request: &RunRequest,
    result: &BoundedSearchResult,
) -> Vec<CorrectnessMismatch> {
    let mut mismatches = Vec::new();
    let r = &result.wire;
    let expected = settings(request.scenario);
    let checksum = expected_checksum(expected.buffer_size);
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
        "failed-searches",
        if expected.search {
            request.iterations.get()
        } else {
            0
        },
        r.failed_searches,
    );
    mismatch(
        &mut mismatches,
        "buffer-size",
        expected.buffer_size,
        r.buffer_size,
    );
    mismatch(
        &mut mismatches,
        "buffer-bytes",
        expected.buffer_size + 2,
        r.buffer_bytes,
    );
    mismatch(&mut mismatches, "multibyte", true, r.multibyte);
    mismatch(&mut mismatches, "edit-enabled", expected.edit, r.edit);
    mismatch(&mut mismatches, "search-enabled", expected.search, r.search);
    mismatch(&mut mismatches, "point", 1, r.point);
    mismatch(
        &mut mismatches,
        "match-data",
        format!("{:?}", [7, 9]),
        format!("{:?}", r.match_data),
    );
    mismatch(
        &mut mismatches,
        "initial-content",
        checksum.as_str(),
        r.initial_checksum.as_str(),
    );
    mismatch(
        &mut mismatches,
        "final-content",
        checksum.as_str(),
        r.final_checksum.as_str(),
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

pub(crate) fn valid_bounded_search_measurements(
    result: &BoundedSearchResult,
    wall_elapsed_us: u128,
) -> Vec<Measurement> {
    let r = &result.wire;
    [
        (MetricName::ProcessWallTime, wall_elapsed_us as f64),
        (MetricName::WorkloadCpuTime, r.elapsed_us as f64),
        (MetricName::WorkloadWallTime, r.elapsed_wall_us as f64),
        (
            MetricName::PerOperationCpuTime,
            r.elapsed_us as f64 / f64::from(r.iterations),
        ),
        (
            MetricName::PerOperationWallTime,
            r.elapsed_wall_us as f64 / f64::from(r.iterations),
        ),
        (
            MetricName::OperationCount,
            f64::from(r.completed_operations),
        ),
        (MetricName::Iterations, f64::from(r.iterations)),
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
#[path = "bounded_search/tests/bounded_search_test.rs"]
mod tests;
