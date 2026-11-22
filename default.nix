{
  pkgs ? import <nixpkgs> { },
}:
pkgs.callPackage ./package.nix {
  guestPlugins = [
    (pkgs.callPackage ./nix/calculator-plugin.nix { })
    (pkgs.callPackage ./nix/maps-plugin.nix { })
  ];
}
