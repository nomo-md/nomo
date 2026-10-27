<p align="center">
  <img src="./src-tauri/icons/128x128.png" alt="Nomo icon" width="60">
</p>

<h1 align="center">Nomo</h1>

<p align="center">
  <a href="https://github.com/nomo-md/nomo/releases/latest">
    <img src="https://img.shields.io/github/v/release/nomo-md/nomo?label=release" alt="GitHub Release">
  </a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey" alt="Available for macOS and Windows">
  <a href="./LICENSE">
    <img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-blue" alt="AGPL-3.0-or-later">
  </a>
</p>

<p align="center">
  <a href="./README.md">简体中文</a>
  ·
  <a href="./README.en.md"><strong>English</strong></a>
</p>

<p align="center">
  <strong>A local-first Markdown editor that keeps writing simple.</strong>
</p>

<p align="center">
  <img src="./assets/nomo-editor-en.gif" alt="Nomo writing demo in English" width="800">
</p>

## Download and Install

Get the latest version from [GitHub Releases](https://github.com/nomo-md/nomo/releases/latest).

| Platform | Requirements | Installation |
| :--- | :--- | :--- |
| Windows | Windows 10 / 11, x64 | `.exe` installer / `.zip` portable |
| macOS | macOS 12.0+, Apple Silicon | `.dmg` / Homebrew |

On macOS, you can also install with Homebrew:

```bash
brew tap nomo-md/nomo https://github.com/nomo-md/nomo
brew install --cask nomo
```

Upgrade with `brew upgrade --cask nomo`.

SmartScreen or Gatekeeper may prompt on first launch; download from this project's Releases.

## Features

- **Markdown editing**: Live preview, source and split modes, with `/` for quick insertion.
- **Rich content**: Tables, syntax highlighting, math formulas and Mermaid diagrams.
- **File management**: Type to locate files in the explorer, work in multiple tabs and edit large TXT / JSON files in segments.
- **Save and restore**: Optional autosave and local Markdown snapshots, plus workspace and reading position restoration.
- **Comfortable writing**: Light and dark themes, [custom color themes and fonts](docs/custom-themes.md) (guide in Chinese), optional fine typography and an always-on-top Markdown mini window.
- **Export and preview**: Export HTML / PDF / Word (.docx), with editable Word text and tables, formulas and diagrams as images, and Quick Look previews on macOS.

See [release notes](https://github.com/nomo-md/nomo/releases) for version changes.

## Development

Built with Tauri 2 + Svelte 5. Requires Node.js 22, pnpm 11.5.1 and Rust / Cargo.

- Windows: Visual Studio 2022 Build Tools or the full Visual Studio.
- macOS: Xcode Command Line Tools; building the Quick Look extension requires the full Xcode.

```bash
pnpm install
pnpm tauri dev
```

## Get Involved

[Issues](https://github.com/nomo-md/nomo/issues) and [pull requests](https://github.com/nomo-md/nomo/pulls) are welcome; include your OS version, steps to reproduce and a sample when reporting a bug.

Enjoy Nomo? [Star the project](https://github.com/nomo-md/nomo) or follow [LIXianSenQwQ](https://github.com/LIXianSenQwQ). Community: [linux.do](https://linux.do/).

Licensed under [AGPL-3.0-or-later](./LICENSE); contact the maintainers for separate commercial licensing. Third-party components retain their own licenses.

Thanks to Tauri, Svelte, ProseMirror, CodeMirror, markdown-it, Shiki, KaTeX, Mermaid and other open-source projects.
