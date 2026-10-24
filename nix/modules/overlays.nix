{ inputs, lib, ... }:
let
  mkRomacs =
    final: inputs:
    import ../package.nix {
      inherit (inputs) crane;
      pkgs = final;
      rustToolchain = final.rust-romacs;
      source = ../..;
      version =
        inputs.self.shortRev or inputs.self.dirtyShortRev or inputs.self.lastModifiedDate or "0.0.1";
      wpeWebkit = if final.stdenv.hostPlatform.isLinux then final.romacs-wpewebkit else null;
    };
  romacsOverlay =
    final: prev:
    {
      rust-romacs = (final.rust-bin.fromRustupToolchainFile ../../rust-toolchain.toml).override {
        extensions = [
          "rust-src"
          "rust-analyzer"
        ];
      };

      romacs = mkRomacs final inputs;
    }
    // lib.optionalAttrs prev.stdenv.hostPlatform.isLinux {
      # Keep nix-wpe-webkit's pinned nixpkgs so this resolves to its cache,
      # while avoiding an override of the consumer's generic wpewebkit.
      romacs-wpewebkit = inputs.nix-wpe-webkit.packages.${prev.stdenv.hostPlatform.system}.wpewebkit;
    };
in
{
  # One self-contained public overlay: consumers get both the pinned Rust
  # toolchain and pkgs.romacs by applying only overlays.default.
  flake.overlays.default = lib.composeManyExtensions [
    inputs.rust-overlay.overlays.default
    romacsOverlay
  ];

  perSystem =
    { system, ... }:
    {
      _module.args.pkgs = import inputs.nixpkgs {
        inherit system;
        config = { };
        overlays = [ inputs.self.overlays.default ];
      };
    };
}
