// The WPE WebKit adapter with the `webview` feature, else a backend that
// reports the feature missing.
#[cfg(feature = "webview")]
mod linux;
#[cfg(feature = "webview")]
pub(crate) use linux::LinuxPlatform as CurrentPlatform;
#[cfg(not(feature = "webview"))]
mod unsupported;
#[cfg(not(feature = "webview"))]
pub(crate) use unsupported::UnsupportedPlatform as CurrentPlatform;
