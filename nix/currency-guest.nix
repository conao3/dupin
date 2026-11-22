{ callPackage }:
callPackage ./wasm-plugin.nix {
  directory = "experiments/currency-guest";
  resourceName = "currency-fixture.wasm";
  cargoHash = "sha256-i20JtKZL0VvLlGp2af//1ooAPtWkJyqgl/3X+fjaxgg=";
}
