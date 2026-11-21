let
  root = ./..;
in
{
  perSystem =
    { config, pkgs, ... }:
    let
      inherit (config.packages) dupin;
      # A check that runs `script` over the repository's tracked files.
      repoCheck =
        name: inputs: script:
        pkgs.runCommand name { nativeBuildInputs = inputs; } ''
          cd ${root}
          ${script}
          touch $out
        '';
    in
    {
      checks = {
        inherit dupin;
        # Clippy and the tests share one debug build of the dependencies. Formatting is
        # `checks.treefmt`, from nix/treefmt.nix.
        lint-and-test = pkgs.rustPlatform.buildRustPackage (
          dupin.passthru.commonArgs
          // {
            pname = "dupin-lint-and-test";
            nativeBuildInputs = [ pkgs.clippy ];
            buildPhase = ''
              runHook preBuild
              cargo clippy --workspace --all-targets --offline -- -D warnings
              runHook postBuild
            '';
            checkPhase = ''
              runHook preCheck
              cargo test --workspace --offline
              runHook postCheck
            '';
            installPhase = "touch $out";
          }
        );
        gitleaks = repoCheck "dupin-gitleaks" [ pkgs.gitleaks ] ''
          gitleaks detect --source . --config .gitleaks.toml --no-git --redact
        '';
        nushell-ide-check = repoCheck "dupin-nushell-ide-check" [ pkgs.nushell ] ''
          for script in scripts/*.nu; do
            if nu --no-config-file --ide-check 10 "$script" | grep -q '"severity":"Error"'; then
              nu --no-config-file --ide-check 10 "$script"
              exit 1
            fi
          done
        '';
      };
    };
}
