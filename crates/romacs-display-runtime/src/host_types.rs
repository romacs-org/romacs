//! Owned native transport payloads, independent of evaluator values.

pub use independent::*;

mod independent {
    use romacs_display_protocol::{
        ImageEmbeddedMetadata, ImageLayoutExtent, ImageMaskKind, ImageReportedExtent,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FrontendKey {
        Character(char),
        Keysym(u32),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct GuiFrameGeometryHints {
        pub base_width: u32,
        pub base_height: u32,
        pub min_width: u32,
        pub min_height: u32,
        pub width_inc: u32,
        pub height_inc: u32,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct ResolvedImageMetadata {
        pub layout: ImageLayoutExtent,
        pub reported: ImageReportedExtent,
        pub background: u32,
        pub background_transparent: bool,
        pub mask: ImageMaskKind,
        pub embedded: ImageEmbeddedMetadata,
    }
    impl ResolvedImageMetadata {
        #[cfg(test)]
        pub const fn layout_is_image_pixels(
            width: u32,
            height: u32,
            background: u32,
            background_transparent: bool,
            mask: ImageMaskKind,
        ) -> Self {
            Self {
                layout: ImageLayoutExtent::new(width, height),
                reported: ImageReportedExtent::new(width, height),
                background,
                background_transparent,
                mask,
                embedded: ImageEmbeddedMetadata::EMPTY,
            }
        }
    }
}
