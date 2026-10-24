//! Display subpixel geometry independent of font discovery.

/// Physical color-component order used by LCD mask rasterization.
///
/// The historical name is retained for protocol compatibility, but the type
/// belongs to the display surface and is not a Fontconfig result on macOS or
/// Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontconfigSubpixelOrder {
    Unknown,
    None,
    Rgb,
    Bgr,
    VRgb,
    VBgr,
}

impl FontconfigSubpixelOrder {
    pub fn allows_horizontal_subpixel(self) -> bool {
        matches!(self, Self::Rgb | Self::Bgr | Self::Unknown)
    }
}

/// The display's subpixel order as Fontconfig reports it for `Monospace`.
///
/// Cached until [`invalidate_cache`]; the native catalog owner calls that on
/// a catalog change, as it does for every other Fontconfig answer.
pub fn default_subpixel_order() -> FontconfigSubpixelOrder {
    #[cfg(target_os = "linux")]
    {
        let cache = subpixel_cache();
        if let Some(order) = *cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            return order;
        }
        let order = query_default_subpixel_order();
        *cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(order);
        order
    }
    #[cfg(not(target_os = "linux"))]
    {
        FontconfigSubpixelOrder::Unknown
    }
}

/// Forget the cached order so the next query observes a new catalog.
pub fn invalidate_cache() {
    #[cfg(target_os = "linux")]
    {
        *subpixel_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

#[cfg(target_os = "linux")]
fn subpixel_cache() -> &'static std::sync::Mutex<Option<FontconfigSubpixelOrder>> {
    static CACHE: std::sync::Mutex<Option<FontconfigSubpixelOrder>> = std::sync::Mutex::new(None);
    &CACHE
}

#[cfg(target_os = "linux")]
fn query_default_subpixel_order() -> FontconfigSubpixelOrder {
    use fontconfig_sys::constants::{
        FC_RGBA, FC_RGBA_BGR, FC_RGBA_NONE, FC_RGBA_RGB, FC_RGBA_VBGR, FC_RGBA_VRGB,
    };
    let Some(fc) = crate::fontconfig_catalog::handle() else {
        return FontconfigSubpixelOrder::Unknown;
    };
    let _serial = crate::fontconfig_catalog::serialize();
    let Ok(mut pattern) = fontconfig::Pattern::new(fc) else {
        return FontconfigSubpixelOrder::Unknown;
    };
    if pattern
        .add_string(fontconfig::FC_FAMILY, c"Monospace")
        .is_err()
    {
        return FontconfigSubpixelOrder::Unknown;
    }
    let rgba = pattern
        .font_match()
        .ok()
        .and_then(|matched| matched.get_int(FC_RGBA).ok());
    match rgba {
        Some(FC_RGBA_NONE) => FontconfigSubpixelOrder::None,
        Some(FC_RGBA_RGB) => FontconfigSubpixelOrder::Rgb,
        Some(FC_RGBA_BGR) => FontconfigSubpixelOrder::Bgr,
        Some(FC_RGBA_VRGB) => FontconfigSubpixelOrder::VRgb,
        Some(FC_RGBA_VBGR) => FontconfigSubpixelOrder::VBgr,
        _ => FontconfigSubpixelOrder::Unknown,
    }
}
