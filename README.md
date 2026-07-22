# Game Config Edit

A Rust and Dioxus desktop editor for game CSV configuration files. It provides virtualized table and text views, whole-column type diagnostics, workspace search, safe saves and external-change detection.

Product behavior and acceptance criteria are defined in [`docs/requirements.md`](docs/requirements.md).

## Install

Windows x64 and macOS Apple Silicon are supported:

```sh
npm install --global game-config-edit
gconf [workspace]
```

## Development

```sh
cargo run -- .
cargo test --all-targets
npm test
```
