{
  lib,
  pkgs,
  pluginInputs,
}:
let
  gstreamer = pkgs.gst_all_1.gstreamer;
in
{
  # GStreamer is a multi-output derivation.  Its default coercion selects the
  # command-line `bin` output, while both core elements and the plugin scanner
  # live in the library output.  Resolve output roles explicitly so every
  # consumer gets the same complete, cache-independent runtime contract.
  pluginSystemPath = lib.makeSearchPath "lib/gstreamer-1.0" (map lib.getLib pluginInputs);
  pluginScanner = "${lib.getLib gstreamer}/libexec/gstreamer-1.0/gst-plugin-scanner";
  inspect = "${lib.getBin gstreamer}/bin/gst-inspect-1.0";
}
