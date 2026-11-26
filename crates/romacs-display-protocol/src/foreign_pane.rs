//! A window the master shows and an owner lays out (design O1b.md rev 3,
//! §2.4): the root frame names it by an entry, and the render thread draws
//! the owner's pane picture for it in the entry's rect. The entry is what
//! a picture is checked against before it draws (§6.2).
use crate::pane_picture::whole_px;
use crate::types::Rect;

/// One owner-laid window of a root frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForeignPaneEntry {
    /// Where the pane draws, in the frame's logical px: the window's box less
    /// its dividers.
    pub rect: Rect,
    /// The master's window id.
    pub window: u64,
    /// The owner's incarnation.
    pub owner: u64,
    /// The master view generation the window shows.
    pub view: u64,
    /// The newest request revision sent for the window.
    pub request: u64,
    /// The master blinked the pane's cursor off.
    pub cursor_off: bool,
}

impl ForeignPaneEntry {
    /// The box a picture for this entry has: its rect's size in whole
    /// logical px, as `PanePicture::from_frame` sizes one.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (whole_px(self.rect.width), whole_px(self.rect.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entrys_size_is_its_rects_in_whole_px() {
        let entry = ForeignPaneEntry { rect: Rect::new(3.5, 4.0, 399.6, 100.0), window: 1, owner: 2, view: 3, request: 4, cursor_off: false };
        assert_eq!(entry.size(), (400, 100));
    }

    #[test]
    fn a_frame_has_no_foreign_panes_by_default() {
        assert!(crate::frame_glyphs::FrameGlyphBuffer::with_size(10.0, 10.0).foreign_panes.is_empty());
    }
}
