# Changelog

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file is maintained by [release-plz](https://release-plz.dev) from
[conventional commits](https://www.conventionalcommits.org).

## [Unreleased]

## [0.1.0](https://github.com/nannou-org/wisp/releases/tag/v0.1.0) - 2026-06-12

### Added

- *(examples)* split editor layout - panel left, shader right
- *(ui)* embeddable params/errors widgets
- *(render)* honour the camera viewport
- *(examples)* live-coding editor example
- prefer mailbox presentation for wisp windows
- *(wisp)* audio waveform and FFT textures (audio feature)
- *(wisp)* compute passes
- *(wisp)* auto-generated egui control panel (ui feature)
- *(wisp)* multi-pass rendering with persistent and feedback targets
- *(wisp)* surface load and pipeline errors for live coding
- *(wisp)* render single-pass wisp shaders to camera views
- *(wisp)* add nannou_wisp crate with reflected WGSL schema

### Fixed

- Readme space
- *(render)* never mix per-view render components across shaders

### Other

- document mailbox preference and the editor example
- Small README amendments
- *(ci)* nix-driven CI and release-plz automation
- *(nix)* flake with dev shell for reproducible tooling
- standalone README with badges and bevy compatibility table
- *(examples)* port examples from nannou to plain bevy
- standalone bevy_wisp crate
- *(wisp)* writing-wisp-shaders guide, image-input example, changelog
