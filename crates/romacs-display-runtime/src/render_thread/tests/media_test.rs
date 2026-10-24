use super::*;

#[cfg(feature = "video")]
#[test]
fn video_service_keeps_targets_independent_across_presenters() {
    let now = std::time::Instant::now();
    let fast = romacs_display_protocol::types::VideoId::new(1);
    let slow = romacs_display_protocol::types::VideoId::new(2);

    let request = video_service_request(
        now,
        [
            (fast, std::time::Duration::from_millis(16)),
            (slow, std::time::Duration::from_millis(16)),
            (fast, std::time::Duration::from_millis(8)),
        ],
    );

    assert_eq!(
        request.timing_for(fast).target_presentation_time(),
        now + std::time::Duration::from_millis(8)
    );
    assert_eq!(
        request.timing_for(slow).target_presentation_time(),
        now + std::time::Duration::from_millis(16)
    );
}
