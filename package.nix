{
  lib,
  rustPlatform,
  resvg,
  python3Packages,
  inter,
  stdenvNoCC,
  guestPlugins,
  withWasm ? true,
}:
let
  root = ./.;
  # tagpr bumps VERSION; crate versions stay 0.0.0 (see Cargo.toml).
  version = lib.fileContents (root + /VERSION);

  cargoFiles = lib.fileset.unions [
    (root + /Cargo.toml)
    (root + /Cargo.lock)
    (root + /rustfmt.toml)
    (root + /crates)
    # The backdrop shader examples, which a test compiles.
    (root + /examples/shaders)
  ];
  bundleFiles = lib.fileset.unions [
    (root + /packaging)
    (root + /assets/logo/dupin.svg)
  ];
  cargoSrc = lib.fileset.toSource {
    inherit root;
    fileset = cargoFiles;
  };
  bundleSrc = lib.fileset.toSource {
    inherit root;
    fileset = bundleFiles;
  };
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      cargoFiles
      bundleFiles
    ];
  };

  # fetchCargoVendor, not importCargoLock: Rune vendors a patched tiny-skia-path as a path
  # dependency while resvg keeps the published one, so the lock holds two tiny-skia-path
  # 0.11.4 packages, and importCargoLock's name-version directories collide.
  # Update the hash whenever Cargo.lock changes: `nu scripts/update-cargo-hash.nu`.
  # The name carries Cargo.lock's hash. A fixed-output derivation whose hash didn't change is
  # taken from the store as it is, so with a constant name a stale vendor tree from before a
  # lock change passed every local build and failed only on a clean CI store.
  cargoDeps = rustPlatform.fetchCargoVendor {
    inherit src;
    name = "dupin-vendor-${builtins.substring 0 16 (builtins.hashFile "sha256" (root + /Cargo.lock))}";
    hash = "sha256-oPL76a2R4hmcAn7Ie0le4EjuOgs0oiGRwzbnR9hAUIc=";
  };

  # What every Rust derivation of the workspace shares (the package here, clippy and the
  # tests in nix/checks.nix). The toolchain is nixpkgs' own, which matches the version
  # Rune's rust-toolchain.toml pins.
  commonArgs = {
    pname = "dupin";
    inherit version src cargoDeps;
    # RUSTFLAGS would replace cargoSetupHook's target-specific flags.
    prePatch = ''
      export NIX_RUSTFLAGS="''${NIX_RUSTFLAGS-} --remap-path-prefix=$cargoDepsCopy=/vendor"
    '';
  };
  packagingArgs = {
    # The app icon is drawn from the logo here, so no generated image is committed. iconutil
    # ships only with macOS, outside the sandbox; icnsutil writes the same icns.
    nativeBuildInputs = [
      resvg
      python3Packages.icnsutil
    ];
    # cargoBuildHook overrides the profile's `strip` and leaves stripping to stdenv, whose
    # default flags include `-p`, which llvm-strip rejects for Mach-O; the hook ignores the
    # failure and leaves every symbol in place.
    stripAllList = [ "Applications/Dupin.app/Contents/MacOS" ];
    stripAllFlags = [ "--strip-all" ];
    postInstall = ''
      app=$out/Applications/Dupin.app/Contents
      mkdir -p $app/MacOS
      mv $out/bin/dupin $app/MacOS/dupin
      substitute packaging/Info.plist $app/Info.plist --subst-var-by version ${version}

      # Every size an iconset has: 16 to 512 points, each also at 2x (up to 1024 pixels). The
      # nightly icon's ribbon is text, and the sandbox has no system fonts.
      icns() {
        local iconset
        iconset=$(mktemp -d)
        for size in 16 32 128 256 512; do
          resvg --skip-system-fonts --use-fonts-dir ${inter}/share/fonts -w $size -h $size "$1" $iconset/icon_''${size}x''${size}.png
          resvg --skip-system-fonts --use-fonts-dir ${inter}/share/fonts -w $((size * 2)) -h $((size * 2)) "$1" $iconset/icon_''${size}x''${size}@2x.png
        done
        icnsutil compose --toc "$2" $iconset/*.png
        icnsutil test "$2"
      }
      mkdir -p $app/Resources
      icns assets/logo/dupin.svg $app/Resources/dupin.icns
      # scripts/package-app.nu points a nightly build at this one.
      icns packaging/nightly-icon.svg $app/Resources/dupin-nightly.icns
      ln -s $app/MacOS/dupin $out/bin/dupin
    '';
    meta = {
      description = "A lightweight resident launcher for Apple silicon Macs";
      homepage = "https://dupin.app";
      license = lib.licenses.gpl3Plus;
      mainProgram = "dupin";
      platforms = [ "aarch64-darwin" ];
    };
  };
  nativeApplication = rustPlatform.buildRustPackage (
    commonArgs
    // packagingArgs
    // {
      cargoBuildFlags = [
        "--package"
        "dupin"
      ]
      ++ lib.optional (!withWasm) "--no-default-features";
      # Tests run in the lint-and-test check, in debug as cargo runs them.
      doCheck = false;
      passthru = {
        inherit
          commonArgs
          packagingArgs
          cargoSrc
          bundleSrc
          ;
      };
    }
  );
  wrapApplication =
    application:
    stdenvNoCC.mkDerivation {
      pname = "dupin";
      inherit version;
      dontUnpack = true;
      dontBuild = true;
      dontFixup = true;
      installPhase = ''
        mkdir -p $out/Applications $out/bin
        cp -R ${application}/Applications/Dupin.app $out/Applications/Dupin.app
        chmod -R u+w $out/Applications/Dupin.app
        ${lib.optionalString withWasm ''
          mkdir -p $out/Applications/Dupin.app/Contents/Resources/plugins
          ${lib.concatMapStringsSep "\n" (
            plugin:
            "cp ${plugin}/share/dupin/plugins/*.wasm $out/Applications/Dupin.app/Contents/Resources/plugins/"
          ) guestPlugins}
        ''}
        ln -s $out/Applications/Dupin.app/Contents/MacOS/dupin $out/bin/dupin
      '';
      passthru = application.passthru // {
        inherit
          commonArgs
          cargoDeps
          packagingArgs
          cargoSrc
          bundleSrc
          wrapApplication
          withWasm
          ;
        nativeApplication = application;
      };
      inherit (application) meta;
    };
in
wrapApplication nativeApplication
