{
  pkgs ? import <nixpkgs> { },
}:
pkgs.callPackage ./package.nix {
  calculatorPlugin = pkgs.callPackage ./nix/calculator-plugin.nix { };
}
