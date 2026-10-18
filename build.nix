let
  # Same reviewed nixpkgs revision as the protected ARM host build.
  pkgs = import (builtins.fetchTarball {
    url = "https://releases.nixos.org/nixos/unstable/nixos-26.11pre1053317.0e251e24a4f2/nixexprs.tar.xz";
    sha256 = "sha256-yjGG65gA1jORt0hWxjzrnw3AYkIn8EWolp8vezwXZ3I=";
  }) { };
in import ./bundle.nix {
  inherit pkgs;
  releaseVersion = (builtins.fromJSON (builtins.readFile ./release.json)).version;
}
