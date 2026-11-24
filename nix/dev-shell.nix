{
  perSystem =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    {
      devShells.default = pkgs.mkShell {
        inputsFrom = [ config.packages.dupin.baseline ];
        packages =
          with pkgs;
          [
            clippy
            rustfmt
            rust-analyzer
            nushell
            nodejs
            # `dupin ext build` falls back to the Rolldown on PATH for a package without its own.
            config.packages.rolldown
            # Type-checks extensions/ as `checks.extensions-typecheck` does.
            typescript
            config.packages.pnpm
            just
            nufmt
            config.treefmt.build.wrapper
            typos
            oxfmt
            actionlint
            zizmor
            gitleaks
          ]
          ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            config.packages.blacksmith
            config.packages.cargo-hawk
          ]
          ++ config.pre-commit.settings.enabledPackages;
        # Installs the pre-commit and pre-push hooks from nix/git-hooks.nix.
        shellHook = config.pre-commit.shellHook;
      };
    };
}
