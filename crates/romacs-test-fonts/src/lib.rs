//! Pinned upstream font assets for tests, read from a local cache.
//!
//! Tests never touch the network. `fixtures.json` pins each asset by SHA-256
//! and source; `scripts/fetch-test-fixtures` downloads and verifies them into
//! the cache, `ROMACS_TEST_FONTS_DIR` or `~/.cache/romacs-test-fonts`. A test
//! asking for an asset the cache lacks panics and names that script.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Runtime workspace root: nextest's NEXTEST_WORKSPACE_ROOT when present,
/// the compile-time constant otherwise (see romacs-infra).
pub fn workspace_root() -> PathBuf {
    std::env::var_os("NEXTEST_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_WORKSPACE_DIR")))
}

/// The cache the fetch script fills: OVERRIDE, else HOME's
/// `.cache/romacs-test-fonts`.
fn cache_root_from(override_dir: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    override_dir.or_else(|| home.map(|home| home.join(".cache/romacs-test-fonts")))
}

fn cache_root() -> PathBuf {
    cache_root_from(
        std::env::var_os("ROMACS_TEST_FONTS_DIR").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
    .expect("test fixtures need ROMACS_TEST_FONTS_DIR or HOME")
}

fn missing(path: &Path) -> String {
    format!(
        "test fixture {} is missing; run scripts/fetch-test-fixtures (tests never download)",
        path.display()
    )
}

/// FILE in the cache, which `fixtures.json` lists.
///
/// # Panics
/// When the cache lacks FILE: the fetch script has not run for this cache.
fn cached(cell: &'static OnceLock<PathBuf>, file: &str) -> &'static Path {
    cell.get_or_init(|| {
        let path = cache_root().join(file);
        assert!(path.is_file(), "{}", missing(&path));
        path
    })
}

macro_rules! fixtures {
    ($($(#[$doc:meta])* $name:ident => $file:literal;)*) => {
        $(
            $(#[$doc])*
            #[must_use]
            pub fn $name() -> &'static Path {
                static PATH: OnceLock<PathBuf> = OnceLock::new();
                cached(&PATH, $file)
            }
        )*
        /// Every file the functions above name, for the manifest check.
        #[cfg(test)]
        const FILES: &[&str] = &[$($file),*];
    };
}

fixtures! {
    /// The W3C decoder-suite WOFF2 collection, for nonzero collection
    /// selectors surviving decoding to standalone SFNT.
    woff2_collection => "w3c-roundtrip-collection-order-001.woff2";
    /// The bitmap-only color SFNT, for exact platform selection retaining
    /// Swash's scalable color-bitmap replay path.
    noto_color_emoji_2_051 => "noto-color-emoji-2.051.ttf";
    /// The COLRv1 build of Noto Color Emoji. Its emoji are COLR version 1
    /// paint graphs over empty `glyf` outlines, so it draws nothing unless
    /// the rasterizer traverses `BaseGlyphList`, and the monochrome-outline
    /// fallback has nothing to show.
    noto_color_emoji_colrv1 => "noto-color-emoji-colrv1.ttf";
    /// Nabla, a variable COLRv1 display face (axes `EDPT` and `EHLT`): its
    /// paint graph follows the resolved instance, and it carries an `SVG `
    /// table too.
    nabla_color_colrv1 => "nabla.ttf";
    /// The static M PLUS 1 Code Thin face, for nearest-weight matching
    /// without an installed Regular sibling.
    mplus_1_code_thin => "MPLUS1Code-Thin.ttf";
    /// LXGW WenKai Nerd Font Regular 1.522, the reporter's exact family.
    lxgw_wenkai_nerd_regular_1_522 => "lxgw-wenkai-nerd-1.522/LXGWWenKaiNerdFont-Regular.ttf";
    /// Nerd Fonts v3.4.0 `Symbols Nerd Font Mono`, whose cmap has no ASCII,
    /// not even U+0020.
    symbols_nerd_font_mono_3_4_0 => "SymbolsNerdFontMono-Regular-v3.4.0.ttf";
    /// Nerd Fonts v3.4.0 `JetBrainsMono Nerd Font` Regular: a text monospace
    /// face that also carries the Nerd Font icons.
    jetbrains_mono_nerd_font_regular_3_4_0 => "JetBrainsMonoNerdFont-Regular-v3.4.0.ttf";
    /// Variable `Noto Sans Mono` (`wdth`/`wght` axes) with fvar named
    /// instances from Thin 100 to Black 900, including SemiBold 600.
    noto_sans_mono_variable => "NotoSansMono-wdth-wght.ttf";
    /// The reported dual-width fixed-pitch font from issue #516, v3.1.0.
    plemol_jp_console_nf_regular => "PlemolJPConsoleNF-Regular-v3.1.0.ttf";
    /// nyan-mode's `nyan.xpm`, whose palette is a ladder of `grayNN` names.
    nyan_mode_xpm => "nyan-mode-nyan.xpm";
}

/// Spleen 2.2.0's 8x16 face, in each format it ships, and the PCF gzipped.
#[derive(Clone, Debug)]
pub struct SpleenFixtures {
    root: PathBuf,
}

impl SpleenFixtures {
    /// The face's files, in the order of the methods below.
    const FILES: [&str; 6] = [
        "spleen-8x16.bdf",
        "spleen-8x16.pcf",
        "spleen-8x16.pcf.gz",
        "spleen-8x16.otb",
        "spleen-8x16.woff",
        "spleen-8x16.woff2",
    ];

    #[must_use]
    pub fn bdf(&self) -> PathBuf {
        self.root.join(Self::FILES[0])
    }
    #[must_use]
    pub fn pcf(&self) -> PathBuf {
        self.root.join(Self::FILES[1])
    }
    #[must_use]
    pub fn pcf_gz(&self) -> PathBuf {
        self.root.join(Self::FILES[2])
    }
    #[must_use]
    pub fn otb(&self) -> PathBuf {
        self.root.join(Self::FILES[3])
    }
    #[must_use]
    pub fn woff(&self) -> PathBuf {
        self.root.join(Self::FILES[4])
    }
    #[must_use]
    pub fn woff2(&self) -> PathBuf {
        self.root.join(Self::FILES[5])
    }
}

/// The Spleen fixtures.
///
/// # Panics
/// When the cache lacks one of them (see [`cached`]).
#[must_use]
pub fn spleen_2_2_0() -> &'static SpleenFixtures {
    static SPLEEN: OnceLock<SpleenFixtures> = OnceLock::new();
    SPLEEN.get_or_init(|| {
        let root = cache_root().join("spleen-2.2.0");
        for file in SpleenFixtures::FILES {
            let path = root.join(file);
            assert!(path.is_file(), "{}", missing(&path));
        }
        SpleenFixtures { root }
    })
}

/// The Iosevka files Romacs ships (assets/fonts/sources.json): Regular,
/// Bold, Italic and Bold Italic, in that order. They are in the checkout.
#[must_use]
pub fn bundled_iosevka() -> [PathBuf; 4] {
    ["Regular", "Bold", "Italic", "BoldItalic"]
        .map(|style| workspace_root().join(format!("assets/fonts/Iosevka-{style}.ttf")))
}

/// A Fontconfig configuration under ROOT whose only font directories are
/// FONT_DIRS (none: only the fonts a test registers), with its cache under
/// ROOT too; its path, for FONTCONFIG_FILE, so no host font takes part.
pub fn isolated_fontconfig(root: &Path, font_dirs: &[&Path]) -> io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    let dirs: String = font_dirs.iter().map(|dir| format!("<dir>{}</dir>", dir.display())).collect();
    let config = root.join("fonts.conf");
    fs::write(&config, format!("<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
        <fontconfig>{dirs}<cachedir>{}</cachedir></fontconfig>\n", root.join("cache").display()))?;
    Ok(config)
}

#[cfg(test)]
#[path = "tests/lib_test.rs"]
mod tests;
