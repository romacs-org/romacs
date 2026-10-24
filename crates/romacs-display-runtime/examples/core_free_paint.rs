//! Native smoke probe, not the G1a text/colour acceptance oracle.
//! Run on an isolated compositor socket; never on the user's live desktop.
use romacs_display_protocol::{
    Color, DisplayFrameId, FrameGlyphBuffer, ParentFrameRect, PresentationId,
    PresentedFramePlacement,
};
use romacs_display_runtime::paint_host::{PRIMARY_FRAME, PaintHost};
use romacs_display_runtime::thread_comm::InputEvent;

fn main() -> Result<(), String> {
    let socket = std::env::args_os()
        .nth(1)
        .ok_or("expected private Wayland socket")?;
    let (host, sender) = PaintHost::wayland(
        std::path::Path::new(&socket),
        400,
        300,
        "core-free Seam P probe".into(),
    )?;
    let producer = std::thread::spawn(move || -> Result<(), String> {
        for (revision, background) in [(1, Color::RED), (2, Color::WHITE)] {
            let mut frame = FrameGlyphBuffer::with_size(400.0, 300.0);
            frame.presentation_id = PresentationId::new(revision);
            frame.frame_placement = PresentedFramePlacement::new(
                DisplayFrameId::new(PRIMARY_FRAME),
                frame.presentation_id,
                None,
                ParentFrameRect::new(0.0, 0.0, 400.0, 300.0).unwrap(),
                0,
            );
            frame.background = background;
            sender.submit(frame)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let timeout = deadline.saturating_duration_since(std::time::Instant::now());
                match sender
                    .input()
                    .recv_timeout(timeout)
                    .map_err(|error| error.to_string())?
                {
                    InputEvent::PresentationActivated { presentation, .. }
                        if presentation == revision =>
                    {
                        break;
                    }
                    InputEvent::WindowClose { .. } => return Err("native host closed early".into()),
                    _ => (),
                }
            }
            println!("activated {revision}");
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        sender.shutdown()
    });
    host.run().map_err(|error| error.to_string())?;
    producer.join().map_err(|_| "producer panicked")??;
    Ok(())
}
