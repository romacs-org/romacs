{
  lib,
  pkgs,
  homeManagerLib,
  package,
  app,
  devShell,
}:
let
  system = pkgs.stdenv.hostPlatform.system;
  productionCapabilities = package.productionCapabilities;
  outputContract =
    assert lib.assertMsg (pkgs ? romacs) "overlays.default must expose pkgs.romacs";
    assert lib.assertMsg (
      pkgs.romacs == package
    ) "packages.${system}.default must be built through overlays.default";
    assert lib.assertMsg (
      !pkgs.stdenv.hostPlatform.isLinux || pkgs ? romacs-wpewebkit
    ) "overlays.default must expose its pinned WPE package under a Romacs-specific name";
    assert lib.assertMsg (
      package.type or null == "derivation"
    ) "packages.${system}.default must be a derivation";
    assert lib.assertMsg (app.type or null == "app") "apps.${system}.default must be an app";
    assert lib.assertMsg (
      devShell.type or null == "derivation"
    ) "devShells.${system}.default must be a derivation";
    assert lib.assertMsg (
      productionCapabilities ? cargoFeatures
    ) "packages.${system}.default must publish its Cargo capability set";
    assert lib.assertMsg (
      productionCapabilities ? videoBackend
    ) "packages.${system}.default must publish its video backend policy";
    pkgs.runCommand "romacs-${system}-flake-output-contract" { } ''
      touch "$out"
    '';

  canRunPackage = pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform;

  packageContract =
    { checkedPackage }:
    pkgs.runCommand "romacs-${system}-installed-package-contract"
      {
        nativeBuildInputs = [
          pkgs.coreutils
          pkgs.gnugrep
        ];
      }
      ''
        # VM2's CLI is independent of the graphical desktop entrypoint.
        test -x ${checkedPackage}/bin/romacs-vm
        test -x ${checkedPackage}/bin/romacs-mcp
        test -x ${checkedPackage}/bin/romacs-vm-gui
        test -x ${checkedPackage}/bin/.romacs-vm-gui-wrapped
        test ! -L ${checkedPackage}/bin/romacs-vm
        for launcher in romacs emacs; do
          test -L ${checkedPackage}/bin/$launcher
          test "$(readlink -e ${checkedPackage}/bin/$launcher)" \
            = "${checkedPackage}/bin/romacs-vm-gui"
        done
        # Inspect launch paths without starting a GUI in the build sandbox.
        grep -Fq '${checkedPackage}/bin/.romacs-vm-gui-wrapped' \
          ${checkedPackage}/bin/romacs-vm-gui

        test -d ${checkedPackage}/share/romacs/gnu-lisp
        test -d ${checkedPackage}/share/romacs/rlisp
        test -d ${checkedPackage}/share/romacs/vendor-lisp
        test ! -e ${checkedPackage}/share/romacs/lisp
        test ! -e ${checkedPackage}/share/romacs/lisp-bundled
        test -d ${checkedPackage}/share/romacs/etc
        test -f ${checkedPackage}/share/emacs/site-lisp/site-start.el
        test -f ${checkedPackage}/share/emacs/site-lisp/subdirs.el
        test -d ${checkedPackage}/share/applications
        test -d ${checkedPackage}/share/icons
        test -d ${checkedPackage}/share/info
        test -d ${checkedPackage}/share/man
        ${lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
          test -f ${checkedPackage}/share/applications/romacs.desktop
          # The binary visits FILE arguments (crates/romacs-vm-gui/tests/
          # desktop_entry.rs runs it with this Exec=), so the entry passes the
          # chosen files and names the types it opens. Exact lines, each a
          # plain command so errexit fails the check when one is missing.
          grep -Fxq 'Exec=romacs %F' ${checkedPackage}/share/applications/romacs.desktop
          grep -Fxq 'MimeType=text/plain;text/org;' ${checkedPackage}/share/applications/romacs.desktop
          test "$(grep -c '^Exec=' ${checkedPackage}/share/applications/romacs.desktop)" = 1
          test "$(grep -c '^MimeType=' ${checkedPackage}/share/applications/romacs.desktop)" = 1
          test -f ${checkedPackage}/share/icons/hicolor/scalable/apps/romacs.svg
        ''}

        export HOME="$TMPDIR/clean-home"
        mkdir -p "$HOME"
        test "$(printf '%s\n' '(+ 20 22)' | ${checkedPackage}/bin/romacs-vm --source)" = 42
        test "$(printf '%s\n' '["+",20,22]' | ${checkedPackage}/bin/romacs-vm)" = 42

        touch "$out"
      '';
in
{
  flake-output-contract = outputContract;
}
// lib.optionalAttrs canRunPackage {
  installed-package-contract = packageContract { checkedPackage = package; };
  home-manager-contract = import ./home-manager.nix {
    inherit homeManagerLib pkgs package;
  };
}
