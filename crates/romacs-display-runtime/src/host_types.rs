//! Owned native transport payloads, independent of evaluator values.

pub use independent::*;

mod independent {
    use romacs_display_protocol::{
        EncodedBytes, ImageEmbeddedMetadata, ImageLayoutExtent, ImageMaskKind, ImageReportedExtent,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FrontendKey {
        Character(char),
        Keysym(u32),
    }

    pub fn native_key_macos(key: u16) -> u32 {
        0x2000_0000 | u32::from(key)
    }
    pub fn native_key_windows(key: u16) -> u32 {
        0x2100_0000 | u32::from(key)
    }
    pub fn native_key_android(key: u32) -> u32 {
        0x2200_0000 | (key & 0xffff)
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct GuiFrameGeometryHints {
        pub base_width: u32,
        pub base_height: u32,
        pub min_width: u32,
        pub min_height: u32,
        pub width_inc: u32,
        pub height_inc: u32,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub struct ImageBaseUri(pub String);
    impl ImageBaseUri {
        pub fn as_utf8_str(&self) -> Option<&str> {
            Some(&self.0)
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub enum ImageDataSource {
        Isolated(EncodedBytes),
        WithBaseUri {
            data: EncodedBytes,
            base_uri: ImageBaseUri,
        },
    }
    impl ImageDataSource {
        pub fn bytes(&self) -> &[u8] {
            match self {
                Self::Isolated(data) | Self::WithBaseUri { data, .. } => data,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ResolvedImageMetadata {
        pub layout: ImageLayoutExtent,
        pub reported: ImageReportedExtent,
        pub background: u32,
        pub background_transparent: bool,
        pub mask: ImageMaskKind,
        pub embedded: ImageEmbeddedMetadata,
    }
    impl ResolvedImageMetadata {
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
