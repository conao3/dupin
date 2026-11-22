{ callPackage }:
callPackage ./wasm-plugin.nix {
  directory = "plugins/calculator";
  resourceName = "calculator.wasm";
  cargoHash = "sha256-k19gSyvYfLaQU7ktzzy6/dXtrjJnTc/1JFFKXkhFyDo=";
}
