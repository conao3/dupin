#!/usr/bin/env nu

# Refresh a package's vendored-dependency hash after its Cargo.lock changes.
#
# nix-update evaluates the flake from its store path, which Nix's lazy trees don't create
# until something asks for it, so archive the flake first.
def main [package: string = "default"]: nothing -> nothing {
    cd ($env.FILE_PWD | path dirname)
    nix flake archive --json | from json | ignore
    nix-update --flake --version=skip $package
}
