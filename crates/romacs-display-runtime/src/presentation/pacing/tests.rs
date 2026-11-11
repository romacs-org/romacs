use super::*;
#[test]
fn latest_frame_mode_is_selected_only_when_supported() {
    use wgpu::PresentMode::*;
    assert_eq!(
        select_present_mode(&[Fifo, Immediate, Mailbox], None),
        Mailbox
    );
    assert_eq!(select_present_mode(&[Fifo, Immediate], None), Fifo);
    assert_eq!(select_present_mode(&[Fifo], None), Fifo);
}

#[test]
fn benchmark_modes_are_explicit_and_fail_closed() {
    use wgpu::PresentMode::*;
    assert_eq!(select_present_mode(&[Fifo, Mailbox], Some("fifo")), Fifo);
    assert_eq!(
        select_present_mode(&[Fifo, Mailbox], Some("mailbox")),
        Mailbox
    );
    assert!(std::panic::catch_unwind(|| select_present_mode(&[Fifo], Some("mailbox"))).is_err());
    assert!(std::panic::catch_unwind(|| select_present_mode(&[Fifo], Some("typo"))).is_err());
}
