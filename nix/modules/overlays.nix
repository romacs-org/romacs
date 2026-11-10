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
