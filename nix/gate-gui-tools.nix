{ pkgs, gnuSource }:
let
  weston = pkgs.weston;
  mesa = pkgs.mesa;
  xserver = pkgs.xorg.xorgserver;
  westonWithEgl = pkgs.writeShellScriptBin "weston" ''
    # Scope the vendor selection to Weston, not every test client.
    export __EGL_VENDOR_LIBRARY_FILENAMES=${mesa}/share/glvnd/egl_vendor.d/50_mesa.json
    exec ${weston}/bin/weston "$@"
  '';
in
{
  gnu-x11 = (pkgs.emacs.override {
    withPgtk = false;
    withX = true;
    withGTK3 = false;
    withXwidgets = false;
    withNativeCompilation = false;
    toolkit = "lucid";
  }).overrideAttrs (_: {
    pname = "romacs-gnu-x11";
    version = "32.0.50";
    src = gnuSource;
    patches = [];
  });
  fontconfig = pkgs.makeFontsConf {
    fontDirectories = [ pkgs.dejavu_fonts pkgs.ubuntu-classic ];
  };
  display-tools = pkgs.buildEnv {
    name = "romacs-gate-display-tools";
    paths = [ westonWithEgl xserver pkgs.xauth pkgs.xdpyinfo ];
    pathsToLink = [ "/bin" ];
  };
}
