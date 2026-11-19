use std::env;

fn add_sources(build: &mut cc::Build, root: &str, files: &[&str]) {
    let root = std::path::Path::new(root);
    build.files(files.iter().map(|src| {
        let mut p = root.join(src);
        p.set_extension("c");
        p
    }));

    build.include(root);
}

/// FreeType and libpng from the sources here, linked into the executable
/// and kept out of its dynamic symbols: the host's libfontconfig and
/// libharfbuzz load the host's libfreetype.so and libpng.so, and an
/// executable that exported our FT_* or png_* would capture their calls,
/// putting two versions on one set of objects (ROMACS.md).
fn main() {
    let out = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // From libpng/png.h: "If pnglibconf.h is missing, you can copy
    // scripts/pnglibconf.h.prebuilt to pnglibconf.h". The copy goes to
    // OUT_DIR: the sources may be read-only (a Nix build's store path).
    std::fs::create_dir_all(out.join("png")).unwrap();
    std::fs::copy("libpng/scripts/pnglibconf.h.prebuilt", out.join("png/pnglibconf.h")).unwrap();

    let mut build = cc::Build::new();

    build
        .warnings(false)
        .include(".")
        .include("freetype2/include")
        .include("libpng")
        .include(out.join("png"))
        .define("FT2_BUILD_LIBRARY", None)
        .define("FT_CONFIG_OPTION_USE_PNG", None)
        .define("FT_PUBLIC_FUNCTION_ATTRIBUTE", Some(""))
        .flag_if_supported("-fvisibility=hidden");

    add_sources(
        &mut build,
        "freetype2/src",
        &[
            "autofit/autofit",
            "base/ftbase",
            "base/ftbbox",
            "base/ftbdf",
            "base/ftbitmap",
            "base/ftcid",
            "base/ftdebug",
            "base/ftfstype",
            "base/ftgasp",
            "base/ftglyph",
            "base/ftgxval",
            "base/ftinit",
            "base/ftmm",
            "base/ftotval",
            "base/ftpatent",
            "base/ftpfr",
            "base/ftstroke",
            "base/ftsynth",
            "base/ftsystem",
            "base/fttype1",
            "base/ftwinfnt",
            "bdf/bdf",
            "bzip2/ftbzip2",
            "cache/ftcache",
            "cff/cff",
            "cid/type1cid",
            "gzip/ftgzip",
            "lzw/ftlzw",
            "pcf/pcf",
            "pfr/pfr",
            "psaux/psaux",
            "pshinter/pshinter",
            "psnames/psnames",
            "raster/raster",
            "sdf/sdf",
            "svg/svg",
            "sfnt/sfnt",
            "smooth/smooth",
            "truetype/truetype",
            "type1/type1",
            "type42/type42",
            "winfonts/winfnt",
        ],
    );

    build.compile("freetype2");

    println!("cargo:rustc-link-lib=z");

    let mut build = cc::Build::new();
    build
        .include("libpng")
        .include(out.join("png"))
        .flag_if_supported("-fvisibility=hidden");

    // The crates.io package does not contain the repository's libz-sys
    // submodule.  Use the include directory exported by the libz-sys build
    // script instead, which also follows whichever zlib source it selected.
    if let Ok(include_paths) = env::var("DEP_Z_INCLUDE") {
        for path in include_paths.split(',').filter(|path| !path.is_empty()) {
            build.include(path);
        }
    }

    // libz-sys builds a static zlib on MSVC.  Keep libpng's declarations in
    // sync so it does not request DLL-import symbols at link time.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        build.define("ZLIB_STATIC", None);
    }
    build
        .file("libpng/png.c")
        .file("libpng/pngerror.c")
        .file("libpng/pngget.c")
        .file("libpng/pngmem.c")
        .file("libpng/pngpread.c")
        .file("libpng/pngread.c")
        .file("libpng/pngrio.c")
        .file("libpng/pngrtran.c")
        .file("libpng/pngrutil.c")
        .file("libpng/pngset.c")
        .file("libpng/pngtrans.c")
        .file("libpng/pngwio.c")
        .file("libpng/pngwrite.c")
        .file("libpng/pngwtran.c")
        .file("libpng/pngwutil.c");

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    if arch == "arm" || arch == "aarch64" {
        // libpng's filter_neon.S is an obsolete compatibility stub.  Upstream
        // builds use the intrinsics sources, and MSVC cannot compile .S files.
        build
            .file("libpng/arm/arm_init.c")
            .file("libpng/arm/filter_neon_intrinsics.c")
            .file("libpng/arm/palette_neon_intrinsics.c");
    }

    build.compile("libpng.a");
}
