use romacs_display_protocol::{Color, NativeChromeSupport, UnsupportedChrome, WindowChromePolicy};
use winit::window::{Window, WindowAttributes};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ChromeError {
    #[error("unsupported native chrome: {0:?}")]
    Unsupported(UnsupportedChrome),
}

pub(super) fn prepare(attrs: WindowAttributes, policy: WindowChromePolicy) -> WindowAttributes {
    let _ = policy;
    attrs
}

pub(super) fn apply(
    window: &dyn Window,
    policy: WindowChromePolicy,
    color: Color,
) -> Result<(), ChromeError> {
    policy
        .resolve(NativeChromeSupport::SystemOnly)
        .map_err(ChromeError::Unsupported)?;
    let _ = (window, color);
    Ok(())
}
