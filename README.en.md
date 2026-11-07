<p align="center">
  <img src="./assets/128x128.png" width="64" height="64" alt="Nomo icon">
</p>

<h1 align="center">Nomo</h1>

<p align="center">
  <strong>A free, open-source, high-performance WYSIWYG Markdown editor.</strong><br>
  Source and split-view editing, with math, diagrams, syntax highlighting, and fine typography.
</p>

<p align="center">
  <a href="https://github.com/nomo-md/nomo/releases"><img src="https://img.shields.io/github/v/release/nomo-md/nomo?style=flat-square&amp;label=release&amp;color=2f7d6f" alt="Published release"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-68707a?style=flat-square" alt="Windows, macOS, and Linux">
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-2f7d6f?style=flat-square" alt="AGPL-3.0-or-later license"></a>
</p>

<p align="center">
  <a href="https://github.com/nomo-md/nomo/releases"><strong>Download Nomo</strong></a> ·
  <a href="#features">Features</a> ·
  <a href="#performance-optimizations">Performance</a> ·
  <a href="./README.md">简体中文</a>
</p>

<p align="center">
  <img src="./assets/nomo-editor-en.gif" width="800" alt="Nomo Markdown editing demo in English">
</p>

## Features

- **Free and open source:** free to use under AGPL-3.0-or-later, with publicly available source code. Explore the code, report issues, or contribute improvements.
- **WYSIWYG editing:** see Markdown formatting as you edit. Switch to source or split view with the same document and undo history. Type `/` to insert content quickly.
- **Native performance:** built with Rust + GPUI. Load and lay out content on demand, prioritize the visible area, and reduce repeated parsing and unnecessary resource retention.
- **Rich Markdown support:** tables, syntax-highlighted code, LaTeX math, Mermaid diagrams, tasks, callouts, footnotes, inline comments, collapsible content, and front matter.
- **Fine typography and themes:** optional Knuth–Plass line breaking, English hyphenation, and hanging punctuation; four built-in themes, light and dark palettes, installed fonts, and theme import/export. Display-only hyphens never alter the source.
- **A local document workspace:** a file tree, recent files, tabs, outlines, and full-document search and replace, with autosave, local snapshots, and workspace restoration. Documents stay in your own folders.
- **Mini and document comparison:** keep the current document in a compact Markdown Mini window, with pinning on Windows. Windows comparison tabs can place two documents side by side.
- **Exports and system integration:** export HTML, PDF, and Word, retaining editable body text and tables in Word. Use macOS Quick Look, local image management, and optional PicGo uploads.

Nomo also includes Markdown linting, custom shortcuts, word counts, reading-time estimates, and four interface languages: Simplified Chinese, Traditional Chinese, English, and Japanese.

## Performance optimizations

From everyday notes to Markdown with images, code, and diagrams, Nomo uses native rendering and on-demand processing to reduce repeated work when opening, editing, and switching documents.

- **On-demand loading and layout:** prioritize visible content, prefetch adjacent regions as needed, and retire offscreen regions.
- **Shared document state:** WYSIWYG, source, and split views share a document session and history instead of keeping a separate full-text copy for each mode.
- **Bounded resources:** layout, images, highlighting, and enhanced rendering have budgets. Stale work is cancelled or invalidated, and related resources are reclaimed when documents close.

### Time to first visible content

| Test scenario | First visible content |
| --- | ---: |
| 5 MiB plain-text Markdown | **160 ms** |
| 10 MiB mixed Markdown with all node types | **203 ms** |
| Image-heavy document | **119 ms** |
| Seven Mermaid diagram types | **137 ms** |
| Code in 29 languages | **216 ms** |
| Multiple tabs: opening the fourth document | **146 ms** |

Tested on November 7, 2026, on Windows with an Intel Core i5-12400. Values are medians of three rounds, measured from an open request in an already running app until the first screen's text appears; app startup is excluded. See the [performance notes](./docs/performance.md) for the build, environment, and source data.

## Download and install

The native Rust + GPUI edition has not had its first official release yet. Existing Releases contain the historical edition, which differs from the native development version shown here. To try the native edition, build from source or download packages from a manually verified Actions run.

Get the version for your system from [GitHub Releases](https://github.com/nomo-md/nomo/releases):

| Platform | Release files |
| --- | --- |
| Windows x64 | `.exe` installer / portable `.zip` |
| macOS Apple Silicon | `.dmg` / `.app.tar.gz` |
| Linux x64 | `.deb` / `.rpm` |

Open a local Markdown file or folder, or create a document from the home page. See [release notes](https://github.com/nomo-md/nomo/releases) for version changes and the [build guide](./docs/native-build.md) for platform builds and integrations.

## Run from source

Use stable Rust. The root workspace contains `src-gpui`, with dependencies locked in the root `Cargo.lock`. The current build uses `gpui-pre` / `gpui-pre-platform` **0.3.10**.

```sh
cargo run --locked --bin nomo
cargo run --locked --bin nomo -- /path/to/document.md
cargo check --locked --bin nomo
cargo build --locked --release --bin nomo
```

Windows requires MSVC C++ Build Tools, the Windows SDK, and PowerShell 7; packaging also needs NSIS 3. macOS requires full Xcode and the Metal toolchain. Linux prerequisites are listed in the [build guide](./docs/native-build.md).

One-command Windows packaging:

```powershell
pwsh -File scripts/package-windows.ps1
```

The installer and portable ZIP are written to `.artifacts/packages/`. See [NomoMark notes](./docs/nomo-mark.md) for the development entrypoint, [native test tools](./test-lab/README.md) for verification, and [CLASSMAP](./CLASSMAP.md) for code ownership.

## Contribute and license

When reporting an [issue](https://github.com/nomo-md/nomo/issues), include your system, app version, reproduction steps, and a shareable sample. [Pull requests](https://github.com/nomo-md/nomo/pulls) are welcome.

Native development uses `master` as its mainline. Create feature branches from `master` and merge them back through pull requests; pushing code and publishing an official release are separate steps.

Nomo is licensed under [AGPL-3.0-or-later](./LICENSE). Lucide icons retain their [ISC license](./src-gpui/assets/icons/LICENSE); other components retain their own licenses. Thanks to GPUI, pulldown-cmark, latex-rust, merman, resvg, and the projects behind them.
