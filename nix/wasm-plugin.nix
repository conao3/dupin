{
  lib,
  rustPlatform,
  rust,
  lld,
  directory,
  resourceName,
  cargoHash,
}:
let
  root = ../.;
  manifest = builtins.fromTOML (builtins.readFile (root + "/${directory}/Cargo.toml"));
  library = lib.replaceStrings [ "-" ] [ "_" ] manifest.package.name;
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name;
  version = manifest.package.version;
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + /Cargo.toml)
      (root + "/${directory}")
      (root + /crates/dupin-plugin-protocol)
    ];
  };
  cargoRoot = directory;
  cargoDepsName = "${manifest.package.name}-vendor-${
    builtins.substring 0 16 (builtins.hashFile "sha256" (root + "/${directory}/Cargo.lock"))
  }";
  inherit cargoHash;
  nativeBuildInputs = [ lld ];
  env.CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER = "${lld}/bin/wasm-ld";
  auditable = false;
  dontStrip = true;
  doCheck = true;

  prePatch = ''
    export NIX_RUSTFLAGS="''${NIX_RUSTFLAGS-} --remap-path-prefix=$cargoDepsCopy=/vendor"
  '';

  # The standard Cargo hooks select the native target; this resource is bare Wasm.
  buildPhase = ''
    runHook preBuild
    export CARGO_TARGET_DIR="$PWD/target"
    ${rust.envVars.setEnv} cargo build --manifest-path ${directory}/Cargo.toml \
      --offline --locked --release --lib --target wasm32-unknown-unknown
    runHook postBuild
  '';
  checkPhase = ''
    runHook preCheck
    ${rust.envVars.setEnv} cargo test --manifest-path ${directory}/Cargo.toml --offline --locked
    runHook postCheck
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p $out/share/dupin/plugins
    cp target/wasm32-unknown-unknown/release/${library}.wasm $out/share/dupin/plugins/${resourceName}
    runHook postInstall
  '';
}
