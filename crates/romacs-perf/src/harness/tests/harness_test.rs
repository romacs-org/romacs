use std::fs;
use std::num::NonZeroU32;

use crate::{
    Frontend, MetricName, PerfHarness, RunRequest, RunVerdict, ScenarioId, validate_harness_build,
    validate_harness_revision,
};

#[test]
fn mx_tab_result_is_valid_only_when_the_real_completion_window_lifecycle_completed() {
    let workspace_tmp = crate::workspace_root().join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let workspace = tempfile::Builder::new()
        .prefix("romacs-perf-mx-tab-result-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let harness = PerfHarness::new(workspace.path());
    let request = RunRequest::new(
        ScenarioId::MxTabCompletion,
        workspace.path().join("fake-romacs"),
        NonZeroU32::new(2).expect("non-zero iterations"),
    )
    .with_frontend(Frontend::Tui {
        rows: 40,
        columns: 120,
    });

    let report = harness
        .record_fixture_result(
            &request,
            r##"{
              "schema_version": 1,
              "scenario": "mx-tab-completion",
              "status": "ok",
              "iterations": 2,
              "elapsed_us": 1000,
              "completion_help_calls": 2,
              "completion_visible": true,
              "completion_mode_correct": true,
              "known_commands_present": true,
              "completion_candidate_count": 1024,
              "candidate_count_stable": true,
              "completion_hidden_after_exit": true,
              "minibuffer_depth_restored": true,
              "selected_buffer_restored": true,
              "error": null
            }"##,
        )
        .expect("record fixture result");

    let RunVerdict::Valid { measurements } = report.artifact.verdict else {
        panic!("correct M-x TAB lifecycle was rejected")
    };
    let per_completion = measurements
        .iter()
        .find(|measurement| measurement.name == crate::MetricName::PerCompletionCpuTime)
        .expect("per-completion metric");
    assert_eq!(per_completion.value, 500.0);
    assert_eq!(
        per_completion.unit,
        crate::MetricUnit::MicrosecondsPerCompletion
    );
}

#[test]
fn mx_tab_result_cannot_treat_a_missing_completion_window_as_a_fast_sample() {
    let workspace_tmp = crate::workspace_root().join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let workspace = tempfile::Builder::new()
        .prefix("romacs-perf-mx-tab-mismatch-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let harness = PerfHarness::new(workspace.path());
    let request = RunRequest::new(
        ScenarioId::MxTabCompletion,
        workspace.path().join("fake-romacs"),
        NonZeroU32::new(1).expect("non-zero iterations"),
    );

    let report = harness
        .record_fixture_result(
            &request,
            r##"{
              "schema_version": 1,
              "scenario": "mx-tab-completion",
              "status": "ok",
              "iterations": 1,
              "elapsed_us": 1,
              "completion_help_calls": 1,
              "completion_visible": false,
              "completion_mode_correct": true,
              "known_commands_present": true,
              "completion_candidate_count": 1023,
              "candidate_count_stable": false,
              "completion_hidden_after_exit": true,
              "minibuffer_depth_restored": true,
              "selected_buffer_restored": true,
              "error": null
            }"##,
        )
        .expect("record fixture result");

    let RunVerdict::CorrectnessMismatch { mismatches } = report.artifact.verdict else {
        panic!("missing completion window was accepted")
    };
    assert!(
        mismatches
            .iter()
            .any(|mismatch| mismatch.invariant == "completion-window-visible")
    );
    assert!(
        mismatches
            .iter()
            .any(|mismatch| mismatch.invariant == "completion-candidate-count-stable")
    );
    assert!(
        mismatches
            .iter()
            .any(|mismatch| mismatch.invariant == "completion-candidate-count")
    );
}

#[test]
fn bytecode_call_loop_accepts_only_the_full_interpreted_call_count() {
    let workspace_tmp = crate::workspace_root().join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let workspace = tempfile::Builder::new()
        .prefix("romacs-perf-bytecode-call-result-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let harness = PerfHarness::new(workspace.path());
    let request = RunRequest::new(
        ScenarioId::BytecodeCallLoop,
        workspace.path().join("fake-romacs"),
        NonZeroU32::new(20).expect("non-zero iterations"),
    );

    let report = harness
        .record_fixture_result(
            &request,
            r##"{
              "schema_version": 1,
              "scenario": "bytecode-call-loop",
              "status": "ok",
              "iterations": 20,
              "elapsed_us": 10,
              "bytecode_calls": 20,
              "result": 1,
              "expected_result": 1,
              "bytecode_functions_compiled": true,
              "interpreter_requested": true,
              "error": null
            }"##,
        )
        .expect("record fixture result");

    let RunVerdict::Valid { measurements } = report.artifact.verdict else {
        panic!("correct bytecode call loop was rejected")
    };
    let per_call = measurements
        .iter()
        .find(|measurement| measurement.name == crate::MetricName::PerBytecodeCallCpuTime)
        .expect("per-bytecode-call metric");
    assert_eq!(per_call.value, 0.5);
    assert_eq!(
        per_call.unit,
        crate::MetricUnit::MicrosecondsPerBytecodeCall
    );
}

#[test]
fn bytecode_call_loop_rejects_a_short_or_wrong_result() {
    let workspace_tmp = crate::workspace_root().join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let workspace = tempfile::Builder::new()
        .prefix("romacs-perf-bytecode-call-mismatch-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let harness = PerfHarness::new(workspace.path());
    let request = RunRequest::new(
        ScenarioId::BytecodeCallLoop,
        workspace.path().join("fake-romacs"),
        NonZeroU32::new(20).expect("non-zero iterations"),
    );

    let report = harness
        .record_fixture_result(
            &request,
            r##"{
              "schema_version": 1,
              "scenario": "bytecode-call-loop",
              "status": "ok",
              "iterations": 20,
              "elapsed_us": 1,
              "bytecode_calls": 19,
              "result": 20,
              "expected_result": 1,
              "bytecode_functions_compiled": true,
              "interpreter_requested": true,
              "error": null
            }"##,
        )
        .expect("record fixture result");

    let RunVerdict::CorrectnessMismatch { mismatches } = report.artifact.verdict else {
        panic!("incomplete bytecode call loop was accepted")
    };
    assert!(
        mismatches
            .iter()
            .any(|mismatch| mismatch.invariant == "bytecode-call-count")
    );
    assert!(
        mismatches
            .iter()
            .any(|mismatch| mismatch.invariant == "bytecode-result")
    );
}

#[test]
fn editing_simulation_requires_every_promoted_phase_measurement() {
    let workspace_root = crate::workspace_root();
    let workspace_tmp = workspace_root.join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let scratch = tempfile::Builder::new()
        .prefix("romacs-perf-editor-workload-test-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let request = RunRequest::new(
        ScenarioId::EditingSimulation,
        "/unused/editor",
        NonZeroU32::new(1).expect("non-zero literal"),
    );
    let raw_result = r#"{
      "schema_version": 1,
      "scenario": "editing-simulation",
      "status": "ok",
      "iterations": 1,
      "elapsed_us": 100,
      "elapsed_wall_us": 120,
      "operation_count": 1,
      "initial_checksum": "same",
      "final_checksum": "same",
      "point_restored": true,
      "expected_major_mode": "emacs-lisp-mode",
      "actual_major_mode": "emacs-lisp-mode",
      "type_phase_us": 1,
      "comment_phase_us": 1,
      "kill_yank_phase_us": 1,
      "indent_phase_us": 1,
      "regex_phase_us": 1,
      "latency_samples_us": [],
      "mode_phase_us": 1,
      "fontify_phase_us": 1,
      "replace_phase_us": 1,
      "undo_redo_phase_us": 1,
      "isearch_phase_us": 1,
      "buffer_switch_phase_us": 1,
      "how_many_phase_us": 1,
      "motion_phase_us": 0,
      "error": null
    }"#;

    let report = PerfHarness::new(scratch.path())
        .record_fixture_result(&request, raw_result)
        .expect("persist invalid promoted workload result");
    let RunVerdict::CorrectnessMismatch { mismatches } = report.artifact.verdict else {
        panic!("a missing promoted phase must reject the sample")
    };
    assert!(
        mismatches
            .iter()
            .any(|mismatch| mismatch.invariant == "motion-phase-time")
    );
}

#[test]
fn sustained_editing_reports_insert_and_delete_as_two_edits() {
    let workspace_root = crate::workspace_root();
    let workspace_tmp = workspace_root.join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let scratch = tempfile::Builder::new()
        .prefix("romacs-perf-sustained-editing-test-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let request = RunRequest::new(
        ScenarioId::SustainedEditing,
        "/unused/editor",
        NonZeroU32::new(2).expect("non-zero literal"),
    );
    let raw_result = r#"{
      "schema_version": 1,
      "scenario": "sustained-editing",
      "status": "ok",
      "iterations": 2,
      "elapsed_us": 100,
      "elapsed_wall_us": 120,
      "operation_count": 2,
      "initial_checksum": "same",
      "final_checksum": "same",
      "point_restored": true,
      "expected_major_mode": "emacs-lisp-mode",
      "actual_major_mode": "emacs-lisp-mode",
      "type_phase_us": 100,
      "comment_phase_us": 0,
      "kill_yank_phase_us": 0,
      "indent_phase_us": 0,
      "regex_phase_us": 0,
      "latency_samples_us": [],
      "mode_phase_us": 0,
      "fontify_phase_us": 0,
      "replace_phase_us": 0,
      "undo_redo_phase_us": 0,
      "isearch_phase_us": 0,
      "buffer_switch_phase_us": 0,
      "how_many_phase_us": 0,
      "motion_phase_us": 0,
      "error": null
    }"#;

    let report = PerfHarness::new(scratch.path())
        .record_fixture_result(&request, raw_result)
        .expect("persist sustained workload result");
    let measurements = report
        .artifact
        .verdict
        .measurements()
        .expect("valid sustained workload measurements");
    let edits = measurements
        .iter()
        .find(|measurement| measurement.name == MetricName::Edits)
        .expect("typed edit count");
    let per_edit = measurements
        .iter()
        .find(|measurement| measurement.name == MetricName::PerEditCpuTime)
        .expect("typed per-edit duration");
    let per_edit_wall = measurements
        .iter()
        .find(|measurement| measurement.name == MetricName::PerEditWallTime)
        .expect("typed wall-clock per-edit duration");
    assert_eq!(edits.value, 4.0);
    assert_eq!(per_edit.value, 25.0);
    assert_eq!(per_edit_wall.value, 30.0);
}

#[test]
fn gui_input_latency_counts_keystrokes_over_the_one_millisecond_budget() {
    let workspace_root = crate::workspace_root();
    let workspace_tmp = workspace_root.join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let scratch = tempfile::Builder::new()
        .prefix("romacs-perf-gui-latency-budget-test-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let request = RunRequest::new(
        ScenarioId::GuiInputLatency,
        "/unused/editor",
        NonZeroU32::new(5).expect("non-zero literal"),
    );
    // Exactly 1000 us is within budget; the count is of samples strictly over it.
    let raw_result = r#"{
      "schema_version": 1,
      "scenario": "gui-input-latency",
      "status": "ok",
      "iterations": 5,
      "elapsed_us": 100,
      "elapsed_wall_us": 120,
      "operation_count": 5,
      "initial_checksum": "same",
      "final_checksum": "same",
      "point_restored": true,
      "expected_major_mode": "emacs-lisp-mode",
      "actual_major_mode": "emacs-lisp-mode",
      "type_phase_us": 0,
      "comment_phase_us": 0,
      "kill_yank_phase_us": 0,
      "indent_phase_us": 0,
      "regex_phase_us": 0,
      "latency_samples_us": [500, 1500, 999, 1000, 2000],
      "mode_phase_us": 0,
      "fontify_phase_us": 0,
      "replace_phase_us": 0,
      "undo_redo_phase_us": 0,
      "isearch_phase_us": 0,
      "buffer_switch_phase_us": 0,
      "how_many_phase_us": 0,
      "motion_phase_us": 0,
      "error": null
    }"#;

    let report = PerfHarness::new(scratch.path())
        .record_fixture_result(&request, raw_result)
        .expect("persist gui latency result");
    let measurements = report
        .artifact
        .verdict
        .measurements()
        .expect("valid gui latency measurements");
    let metric = |name: MetricName| {
        measurements
            .iter()
            .find(|measurement| measurement.name == name)
            .unwrap_or_else(|| panic!("{name:?} measurement"))
            .value
    };
    assert_eq!(metric(MetricName::InputLatencyOverBudgetCount), 2.0);
    assert_eq!(metric(MetricName::P50InputToRedisplayLatency), 1000.0);
    assert_eq!(metric(MetricName::P99InputToRedisplayLatency), 2000.0);
}

#[test]
fn mx_tab_fixture_uses_a_controlled_cross_editor_candidate_namespace() {
    let fixture = include_str!(concat!(
        env!("CARGO_WORKSPACE_DIR"),
        "/crates/romacs-perf/fixtures/mx-tab-completion.el"
    ));
    assert!(fixture.contains("romacs-perf-mx-tab--candidate-total 1024"));
    assert!(fixture.contains("romacs-perf-command-%04d"));
    assert!(fixture.contains("romacs-perf-command-0000"));
    assert!(fixture.contains("romacs-perf-command-1023"));
}

#[test]
fn stale_harness_revision_cannot_be_attributed_to_the_current_checkout() {
    assert!(validate_harness_revision("same", "same").is_ok());
    let error = validate_harness_revision("old-commit", "new-commit")
        .expect_err("stale harness must be rejected");
    assert!(error.contains("old-commit"));
    assert!(error.contains("new-commit"));
    assert!(error.contains("rebuild"));
}

#[test]
fn harness_built_from_dirty_tracked_sources_cannot_be_acceptance_evidence() {
    assert!(validate_harness_build("same", "same", false).is_ok());
    let error = validate_harness_build("same", "same", true)
        .expect_err("dirty-built harness must be rejected after the checkout is restored");
    assert!(error.contains("dirty tracked harness inputs"));
    assert!(error.contains("rebuild"));
}

/// The editor child gets the allowlisted host variables plus any operator-set
/// allowlisted collector knob (including GC trace, not the logging filter).

#[test]
fn steady_mx_tab_row_requires_its_warmup_and_reports_it() {
    let workspace_tmp = crate::workspace_root().join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let workspace = tempfile::Builder::new()
        .prefix("romacs-perf-mx-steady-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let harness = PerfHarness::new(workspace.path());
    let request = RunRequest::new(
        ScenarioId::MxTabCompletionSteady,
        workspace.path().join("fake-romacs"),
        NonZeroU32::new(50).expect("non-zero iterations"),
    )
    .with_frontend(Frontend::Tui {
        rows: 40,
        columns: 120,
    });
    let result_json = r##"{
              "schema_version": 1,
              "scenario": "mx-tab-completion-steady",
              "status": "ok",
              "iterations": 50,
              "elapsed_us": 250000,
              "completion_help_calls": 50,
              "warmup_completion_help_calls": 5,
              "completion_visible": true,
              "completion_mode_correct": true,
              "known_commands_present": true,
              "completion_candidate_count": 1024,
              "candidate_count_stable": true,
              "completion_hidden_after_exit": true,
              "minibuffer_depth_restored": true,
              "selected_buffer_restored": true,
              "error": null
            }"##;

    let report = harness
        .record_fixture_result(&request, result_json)
        .expect("record fixture result");
    let RunVerdict::Valid { measurements } = report.artifact.verdict else {
        panic!("steady row with a warm-up pass was rejected")
    };
    let warmup = measurements
        .iter()
        .find(|measurement| measurement.name == crate::MetricName::WarmupCompletionHelpCalls)
        .expect("warm-up metric is reported");
    assert_eq!(warmup.value, 5.0);
    // The timed window contains only the timed calls: warm-up never leaks in.
    let per_completion = measurements
        .iter()
        .find(|measurement| measurement.name == crate::MetricName::PerCompletionCpuTime)
        .expect("per-completion metric");
    assert_eq!(per_completion.value, 5000.0);

    // A steady row whose warm-up never ran is just a slow cold row: reject.
    let no_warmup = result_json.replace(
        "\"warmup_completion_help_calls\": 5",
        "\"warmup_completion_help_calls\": 0",
    );
    let workspace2 = tempfile::Builder::new()
        .prefix("romacs-perf-mx-steady-nowarm-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let report = PerfHarness::new(workspace2.path())
        .record_fixture_result(&request, &no_warmup)
        .expect("record fixture result");
    match report.artifact.verdict {
        RunVerdict::CorrectnessMismatch { mismatches } => assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.invariant == "warmup-completions-ran"),
            "a warm-up-free steady row must be a typed mismatch"
        ),
        verdict => panic!("steady row without warm-up was accepted: {verdict:?}"),
    }
}

#[test]
fn scrolling_row_validates_phases_checksums_and_restoration() {
    let workspace_tmp = crate::workspace_root().join("tmp");
    fs::create_dir_all(&workspace_tmp).expect("create workspace-local test scratch root");
    let workspace = tempfile::Builder::new()
        .prefix("romacs-perf-scrolling-ok-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let harness = PerfHarness::new(workspace.path());
    let request = RunRequest::new(
        ScenarioId::Scrolling,
        workspace.path().join("fake-romacs"),
        NonZeroU32::new(10).expect("non-zero iterations"),
    )
    .with_frontend(Frontend::Tui {
        rows: 40,
        columns: 120,
    });
    let result_json = r##"{
              "schema_version": 1,
              "scenario": "scrolling",
              "status": "ok",
              "iterations": 10,
              "elapsed_us": 4603000,
              "elapsed_wall_us": 4690000,
              "operation_count": 1449,
              "cold_scroll_us": 173000,
              "warm_scroll_us": 4430000,
              "cold_scroll_commands": 69,
              "warm_scroll_commands": 1380,
              "initial_checksum": "abc123",
              "final_checksum": "abc123",
              "point_restored": true,
              "window_start_restored": true,
              "expected_major_mode": "fundamental-mode",
              "actual_major_mode": "fundamental-mode",
              "error": null
            }"##;

    let report = harness
        .record_fixture_result(&request, result_json)
        .expect("record fixture result");
    let RunVerdict::Valid { measurements } = report.artifact.verdict else {
        panic!("correct scrolling run was rejected")
    };
    for expected in [
        crate::MetricName::ColdScrollPhaseCpuTime,
        crate::MetricName::WarmScrollPhaseCpuTime,
        crate::MetricName::ScrollCommandCount,
    ] {
        assert!(
            measurements
                .iter()
                .any(|measurement| measurement.name == expected),
            "scrolling must report {expected:?}"
        );
    }

    // A changed buffer during scrolling is a correctness failure, not a
    // fast sample: scrolling must never mutate what it displays.
    let mutated = result_json.replace(
        "\"final_checksum\": \"abc123\"",
        "\"final_checksum\": \"different\"",
    );
    let workspace2 = tempfile::Builder::new()
        .prefix("romacs-perf-scrolling-bad-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let report = PerfHarness::new(workspace2.path())
        .record_fixture_result(&request, &mutated)
        .expect("record fixture result");
    match report.artifact.verdict {
        RunVerdict::CorrectnessMismatch { mismatches } => assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.invariant == "final-buffer-checksum")
        ),
        verdict => panic!("scrolling run that changed the buffer was accepted: {verdict:?}"),
    }

    // Phase counters that do not sum to the operation count mean the
    // fixture mis-accounted its own work.
    let miscounted = result_json.replace("\"operation_count\": 1449", "\"operation_count\": 100");
    let workspace3 = tempfile::Builder::new()
        .prefix("romacs-perf-scrolling-mis-")
        .tempdir_in(&workspace_tmp)
        .expect("create workspace-local test directory");
    let report = PerfHarness::new(workspace3.path())
        .record_fixture_result(&request, &miscounted)
        .expect("record fixture result");
    match report.artifact.verdict {
        RunVerdict::CorrectnessMismatch { mismatches } => assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.invariant == "operation-count")
        ),
        verdict => panic!("mis-accounted scroll commands were accepted: {verdict:?}"),
    }
}

#[cfg(test)]
#[path = "gc_window_test.rs"]
mod gc_window_test;
#[path = "retained_face_gather_environment_test.rs"]
mod retained_face_gather_environment_test;

#[cfg(test)]
#[path = "property_keys_environment_test.rs"]
mod property_keys_environment_test;

#[cfg(test)]
#[path = "edit_sync_dense_index_environment_test.rs"]
mod edit_sync_dense_index_environment_test;

#[cfg(test)]
#[path = "edit_sync_fontify_coverage_environment_test.rs"]
mod edit_sync_fontify_coverage_environment_test;

#[test]
fn retired_package_and_editor_workloads_cannot_publish_fixture_measurements() {
    for scenario in [
        ScenarioId::RustLspTyping,
        ScenarioId::RustLspTypingHeavy,
        ScenarioId::MagitStatus,
        ScenarioId::MagitStatusCompiled,
        ScenarioId::MagitStatusHeavy,
        ScenarioId::OrgJournalOpen,
        ScenarioId::OrgJournalOpenCompiled,
        ScenarioId::SustainedNativeVideo,
    ] {
        let request = RunRequest::new(scenario, "/unused/editor", NonZeroU32::new(1).unwrap());
        assert!(matches!(
            crate::validate_fixture_result(&request, "{}", 1),
            Err(crate::PerfError::InvalidScenarioResult { .. })
        ));
    }
}
