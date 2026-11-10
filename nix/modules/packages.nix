{ ... }:
{
  perSystem =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    let
      romacs = pkgs.romacs;
      romacsApp = {
        type = "app";
        program = lib.getExe romacs;
        meta.description = romacs.meta.description;
      };
    in
    {
      packages = {
        default = romacs;
        inherit romacs;
      };

      apps = {
        default = romacsApp;
        romacs = romacsApp;
      };
    };
}
