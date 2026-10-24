//! Real display queries and pixelwise window resizing, including macOS.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use romacs_gui_tests::{
    DisplayHarness, GuiBackend, GuiRunOptions, GuiScenario, GuiTestPlan, ProcessGuiCommandRunner,
};
use std::{fs, path::PathBuf, time::Duration};

fn run_geometry_probe(probe: &str) {
    let root = romacs_infra::crate_root!().join("../..");
    let backend = if cfg!(target_os = "macos") {
        GuiBackend::Macos
    } else {
        GuiBackend::LinuxX11
    };
    let artifacts = root
        .join("target/romacs-gui-tests")
        .join(format!("geometry-{probe}-gnu"));
    let session = DisplayHarness::for_backend(backend)
        .start_session(&artifacts)
        .unwrap();
    let program = std::env::var_os("ROMACS_GNU_GUI_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("emacs"));
    fs::create_dir_all(artifacts.join("tmp")).unwrap();
    let mut plan = GuiTestPlan::new(
        backend,
        &root,
        &artifacts,
        GuiScenario::new(
            probe,
            root.join("crates/romacs-gui-tests/fixtures/display-window-geometry.el"),
        ),
    )
    .with_program(program)
    .with_env("ROMACS_GEOMETRY_PROBE", probe)
    .with_env("TMPDIR", artifacts.join("tmp").to_string_lossy())
    .with_env("RUST_LOG", "warn");
    for (key, value) in session.env() {
        plan = plan.with_env(key, value);
    }
    let result = plan
        .run_with(
            &mut ProcessGuiCommandRunner,
            GuiRunOptions::with_timeout(Duration::from_secs(20)),
        )
        .unwrap();
    let stderr = fs::read_to_string(&result.artifacts.stderr).unwrap_or_default();
    assert!(!result.timed_out, "{result:#?}\n{stderr}");
    assert_eq!(result.exit_code, Some(0), "{result:#?}\n{stderr}");
    assert!(stderr.contains("GEOMETRY-PASS"), "{stderr}");
}

// Prerequisites: GNU GUI Emacs and Xvfb, as for frame_resize_oracle.
#[cfg(target_os = "linux")]
#[test]
fn gnu_pixelwise_window_resize_oracle() {
    run_geometry_probe("pixelwise");
}
