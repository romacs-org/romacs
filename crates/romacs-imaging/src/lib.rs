//! Romacs's CPU image decoder chain: header probes, the format decoders
//! (raster, banded, SVG, XPM, XBM), animation sequences and GNU's masks and
//! metadata.  The renderer uploads what it decodes; the GUI host decodes
//! off the evaluator with it; the attach helper prepares outgoing copies
//! with it, so what is shown is what is sent.

pub mod decode;
pub mod image_bands;
pub mod image_probe;
pub mod image_scale;
pub mod image_sequence;
pub mod svg;
pub mod svg_animation;
pub mod xbm;
pub mod xpm;

pub use image_bands::{BandPlacement, DecodedBand, RasterBand, RowRange, TextureRows};
pub use svg::SvgResourceContext;
