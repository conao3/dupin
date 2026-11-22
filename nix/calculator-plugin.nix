{
  lib,
  rustPlatform,
  rust,
  lld,
}:
let
  root = ../.;
  manifest = builtins.fromTOML (builtins.readFile (root + /plugins/calculator/Cargo.toml));
in
rustPlatform.buildRustPackage {
  pname = "dupin-calculator";
  version = manifest.package.version;
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + /Cargo.toml)
      (root + /plugins/calculator)
      (root + /crates/dupin-plugin-protocol)
    ];
  };
  cargoRoot = "plugins/calculator";
  cargoDepsName = "dupin-calculator-vendor-${
    builtins.substring 0 16 (builtins.hashFile "sha256" (root + /plugins/calculator/Cargo.lock))
  }";
  cargoHash = "sha256-E+x8kpwzdT7zRA9EVJQkX46ulbtP+c/6K4zG7swoeRg=";
  nativeBuildInputs = [ lld ];
  env.CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER = "${lld}/bin/wasm-ld";
  auditable = false;
  dontStrip = true;
  doCheck = true;

  # The standard Cargo hooks select the native target; this resource is bare Wasm.
  buildPhase = ''
    runHook preBuild
    export CARGO_TARGET_DIR="$PWD/target"
    ${rust.envVars.setEnv} cargo build --manifest-path plugins/calculator/Cargo.toml \
      --offline --locked --release --lib --target wasm32-unknown-unknown
    runHook postBuild
  '';
  checkPhase = ''
    runHook preCheck
    ${rust.envVars.setEnv} cargo test --manifest-path plugins/calculator/Cargo.toml --offline --locked
    runHook postCheck
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p $out/share/dupin/plugins
    cp target/wasm32-unknown-unknown/release/dupin_calculator.wasm $out/share/dupin/plugins/calculator.wasm
    runHook postInstall
  '';
}
