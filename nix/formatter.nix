{ inputs, ... }:
{
  imports = [ inputs.treefmt-nix.flakeModule ];

  perSystem = _: {
    treefmt = {
      projectRootFile = "LICENSE";
      programs.nixfmt.enable = true;
      programs.nixf-diagnose.enable = true;
      programs.deadnix.enable = true;
      programs.rustfmt.enable = true;
      programs.taplo.enable = true;
      programs.fish_indent.enable = true;

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
