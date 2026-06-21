{ inputs, ... }:
{
  imports = [ inputs.treefmt-nix.flakeModule ];

  perSystem =
    { pkgs, ... }:
    {
      treefmt = {
        projectRootFile = "LICENSE";
        programs.flake-edit.enable = true;
        programs.nixfmt.enable = true;
        programs.nixf-diagnose.enable = true;
        programs.deadnix.enable = true;
        programs.rustfmt.enable = true;
        programs.taplo.enable = true;
        programs.fish_indent.enable = true;
        programs.shellcheck.enable = true;
        programs.shfmt.enable = true;

        programs.ruff-format.enable = true;
        programs.ruff-check.enable = true;
        programs.mypy.enable = true;
        programs.mypy.directories."nix/checks/forge" = {
          # `nixos-test-driver` is a wrapper derivation, not a buildPythonPackage,
          # so `makePythonPath` ignores it. Point mypy at its site-packages directly.
          extraPythonPaths = [
            "${pkgs.nixos-test-driver}/${pkgs.python3.sitePackages}"
          ];
        };

        settings.excludes = [
          "tests/fixtures/**"
        ];

        settings.formatter.rustfmt.options = [
          "--config"
          "newline_style=Unix"
        ];
      };
    };
}
