# The wasm build of the editor, produced with trunk and ready to serve as a
# static site (see `flake.nix`'s `wisp-editor-web` package).
{ binaryen
, lib
, lld
, rustPlatform
, trunk
, wasm-bindgen-cli
,
}:
let
  src = lib.sourceFilesBySuffices ../. [
    ".lock"
    ".rs"
    ".toml"
    ".html"
    ".css"
    ".js"
    ".json"
    # The editor `include_str!`s the bundled shaders at compile time, so the
    # `.wgsl` assets must be present in the build sandbox.
    ".wgsl"
  ];
in
rustPlatform.buildRustPackage {
  pname = "wisp-editor-web";
  version = "0.1.0";
  inherit src;
  cargoLock.lockFile = ../Cargo.lock;
  doCheck = false;
  dontFixup = true;

  nativeBuildInputs = [
    binaryen
    lld
    trunk
    wasm-bindgen-cli
  ];

  # Tell trunk to use the Nix-provided tools, not download its own.
  TRUNK_SKIP_VERSION_CHECK = "true";

  # buildRustPackage's configurePhase sets up cargo vendoring; override the
  # build to drive trunk (reads the root `Trunk.toml`) instead of cargo.
  buildPhase = ''
    trunk build --release --dist $out
  '';

  installPhase = "true";
}
