//! Frames on an OpenGL 3.3 GPU's limits (Intel Sandy Bridge: no compute, no
//! storage buffers): the renderer asks only what such a device allows, and
//! draws text, cursors and images on it without a validation error.

use super::*;
use std::sync::{Arc, Mutex};

fn gl33_harness() -> Option<(Harness, Arc<Mutex<Vec<String>>>)> {
    let renderer = romacs_renderer_wgpu::gpu_policy::test_gpu::gl33_renderer(W, H).ok()?;
    let errors = Arc::new(Mutex::new(Vec::new()));
    let seen = errors.clone();
    renderer.device().on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        seen.lock().unwrap().push(error.to_string());
    }));
    Some((harness_on(renderer), errors))
}

#[test]
fn a_gl33_device_draws_frames_without_a_validation_error() {
    if romacs_renderer_wgpu::gpu_policy::test_gpu::adapter().is_none() {
        eprintln!("SKIP: no GPU adapter");
        return;
    }
    let (mut h, errors) = gl33_harness().expect("a device within GL 3.3 limits");
    assert_eq!(h.renderer.device().limits().max_compute_workgroups_per_dimension, 0);
    assert_eq!(h.renderer.device().limits().max_storage_buffers_per_shader_stage, 0);
    renders_background_and_cursor(&mut h);
    inline_image_samples_only_its_source_slice(&mut h);
    assert_eq!(*errors.lock().unwrap(), Vec::<String>::new());
}
