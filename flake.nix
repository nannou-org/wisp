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
      packages = perSystemPkgs (_: pkgs:
        let
          # Pinned to match the wasm-bindgen in Cargo.lock (nixpkgs lags behind).
          wasm-bindgen-cli = pkgs.callPackage ./pkgs/wasm-bindgen-cli.nix { };
          wisp-editor-web = pkgs.callPackage ./pkgs/wisp-editor-web.nix { inherit wasm-bindgen-cli; };
          serve-wisp-editor-web = pkgs.callPackage ./pkgs/serve-wisp-editor-web.nix { inherit wisp-editor-web; };
        in
        {
          inherit wasm-bindgen-cli wisp-editor-web serve-wisp-editor-web;
          default = wisp-editor-web;
        });

      devShells = perSystemPkgs (system: pkgs: {
        wisp-dev = pkgs.callPackage ./shell.nix {
          inherit (inputs.self.packages.${system}) wasm-bindgen-cli;
        };
        default = inputs.self.devShells.${system}.wisp-dev;
      });

      formatter = perSystemPkgs (_: pkgs: pkgs.nixpkgs-fmt);
    };
}
