{
  description = ''
    A Nix flake for bevy_wisp development.
  '';

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
  };

  outputs = inputs:
    let
      systems = import inputs.systems;
      lib = inputs.nixpkgs.lib;
      perSystem = f: lib.genAttrs systems f;
      systemPkgs = system: import inputs.nixpkgs { inherit system; };
      perSystemPkgs = f: perSystem (system: f system (systemPkgs system));
    in
    {
      devShells = perSystemPkgs (system: pkgs: {
        wisp-dev = pkgs.callPackage ./shell.nix { };
        default = inputs.self.devShells.${system}.wisp-dev;
      });

      formatter = perSystemPkgs (_: pkgs: pkgs.nixpkgs-fmt);
    };
}
