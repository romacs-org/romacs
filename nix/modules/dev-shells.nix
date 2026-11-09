{ ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      devShells.default = import ../dev-shell.nix {
        inherit pkgs;
        rustToolchain = pkgs.rust-romacs;
      };
    };
}
