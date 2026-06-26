# Changelog

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file is maintained by [release-plz](https://release-plz.dev) from
[conventional commits](https://www.conventionalcommits.org).

## [Unreleased]

## [0.2.0](https://github.com/nannou-org/wisp/releases/tag/wisp_editor-v0.2.0) - 2026-06-25

First release of the editor as its own crate, split out of the `bevy_wisp`
examples during the workspace restructure.

### Added

- Live-coding editor: edit WGSL on the left, see the rendered wisp on the right,
  with shader load and pipeline errors surfaced inline
- Rearrangeable, dockable panel layout via `egui_tiles`
- Auto-generated parameter panel laid out as a label/widget table
- Bundled example shaders, embedded so they load without a filesystem
- Shader persistence via `bevy_pkv` (save/load, including on the web)
- Microphone capture for `@audio` shaders on native, fed through a wait-free
  `rtrb` ring buffer
- A real default image for `@image` inputs
- A wasm build deployed to GitHub Pages

### Changed

- Update to Bevy 0.19 / bevy_egui 0.40 stable
