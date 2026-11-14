use super::*;
use crate::thread_comm::{AssetCommand, FrameRef, WindowCommand};
use romacs_display_protocol::ImageId;

const IMAGE_0: ImageId = ImageId::new(0);
const IMAGE_7: ImageId = ImageId::new(7);
const IMAGE_64: ImageId = ImageId::new(64);

fn asset(id: u32) -> RenderCommand {
    RenderCommand::Asset(AssetCommand::ImageRetire {
        image: ImageId::new(id),
    })
}
fn asset_id(command: RenderCommand) -> ImageId {
    let RenderCommand::Asset(AssetCommand::ImageRetire { image }) = command else {
        panic!("not the queued asset")
    };
    image
}

#[test]
fn selective_startup_preserves_capacity_and_fifo_for_every_queued_asset() {
    let (tx, rx) = command_channel(64);
    tx.try_send(asset(1)).unwrap();
    tx.try_send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }))
        .unwrap();
    tx.try_send(asset(2)).unwrap();
    tx.try_send(RenderCommand::Window(WindowCommand::DestroyWindow {
        frame: FrameRef::Frame(42),
    }))
    .unwrap();
    for id in 3..=62 {
        tx.try_send(asset(id)).unwrap();
    }
    assert_eq!(tx.len(), 64);
    assert!(matches!(tx.try_send(asset(63)), Err(TrySendError::Full(_))));
    assert!(matches!(
        rx.try_recv_startup().unwrap(),
        RenderCommand::Window(WindowCommand::RequestAttention { urgent: false })
    ));
    assert!(matches!(
        rx.try_recv_startup().unwrap(),
        RenderCommand::Window(WindowCommand::DestroyWindow {
            frame: FrameRef::Frame(42)
        })
    ));
    assert!(matches!(rx.try_recv_startup(), Err(TryRecvError::Empty)));
    // Startup extraction frees only the consumed slots, never more storage.
    tx.try_send(asset(63)).unwrap();
    tx.try_send(asset(64)).unwrap();
    assert_eq!(rx.len(), 64);
    assert!(matches!(tx.try_send(asset(65)), Err(TrySendError::Full(_))));
    let ids: Vec<_> = rx.try_iter().map(asset_id).collect();
    assert_eq!(ids, (1..=64).map(ImageId::new).collect::<Vec<_>>());
}

#[test]
fn ordinary_receive_prioritizes_shutdown_without_reordering_assets() {
    let (tx, rx) = command_channel(4);
    tx.send(asset(1)).unwrap();
    tx.send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown))
        .unwrap();
    tx.send(asset(2)).unwrap();
    assert!(matches!(
        rx.recv().unwrap(),
        RenderCommand::Lifecycle(LifecycleCommand::Shutdown)
    ));
    assert_eq!(asset_id(rx.recv().unwrap()), ImageId::new(1));
    assert_eq!(asset_id(rx.recv().unwrap()), ImageId::new(2));
}

#[test]
fn selective_consumption_unblocks_a_backpressured_sender_without_asset_loss() {
    let (tx, rx) = command_channel(2);
    tx.send(asset(1)).unwrap();
    tx.send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }))
        .unwrap();
    let (done, result) = bounded(1);
    let worker = std::thread::spawn(move || {
        done.send(tx.send(asset(2))).unwrap();
    });
    assert!(matches!(
        rx.try_recv_startup().unwrap(),
        RenderCommand::Window(WindowCommand::RequestAttention { urgent: false })
    ));
    assert!(result.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    assert_eq!(asset_id(rx.recv().unwrap()), ImageId::new(1));
    assert_eq!(asset_id(rx.recv().unwrap()), ImageId::new(2));
    worker.join().unwrap();
    assert!(matches!(rx.recv(), Err(RecvError)));
}

#[test]
fn receiver_drop_releases_blocked_sender_and_returns_its_exact_command() {
    let (tx, rx) = command_channel(1);
    tx.send(asset(1)).unwrap();
    let (done, result) = bounded(1);
    let worker = std::thread::spawn(move || {
        done.send(tx.send(asset(2))).unwrap();
    });
    drop(rx);
    let Err(SendError(command)) = result.recv_timeout(Duration::from_secs(2)).unwrap() else {
        panic!("disconnected sender accepted a command")
    };
    assert_eq!(asset_id(command), ImageId::new(2));
    worker.join().unwrap();
}

#[test]
fn sender_clones_disconnect_only_after_last_drop_and_pending_commands_are_drained() {
    let (tx, rx) = command_channel(2);
    let other = tx.clone();
    tx.send(asset(1)).unwrap();
    drop(tx);
    assert_eq!(asset_id(rx.recv().unwrap()), ImageId::new(1));
    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(5)),
        Err(RecvTimeoutError::Timeout)
    ));
    other.send(asset(2)).unwrap();
    drop(other);
    assert_eq!(
        asset_id(rx.recv_timeout(Duration::from_secs(1)).unwrap()),
        ImageId::new(2)
    );
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
}

#[test]
fn tty_select_wake_is_rearmed_for_pending_work_and_stale_wakes_are_not_disconnects() {
    let (tx, rx) = command_channel(2);
    tx.send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }))
        .unwrap();
    tx.send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown))
        .unwrap();
    rx.available().recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        rx.try_recv().unwrap(),
        RenderCommand::Lifecycle(LifecycleCommand::Shutdown)
    ));
    rx.available().recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(rx.try_recv().unwrap(), RenderCommand::Window(_)));
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    tx.send(asset(1)).unwrap();
    assert_eq!(asset_id(rx.try_recv().unwrap()), ImageId::new(1));
    rx.available().recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    drop(tx);
    assert!(rx.available().recv_timeout(Duration::from_secs(1)).is_err());
}

#[test]
fn shutdown_coalesces_outside_full_fifo_and_preserves_rejected_work() {
    let (sender, receiver) = command_channel(crate::thread_comm::COMMAND_CHANNEL_CAPACITY);
    for id in 0..crate::thread_comm::COMMAND_CHANNEL_CAPACITY as u32 {
        sender
            .try_send(RenderCommand::Asset(AssetCommand::ImageRetire {
                image: ImageId::new(id),
            }))
            .unwrap();
    }
    assert!(matches!(
        sender.try_send(RenderCommand::Asset(AssetCommand::ImageRetire {
            image: IMAGE_64
        })),
        Err(TrySendError::Full(RenderCommand::Asset(
            AssetCommand::ImageRetire { image: IMAGE_64 }
        )))
    ));
    for _ in 0..1000 {
        sender
            .try_send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown))
            .unwrap();
    }
    assert!(receiver.take_shutdown());
    assert!(!receiver.take_shutdown());
    for id in 0..crate::thread_comm::COMMAND_CHANNEL_CAPACITY as u32 {
        assert!(
            matches!(receiver.recv().unwrap(), RenderCommand::Asset(AssetCommand::ImageRetire { image: actual }) if actual == ImageId::new(id))
        );
    }
    assert!(receiver.is_empty());
}

#[test]
fn cloned_producers_keep_stream_alive_and_disconnect_returns_ownership() {
    let (sender, receiver) = command_channel(crate::thread_comm::COMMAND_CHANNEL_CAPACITY);
    let other = sender.clone();
    drop(sender);
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    other
        .send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }))
        .unwrap();
    drop(other);
    assert!(matches!(
        receiver.recv().unwrap(),
        RenderCommand::Window(WindowCommand::RequestAttention { urgent: false })
    ));
    assert!(matches!(
        receiver.try_recv(),
        Err(TryRecvError::Disconnected)
    ));
    let (sender, receiver) = command_channel(crate::thread_comm::COMMAND_CHANNEL_CAPACITY);
    drop(receiver);
    assert!(matches!(
        sender.try_send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown)),
        Err(TrySendError::Disconnected(RenderCommand::Lifecycle(
            LifecycleCommand::Shutdown
        )))
    ));
    assert!(matches!(
        sender.send(RenderCommand::Asset(AssetCommand::ImageRetire {
            image: IMAGE_7
        })),
        Err(SendError(RenderCommand::Asset(AssetCommand::ImageRetire {
            image: IMAGE_7
        })))
    ));
}

#[test]
fn blocking_sender_resumes_on_space_or_owner_disconnect() {
    for disconnect in [false, true] {
        let (sender, receiver) = command_channel(crate::thread_comm::COMMAND_CHANNEL_CAPACITY);
        for id in 0..crate::thread_comm::COMMAND_CHANNEL_CAPACITY as u32 {
            sender
                .try_send(RenderCommand::Asset(AssetCommand::ImageRetire {
                    image: ImageId::new(id),
                }))
                .unwrap();
        }
        let (started, start) = bounded(1);
        let (done, result) = bounded(1);
        let worker = std::thread::spawn(move || {
            started.send(()).unwrap();
            done.send(sender.send(RenderCommand::Asset(AssetCommand::ImageRetire {
                image: IMAGE_64,
            })))
            .unwrap();
        });
        start.recv().unwrap();
        assert!(matches!(result.try_recv(), Err(TryRecvError::Empty)));
        if disconnect {
            drop(receiver);
            assert!(matches!(
                result
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap(),
                Err(SendError(RenderCommand::Asset(AssetCommand::ImageRetire {
                    image: IMAGE_64
                })))
            ));
        } else {
            assert!(matches!(
                receiver.try_recv().unwrap(),
                RenderCommand::Asset(AssetCommand::ImageRetire { image: IMAGE_0 })
            ));
            assert!(
                result
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap()
                    .is_ok()
            );
            for id in 1..=crate::thread_comm::COMMAND_CHANNEL_CAPACITY as u32 {
                assert!(
                    matches!(receiver.try_recv().unwrap(), RenderCommand::Asset(AssetCommand::ImageRetire { image: actual }) if actual == ImageId::new(id))
                );
            }
            assert!(receiver.is_empty());
        }
        worker.join().unwrap();
    }
}

#[test]
fn independent_shutdown_retains_len_timeout_fifo_and_last_sender_disconnect() {
    let (tx, rx) = command_channel(2);
    tx.send(asset(1)).unwrap();
    tx.send(asset(2)).unwrap();
    for _ in 0..1000 {
        tx.send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown))
            .unwrap();
    }
    assert_eq!(tx.len(), 3);
    assert_eq!(rx.len(), 3);
    assert!(matches!(tx.try_send(asset(3)), Err(TrySendError::Full(_))));
    rx.available().recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::ZERO).unwrap(),
        RenderCommand::Lifecycle(LifecycleCommand::Shutdown)
    ));
    assert_eq!(tx.len(), 2);
    rx.available().recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(asset_id(rx.try_recv().unwrap()), ImageId::new(1));
    rx.available().recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(asset_id(rx.try_recv().unwrap()), ImageId::new(2));
    assert_eq!(tx.len(), 0);
    assert!(rx.is_empty());
    assert!(matches!(
        rx.recv_timeout(Duration::ZERO),
        Err(RecvTimeoutError::Timeout)
    ));
    tx.send(asset(3)).unwrap();
    tx.send(RenderCommand::Lifecycle(LifecycleCommand::Shutdown))
        .unwrap();
    drop(tx);
    assert!(matches!(
        rx.recv_timeout(Duration::ZERO).unwrap(),
        RenderCommand::Lifecycle(LifecycleCommand::Shutdown)
    ));
    assert_eq!(
        rx.try_iter().map(asset_id).collect::<Vec<_>>(),
        vec![ImageId::new(3)]
    );
    assert!(matches!(
        rx.recv_timeout(Duration::ZERO),
        Err(RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn unserviceable_startup_backlog_is_not_disconnected_or_spuriously_pollable() {
    let (tx, rx) = command_channel(2);
    tx.send(asset(1)).unwrap();
    tx.send(RenderCommand::Window(WindowCommand::RequestAttention { urgent: false }))
        .unwrap();
    assert!(!rx.has_startup_command_for(false));
    assert!(rx.has_startup_command_for(true));
    assert!(matches!(
        rx.try_recv_startup_for(true, false),
        Err(TryRecvError::Empty)
    ));
    drop(tx);
    assert!(matches!(
        rx.try_recv_startup_for(true, false),
        Err(TryRecvError::Empty)
    ));
    assert!(matches!(
        rx.try_recv_startup_for(true, true).unwrap(),
        RenderCommand::Window(WindowCommand::RequestAttention { urgent: false })
    ));
    assert!(!rx.has_startup_command_for(true));
    assert!(matches!(
        rx.try_recv_startup_for(true, true),
        Err(TryRecvError::Empty)
    ));
    assert_eq!(asset_id(rx.try_recv().unwrap()), ImageId::new(1));
    assert!(matches!(
        rx.try_recv_startup_for(true, false),
        Err(TryRecvError::Disconnected)
    ));
}

/// A producer held within a limit is refused at it while the channel still
/// takes everyone else's commands, up to its capacity.
#[test]
fn a_limited_producer_leaves_the_rest_of_the_channel() {
    let capacity = crate::thread_comm::COMMAND_CHANNEL_CAPACITY;
    let room = crate::thread_comm::MODEL_REQUEST_ROOM;
    assert!(room < capacity / 2, "a publication's commands fit beside the requests");
    let (sender, receiver) = command_channel(capacity);
    let retire = |id| RenderCommand::Asset(AssetCommand::ImageRetire { image: ImageId::new(id) });
    let (room_ids, capacity_ids) = (u32::try_from(room).unwrap(), u32::try_from(capacity).unwrap());
    for id in 0..room_ids {
        sender.try_send_within(retire(id), room).unwrap();
    }
    assert!(matches!(sender.try_send_within(retire(999), room), Err(TrySendError::Full(_))));
    for id in room_ids..capacity_ids {
        sender.try_send(retire(id)).unwrap();
    }
    assert!(matches!(sender.try_send(retire(999)), Err(TrySendError::Full(_))));
    assert_eq!(receiver.len(), capacity);
}

/// Paints are a mailbox: a newer paint replaces one the renderer has not
/// taken, behind whatever was sent between them, so the producer never
/// waits for drawing and the renderer draws the newest frame.
#[test]
fn a_newer_paint_supersedes_a_queued_one_behind_the_commands_between() {
    let paint = |width| RenderCommand::Paint(Box::new(romacs_display_protocol::FrameGlyphBuffer::with_size(width, 10.0).into()));
    let width = |command: RenderCommand| match command {
        RenderCommand::Paint(scene) => scene.root.width,
        _ => panic!("not a paint"),
    };
    let (sender, receiver) = command_channel(4);
    sender.try_send(paint(100.0)).unwrap();
    sender.try_send(asset(7)).unwrap();
    sender.try_send(paint(200.0)).unwrap();
    sender.try_send(paint(300.0)).unwrap();
    assert_eq!(receiver.len(), 2, "one paint waits, beside the asset");
    assert_eq!(asset_id(receiver.try_recv().unwrap()), IMAGE_7);
    assert_eq!(width(receiver.try_recv().unwrap()).to_bits(), 300.0_f32.to_bits());
    // A full channel still takes a paint when one waits to be replaced.
    for id in 0..3 { sender.try_send(asset(id)).unwrap(); }
    sender.try_send(paint(400.0)).unwrap();
    assert!(sender.try_send(paint(500.0)).is_ok(), "a waiting paint makes room for its successor");
    assert_eq!(receiver.len(), 4);
}

/// A full channel still takes a frame's opacity setting and a window's
/// destruction, in order behind what waits: their producers never block on
/// a render thread that may be waiting on them.
#[test]
fn a_full_channel_takes_opacity_and_destruction_in_order() {
    let (tx, rx) = command_channel(2);
    tx.try_send(asset(1)).unwrap();
    tx.try_send(asset(2)).unwrap();
    assert!(matches!(tx.try_send(asset(3)), Err(TrySendError::Full(_))));
    tx.try_send(RenderCommand::Window(WindowCommand::SetFrameAlpha { frame: 1, alpha: [0.5, 0.5], limit: 0.0 })).unwrap();
    tx.try_send(RenderCommand::Window(WindowCommand::DestroyWindow { frame: FrameRef::Frame(42) })).unwrap();
    let order: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).map(|command| match command {
        RenderCommand::Asset(_) => "asset",
        RenderCommand::Window(WindowCommand::SetFrameAlpha { .. }) => "alpha",
        RenderCommand::Window(WindowCommand::DestroyWindow { .. }) => "destroy",
        _ => "other",
    }).collect();
    assert_eq!(order, ["asset", "asset", "alpha", "destroy"]);
}

/// However fast Lisp sets a frame's alpha, at most the setters that decide
/// its state wait: the last, and the last to set each alpha; and a frame's
/// destruction waits once.
#[test]
fn exempt_commands_stay_bounded_and_keep_their_effect() {
    let (tx, rx) = command_channel(2);
    let alpha = |frame, alpha: [f32; 2]| RenderCommand::Window(WindowCommand::SetFrameAlpha { frame, alpha, limit: 0.0 });
    for step in 0..100 { tx.try_send(alpha(1, [step as f32 / 100.0, 0.5])).unwrap(); }
    tx.try_send(alpha(2, [0.3, 0.3])).unwrap();
    tx.try_send(alpha(1, [-1.0, 0.7])).unwrap();
    for _ in 0..10 {
        tx.try_send(RenderCommand::Window(WindowCommand::DestroyWindow { frame: FrameRef::Frame(9) })).unwrap();
    }
    let queued: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).map(|command| match command {
        RenderCommand::Window(WindowCommand::SetFrameAlpha { frame, alpha, .. }) => format!("{frame}:{:?}", alpha),
        RenderCommand::Window(WindowCommand::DestroyWindow { .. }) => "destroy".to_owned(),
        _ => "other".to_owned(),
    }).collect();
    // Frame 1: the last setter of its active alpha (0.99), then the last,
    // which sets inactive and keeps the active scalar 0.99 applied.
    assert_eq!(queued, ["1:[0.99, 0.5]", "2:[0.3, 0.3]", "1:[-1.0, 0.7]", "destroy"]);
}

/// The mailbox is per top-level frame: a paint replaces an unread paint of
/// its own frame only, never another frame's.
#[test]
fn a_paint_replaces_only_its_own_frames_unread_paint() {
    use romacs_display_protocol::{DisplayFrameId, FrameGlyphBuffer, types::Color};
    let paint = |frame: u64, width: f32| {
        let mut buffer = FrameGlyphBuffer::with_size(width, 10.0);
        buffer.set_frame_identity(DisplayFrameId::new(frame), DisplayFrameId::new(0), 0.0, 0.0, 0, false, 0.0, Color::BLACK, false, 1.0);
        RenderCommand::Paint(Box::new(buffer.into()))
    };
    let drawn = |command: RenderCommand| match command {
        RenderCommand::Paint(scene) => (scene.root.frame_placement.frame().get(), scene.root.width as u32),
        _ => panic!("not a paint"),
    };
    let (sender, receiver) = command_channel(8);
    sender.try_send(paint(1, 100.0)).unwrap();
    sender.try_send(paint(2, 200.0)).unwrap();
    sender.try_send(paint(1, 110.0)).unwrap();
    sender.try_send(paint(2, 210.0)).unwrap();
    let order: Vec<_> = std::iter::from_fn(|| receiver.try_recv().ok()).map(drawn).collect();
    assert_eq!(order, [(1, 110), (2, 210)], "each frame's newest, in the order sent");
}
