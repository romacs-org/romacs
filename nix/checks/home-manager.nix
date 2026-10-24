{
  homeManagerLib,
  pkgs,
  package,
}:
let
  configuration = homeManagerLib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      {
        home.username = "romacs-nix-check";
        home.homeDirectory = "/tmp/romacs-home-manager-contract";
        home.stateVersion = "24.11";
        # This rolling contract intentionally follows the flake's locked
        # nixpkgs while exercising Home Manager's current Emacs module.  Their
        # development version labels can differ between release branch points.
        home.enableNixpkgsReleaseCheck = false;

        programs.emacs = {
          enable = true;
          package = package;
        };

        # Keep this fixture focused on the package integration contract.
        manual.manpages.enable = false;
        news.display = "silent";
      }
    ];
  };
  finalPackage = configuration.config.programs.emacs.finalPackage;
in
pkgs.runCommand "romacs-home-manager-contract"
  {
    nativeBuildInputs = [
      pkgs.coreutils
      pkgs.gnugrep
    ];
  }
  ''
    test -x ${configuration.activationPackage}/activate
    test -x ${finalPackage}/bin/emacs
    # Home Manager wraps the graphical editor; VM2 has no emacsclient or
    # batch startup interface. Keep this a composition/launcher contract.
    test -x ${package}/bin/romacs-vm-gui
    grep -Fq '${package}/bin/emacs' ${finalPackage}/bin/emacs

    touch "$out"
  ''
