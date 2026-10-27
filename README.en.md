# Nomo

[简体中文](README.md) · [Releases](https://github.com/nomo-md/nomo/releases) · [Build and release](docs/native-build.md)

A local-first native Markdown editor built with Rust and GPUI. Builds and packaging no longer require Node.js, npm, pnpm, Vite or Tauri.

The default Workspace now uses the windowed EditorControl with one document session and history per tab. Normal startup, new files and opened files require no development flags. Semantic, source and shared-session split modes are connected; split editing, the menu chrome and reading-position synchronization still require desktop acceptance. Page parity and complete interaction verification are tracked in [ROADMAP](src-gpui/ROADMAP.md).

**The native source has not reached feature or visual parity with previous desktop releases.** PDF/Word/HTML/image export, Quick Look, MSIX and Shell extensions are currently unavailable. External file monitoring, tray integration, full workspace restoration, custom shortcuts and localization remain incomplete. Existing releases remain available; this migration does not publish a new version.

## Development

Use stable Rust and the root `Cargo.lock`. GPUI remains pinned to 0.2.2.

- Windows: MSVC C++ Build Tools and Windows SDK including `fxc.exe` and `rc.exe`. Packaging needs NSIS 3 and PowerShell 7.
- macOS: full Xcode, Command Line Tools and Metal; packaging uses `codesign`, `hdiutil` and `jq`.
- Linux: Clang, C/C++ tools and native font/X11/Wayland/graphics libraries. See [prerequisites](docs/native-build.md).

```sh
cargo run --locked --bin nomo
cargo run --locked --bin nomo -- /path/to/document.md
cargo check --locked --bin nomo
cargo build --locked --release --bin nomo
cargo run --locked --features editor-control-demo --bin nomo -- --editor-control-demo
cargo run --locked --features editor-control-demo --bin nomo -- --editor-control /path/to/document.md
```

See [CLASSMAP](CLASSMAP.md), [editor-control notes](docs/gpui-editor-control.md) and [build instructions](docs/native-build.md) for ownership, entrypoints and verification.

The product name remains Nomo, the binary is `nomo`, and the identifier remains `com.nomo.desktop`. Existing configuration fields and platform directories are preserved; uninstalling does not remove user documents or settings.

Licensed under [AGPL-3.0-or-later](LICENSE). Lucide icons retain their [ISC license](src-gpui/assets/icons/LICENSE); dependencies retain their licenses. Historical notes are in [docs/archive](docs/archive/README.md).
