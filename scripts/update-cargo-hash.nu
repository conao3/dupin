#!/usr/bin/env nu

# Refresh a package's vendored-dependency hash after its Cargo.lock changes.
#
# nix-update evaluates the flake from its store path, which Nix's lazy trees don't create
# until something asks for it, so archive the flake first.
def main [package: string = "default", --file: path]: nothing -> nothing {
    let flags = if $file == null {
        []
    } else {
        [
            --override-filename
            ($file | path expand)
        ]
    }
    cd ($env.FILE_PWD | path dirname)
    nix flake archive --json | from json | ignore
    let attribute = if $package == "default" { "default.baseline" } else { $package }
    nix-update --flake --version=skip ...$flags $attribute
}
