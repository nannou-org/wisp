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
    version = "0.2.123";
    hash = "sha256-ymeAEYsr7OnupWYJWjSeVGvq3+s+zxSNkODbzY62rYs=";
  };
  cargoDeps = rustPlatform.fetchCargoVendor {
    inherit src;
    inherit (src) pname version;
    hash = "sha256-d7x6gtx5OqEE4MyT6yjYn/qtgjx7GroTpXJewnBV2dU=";
  };
}
