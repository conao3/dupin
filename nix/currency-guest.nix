{ callPackage }:
callPackage ./wasm-plugin.nix {
  directory = "experiments/currency-guest";
  resourceName = "currency-fixture.wasm";
  cargoHash = "sha256-uNnz8VsqEB59iSxTCklKESatIMThpkzjYijLduml9+A=";
}
