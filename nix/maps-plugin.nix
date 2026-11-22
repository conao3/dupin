{ callPackage }:
callPackage ./wasm-plugin.nix {
  directory = "plugins/maps";
  resourceName = "maps.wasm";
  cargoHash = "sha256-64TFAqi1MKRhr3M1whR779CzM8QpDhLRTtvBGSudjcY=";
}
