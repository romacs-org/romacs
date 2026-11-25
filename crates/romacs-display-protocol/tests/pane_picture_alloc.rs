//! `PanePicture::decode` and `PanePicture::into_frame` produce a frame from
//! the wire glyphs. Conversion must skip the normal frame's large glyph reserve.

use romacs_display_protocol::pane_picture::{PaneFrame, PaneHead, PanePicture, PANE_PICTURE_FORMAT};
use romacs_display_protocol::{Color, DisplayWindowId, FrameGlyph, FrameGlyphBuffer, Rect};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static LARGEST_ALLOCATION: Cell<Option<usize>> = const { Cell::new(None) };
}

struct TrackAllocations;

// UNSAFE(GlobalAlloc requires an unsafe impl)
// SAFETY: System allocates and deallocates every block. The counter only observes sizes.
unsafe impl GlobalAlloc for TrackAllocations {
    // UNSAFE(GlobalAlloc::alloc requires an unsafe method and allocator call)
    // SAFETY: the caller's layout is passed unchanged to System.alloc.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = LARGEST_ALLOCATION.try_with(|largest| {
            if let Some(size) = largest.get() {
                largest.set(Some(size.max(layout.size())));
            }
        });
        // UNSAFE(GlobalAlloc::alloc must forward to the system allocator)
        // SAFETY: the caller supplied a valid layout.
        unsafe { System.alloc(layout) }
    }

    // UNSAFE(GlobalAlloc::dealloc requires an unsafe method and allocator call)
    // SAFETY: the pointer and layout are passed unchanged to their allocator.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // UNSAFE(GlobalAlloc::dealloc must forward to the system allocator)
        // SAFETY: System allocated the block with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: TrackAllocations = TrackAllocations;

#[test]
fn a_decoded_small_pane_skips_the_large_frame_glyph_reserve() {
    let window = DisplayWindowId::new(1);
    let mut frame = FrameGlyphBuffer::with_size(32.0, 16.0);
    frame.glyphs.push(FrameGlyph::Background {
        bounds: Rect::new(0.0, 0.0, 32.0, 16.0),
        color: Color::BLACK,
    });
    let head = PaneHead { owner: 1, window: 1, view: 0, request: 0, v: 0, c: 0 };
    let picture = PanePicture::from_frame(
        head,
        PaneFrame { frame, animations: vec![], submitted: None },
        window,
    );
    let bytes = postcard::to_extend(&picture, PANE_PICTURE_FORMAT.to_le_bytes().to_vec()).unwrap();
    let decoded = PanePicture::decode(&bytes, 0).unwrap();
    LARGEST_ALLOCATION.set(Some(0));
    let frame = decoded.into_frame();
    let largest = LARGEST_ALLOCATION.take().unwrap();

    assert_eq!(frame.glyphs.len(), 1);
    assert!(frame.glyphs.capacity() <= frame.glyphs.len() * 2);
    // A capacity check alone misses a reserve that was allocated and discarded.
    assert!(largest <= 64 * std::mem::size_of::<FrameGlyph>(), "conversion allocated {largest} bytes");
}
