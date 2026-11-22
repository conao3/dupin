{ callPackage }:
callPackage ./wasm-plugin.nix {
  directory = "plugins/calculator";
  resourceName = "calculator.wasm";
  cargoHash = "sha256-E+x8kpwzdT7zRA9EVJQkX46ulbtP+c/6K4zG7swoeRg=";
}
