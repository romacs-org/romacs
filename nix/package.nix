{
  crane,
  pkgs,
  rustToolchain,
  source,
  version,
  wpeWebkit,
}:
let
  inherit (pkgs) lib;
  craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
  # Preserve the COMPLETE authenticated local provider, not only *.rs/*.toml.
  providerSource = source + "/local-deps/rio-vt-0.5.5";
  providerPrefix = toString providerSource;
  cargoSrc = lib.cleanSourceWith {
    src = source;
    name = "romacs-cargo-source";
    filter = path: type:
      let p = toString path; in
      p == providerPrefix
      || lib.hasPrefix (providerPrefix + "/") p
      || craneLib.filterCargoSources path type;
  };
  # VM2 has no legacy video/webview feature forwarding or dump pipeline.
  productionCapabilities = {
    cargoFeatures = [ ];
    videoBackend = "none";
  };
  dependencies = import ./dependencies.nix {
    inherit lib pkgs wpeWebkit;
  };
  cargoPackages = [
    "-p"
    "romacs-vm-gui"
    "-p"
    "romacs-vm"
    "-p"
    "romacs-mcp"
  ];
  cargoBuildArgs = lib.concatStringsSep " " ([ "--locked" ] ++ cargoPackages);
  runtimeLibs = dependencies.productionBuildInputs productionCapabilities;
  videoEnabled = builtins.elem "video" productionCapabilities.cargoFeatures;
  gstreamerRuntime = import ./gstreamer-runtime.nix {
    inherit lib pkgs;
    pluginInputs = dependencies.videoPluginInputs;
  };
  commonArgs = {
    pname = "romacs";
    inherit version;
    src = cargoSrc;
    strictDeps = true;
    cargoExtraArgs = cargoBuildArgs;
    nativeBuildInputs = [
      rustToolchain
      pkgs.binutils
      pkgs.pkg-config
      pkgs.llvmPackages.clang
      pkgs.makeWrapper
    ];
    buildInputs = runtimeLibs;
    doCheck = false;
    preConfigure = ''
      diff -r ${providerSource} local-deps/rio-vt-0.5.5
    '';
  };
  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      # Keep dependency artifacts stable across commits. Let buildDepsOnly
      # synthesize its dummy source to avoid import-from-derivation.
      version = "0.0.0";
      # Crane stubs local path crates. Restore ONLY this provider after stubbing;
      # workspace crates stay dummy so dependency preparation remains narrow.
      extraDummyScript = ''
        rm -rf "$out/local-deps/rio-vt-0.5.5"
        mkdir -p "$out/local-deps"
        cp -r ${providerSource} "$out/local-deps/rio-vt-0.5.5"
      '';
    }
  );
  linuxWrapArgs =
    lib.optionals pkgs.stdenv.hostPlatform.isLinux [
      "--set-default"
      "VK_DRIVER_FILES"
      "$(echo ${pkgs.mesa}/share/vulkan/icd.d/*.json | tr ' ' ':')"
    ]
    ++ lib.optionals (pkgs.stdenv.hostPlatform.isLinux && videoEnabled) [
      "--set-default"
      "GST_PLUGIN_SYSTEM_PATH_1_0"
      gstreamerRuntime.pluginSystemPath
      "--set-default"
      "GST_PLUGIN_SCANNER_1_0"
      gstreamerRuntime.pluginScanner
    ]
    ++
      lib.optionals (pkgs.stdenv.hostPlatform.isLinux && builtins.elem "webview" productionCapabilities.cargoFeatures)
        [
          "--set-default"
          "WPE_BACKEND_LIBRARY"
          "${pkgs.libwpe-fdo}/lib/libWPEBackend-fdo-1.0.so"
          "--set-default"
          "GIO_MODULE_DIR"
          "${pkgs.glib-networking}/lib/gio/modules"
          "--set-default"
          "WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS"
          "1"
          "--set-default"
          "WEBKIT_USE_SINGLE_WEB_PROCESS"
          "1"
          "--prefix"
          "PATH"
          ":"
          "${wpeWebkit}/libexec/wpe-webkit-2.0"
        ];
  # makeWrapper arguments, as shell words interpolated into postInstall.
  # They configure the editor process itself: its private library path,
  # runtime root, drivers and plugins.
  wrapArgs = [
    "--prefix"
    "LD_LIBRARY_PATH"
    ":"
    "\"${pkgs.lib.makeLibraryPath runtimeLibs}\""
    "--set-default"
    "RUST_LOG"
    "info"
    "--set-default"
    "ROMACS_RUNTIME_ROOT"
    "\"$out/share/romacs\""
  ]
  ++ linuxWrapArgs;
  # Every variable those arguments change.
  wrappedEnvironmentNames = lib.unique (
    lib.concatLists (
      lib.imap0 (
        index: arg:
        lib.optional (builtins.elem arg [
          "--set"
          "--set-default"
          "--unset"
          "--prefix"
          "--suffix"
          "--prefix-each"
          "--suffix-each"
          "--prefix-contents"
          "--suffix-contents"
        ]) (builtins.elemAt wrapArgs (index + 1))
      ) wrapArgs
    )
  );
  # Before changing anything, record what the user's environment held:
  # ROMACS_WRAPPER_ENV lists the changed names and
  # ROMACS_WRAPPER_ORIGINAL_<NAME> holds each one that was set.  Romacs
  # starts `initial-environment' and `process-environment' from that record,
  # so subprocesses inherit the user's environment rather than the editor's
  # private Nix library path.  A name already recorded by an enclosing
  # wrapped launch keeps its first, original value.
  recordOriginalEnvironment = lib.concatMapStrings (name: ''
    case ":''${ROMACS_WRAPPER_ENV-}:" in
      *:${name}:*) ;;
      *)
        if [ -n "''${${name}+x}" ]; then
          export ROMACS_WRAPPER_ORIGINAL_${name}="''${${name}}"
        fi
        export ROMACS_WRAPPER_ENV="''${ROMACS_WRAPPER_ENV:+''${ROMACS_WRAPPER_ENV}:}${name}"
        ;;
    esac
  '') wrappedEnvironmentNames;
in
craneLib.buildPackage (
  commonArgs
  // {
    # The flake is already a content-addressed, Git-filtered source tree. The
    # full tree is required for Lisp/runtime assets; Cargo-only filtering is
    # used solely for the dependency derivation above.
    src = source;
    inherit cargoArtifacts;

    postInstall = ''
      # The protocol-blind relay is independent of editor wrappers/runtime state.
      install -m 0755 target/release/romacs-mcp "$out/bin/romacs-mcp"

      mkdir -p "$out/share/romacs"
      cp -r lisp lisp-bundled "$out/share/romacs/"
      cp -r etc "$out/share/romacs/"

      # GNU Emacs installs this version-independent site-lisp root.
      # Nixpkgs' emacsPackagesFor wrapper composes its generated site-start
      # with the wrapped editor's original file at this exact path.
      mkdir -p "$out/share/emacs/site-lisp"
      printf '%s\n' \
        ';;; site-start.el --- Nix Emacs package compatibility  -*- lexical-binding: t; -*-' \
        ';;; Commentary:' \
        ';; Romacs runtime paths are configured by its executable wrapper.' \
        ';;; Code:' \
        ';;; site-start.el ends here' \
        > "$out/share/emacs/site-lisp/site-start.el"
      printf '%s\n' \
        ';;; subdirs.el --- Nix Emacs package compatibility  -*- lexical-binding: t; -*-' \
        > "$out/share/emacs/site-lisp/subdirs.el"

      mkdir -p \
        "$out/share/applications" \
        "$out/share/icons" \
        "$out/share/info" \
        "$out/share/man"
      ${lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
        bash scripts/install-linux-desktop-assets.sh "$out"
      ''}

      # Keep VM2's native CLI name; the desktop command launches the GUI.
      ln -s romacs-vm-gui "$out/bin/romacs"
      ln -s romacs "$out/bin/emacs"

      wrapProgram "$out/bin/romacs-vm-gui" \
        --run ${lib.escapeShellArg recordOriginalEnvironment} \
        ${lib.concatStringsSep " \\\n        " wrapArgs}

      # Refuse to produce a package whose runtime Lisp is not the source's:
      # every lisp/, lisp-bundled/ and etc/ file byte-identical, and the
      # wrapper pointing the binary at it. This once installed 0 of 229.
      bash scripts/check-installed-runtime.sh "$out" .
    '';

    passthru = {
      inherit gstreamerRuntime productionCapabilities cargoArtifacts;
      cargoSource = cargoSrc;
      fullSource = source;
    };

    meta = {
      description = "GPU-accelerated Emacs-compatible editor written in Rust";
      homepage = "https://git.thanosapollo.org/romacs";
      license = lib.licenses.gpl3Plus;
      mainProgram = "romacs";
    };
  }
)
