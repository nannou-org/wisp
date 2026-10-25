# wasm-bindgen-cli version must match the exact version of wasm-bindgen used
# within the crate dependencies. nixpkgs' version doesn't always match the
# version picked up in our Cargo.lock, so here we pin to a particular
# wasm-bindgen-cli so we can override the nixpkgs version.
{ buildWasmBindgenCli
, fetchCrate
, rustPlatform
,
}:
buildWasmBindgenCli rec {
  src = fetchCrate {
    pname = "wasm-bindgen-cli";
    version = "0.2.129";
    hash = "sha256-pcecKQd7E8Opw6bkFoE569epUi7gh5qpQF1e5PJY6V8=";
  };
  cargoDeps = rustPlatform.fetchCargoVendor {
    inherit src;
    inherit (src) pname version;
    hash = "sha256-vmUrWVU7kPJJxO5qIVeAkwQyWDELO1Z4Z5gitz2kco8=";
  };
}
