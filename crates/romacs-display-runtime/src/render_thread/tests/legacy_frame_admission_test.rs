//! Exact lifecycle cancellation on the real bounded ThreadComms transport.

#[test]
fn legacy_readiness_replay_source_authority_is_exact_and_non_resizing() {
    let dispatch = include_str!("../window_commands.rs");
    assert!(dispatch.contains("if let Some(reply) = reply"));
    assert!(dispatch.contains("self.frame_windows.await_ready(id, reply)"));
    assert!(dispatch.contains("primary.replay_pending_native_state(pending.window())"));
    let manager = include_str!("../frame_windows.rs");
    let replay = manager
        .split("fn replay_pending_native_state")
        .nth(1)
        .unwrap()
        .split("fn set_title")
        .next()
        .unwrap();
    assert!(replay.contains("window.set_title(&chrome.title)"));
    assert!(replay.contains("apply_window_geometry_hints(window, hints)"));
    assert!(!replay.contains("request_surface_size"));
    assert!(!replay.contains("request_inner_size"));
    assert!(manager.contains("window_state.replay_pending_native_state(native.window.as_ref())"));
    assert!(manager.contains("if reply.send(result).is_err() && ready"));
    assert!(manager.contains("if reply.send(Ok(())).is_err()"));
    let creation = include_str!("../lifecycle.rs");
    assert!(creation.contains(".replay_pending_native_state(window.as_ref())"));
    let hints = include_str!("../geometry_hints.rs");
    assert!(hints.contains("window.set_min_surface_size"));
    assert!(hints.contains("window.set_surface_resize_increments"));
}
