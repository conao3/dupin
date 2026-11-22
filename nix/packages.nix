{
  perSystem =
    { pkgs, ... }:
    let
      blacksmith = pkgs.callPackage ./blacksmith.nix { };
      calculator-plugin = pkgs.callPackage ./calculator-plugin.nix { };
      currency-guest = pkgs.callPackage ./currency-guest.nix { };
      minimal-plugin = pkgs.runCommand "dupin-minimal-plugin" { nativeBuildInputs = [ pkgs.wabt ]; } ''
        mkdir -p $out/share/dupin/plugins
        wat2wasm ${../experiments/wasm-app-size/minimal-plugin.wat} \
          -o $out/share/dupin/plugins/minimal.wasm
      '';
      dupin = pkgs.callPackage ../package.nix { calculatorPlugin = calculator-plugin; };
      dupin-no-wasm = dupin.override { withWasm = false; };
      # scripts/package-app.nu lays out Dupin.dmg over this.
      dmg-background =
        pkgs.runCommand "dupin-dmg-background"
          {
            nativeBuildInputs = [ pkgs.resvg ];
          }
          ''
            mkdir $out
            for scale in 1 2; do
              name=background; [ $scale = 2 ] && name=background@2x
              resvg --skip-system-fonts --use-fonts-dir ${pkgs.inter}/share/fonts \
                -w $((660 * scale)) -h $((400 * scale)) ${../packaging/dmg/background.svg} $out/$name.png
            done
          '';
    in
    {
      packages = {
        inherit
          dupin
          dupin-no-wasm
          blacksmith
          dmg-background
          calculator-plugin
          currency-guest
          minimal-plugin
          ;
        default = dupin;
      };
    };
}
