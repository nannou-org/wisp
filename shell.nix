# A development shell providing the Rust toolchain, release tooling and the
# system libraries bevy needs at build and run time. CI runs everything
# through this shell (`nix develop -c ...`) for reproducibility.
{ alsa-lib
, binaryen
, cargo
, cargo-semver-checks
, clippy
, fontconfig
, lib
, libx11
, libxcursor
, libxi
, libxkbcommon
, libxrandr
, lld
, mkShell
, pkg-config
, release-plz
, rust-analyzer
, rustc
, rustfmt
, stdenv
, trunk
, udev
, vulkan-loader
, vulkan-validation-layers
, wasm-bindgen-cli
, wayland
}:
let
  runtimeLibs = lib.optionals stdenv.isLinux [
    alsa-lib
    fontconfig
    libx11
    libxcursor
    libxi
    libxkbcommon
    libxrandr
    udev
    vulkan-loader
    vulkan-validation-layers
    wayland
  ];
in
mkShell {
  packages = [
    cargo
    cargo-semver-checks
    clippy
    pkg-config
    release-plz
    rust-analyzer
    rustc
    rustfmt
    # wasm build of the editor: `trunk serve`, `trunk build`.
    binaryen
    lld
    trunk
    wasm-bindgen-cli
  ] ++ runtimeLibs;

  env = lib.optionalAttrs stdenv.isLinux {
    # wgpu/winit dlopen their backends at run time.
    LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
  };
}
