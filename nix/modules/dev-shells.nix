{ ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      devShells.default = import ../dev-shell.nix {
        inherit pkgs;
        rustToolchain = pkgs.rust-romacs;
        wpeWebkit = if pkgs.stdenv.hostPlatform.isLinux then pkgs.romacs-wpewebkit else null;
      };
    };
}
