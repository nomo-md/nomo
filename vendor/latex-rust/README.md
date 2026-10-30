# LaTeX-Rust

> **3.0.0 is a breaking release.** MSRV is Rust 1.99, `ttf-parser` is gone
> (RUSTSEC-2026-0192), `MathNode` and `ParseError` are `non_exhaustive`,
> `Fraction` carries a `FracStyle`, prime runs are flat, and optional `egui`
> is 0.36.2. Text mode has its own lexer (`MathNode::Text` holds a
> `TextRun`; see [documents/TEXT_MODE_SPEC.md](documents/TEXT_MODE_SPEC.md)):
> `\text{\{\ )}` and `\text{$x$}` round-trip, and `\\` or unknown commands
> in text are typed errors. See [CHANGELOG.md](CHANGELOG.md) for each change and its
> migration. 3.0.0 was published to crates.io on 2026-10-15; the current release is 3.0.1.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE-MIT)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE-APACHE)

Repository: <https://github.com/jscarr64/latex-rust>

Pure Rust LaTeX math renderer. No JavaScript. No webview. No JavaScript runtime.

It parses LaTeX math into a typed AST, lays the AST out with a TeX-faithful box
model whose dimensions are exact rationals (`Dim`; no hardware `f32`/`f64` in the
layout path), and renders to SVG, PNG (`features = ["png"]`), or egui shapes
(`features = ["egui"]`).

## Features

- Parses LaTeX math to a typed AST
- TeX-faithful layout engine (Appendix G exact)
- SVG output — self-contained, scalable, no font embedding required
- PNG output (`features = ["png"]`) via tiny-skia
- egui integration (`features = ["egui"]`) for native desktop apps
- Full symbol coverage — Greek, AMS, arrows, operators, font styles
- Complete accent and decoration support
- Multiline environments — align, gather, multline, cases, array
- Color support — named, RGB, HTML, CMYK, gray, `\definecolor`
- 100% pure Rust — no C, no Python, no shell, no subprocesses
- MIT OR Apache-2.0

```
src/parser/     LaTeX → AST
src/layout/     AST → box model
src/font/       STIX Two Math metrics / TrueType loader
src/render/svg  box model → SVG
src/render/png  box model → PNG (`tiny-skia`, feature `png`)
src/render/egui box model → egui Shapes (feature `egui`)
golds/          gold tests (repository only)
benches/        parse / layout / render timings
```

This crate does not typeset documents. Text-mode LaTeX, TikZ, chemistry, and PDF
are out of scope and return `Err` — never a fake render.

Inventory: [capabilities](documents/CAPABILITIES.md). Patches: [CONTRIBUTING.md](CONTRIBUTING.md).

## Quick start

```toml
[dependencies]
latex-rust = "3.0"
```

```rust
use latex_rust::{parse, layout, latex_to_svg, MathFont, MathStyle, SvgOptions};

let ast = parse(r"\frac{1}{2}").expect("parse");
let font = MathFont::stix_two_math().expect("STIX Two Math");
let boxed = layout(&ast, &font, MathStyle::Text).expect("layout");
assert!(!boxed.width.is_zero());

let svg = latex_to_svg(r"\frac{1}{2}", &font, &SvgOptions::new()).expect("svg");
assert!(svg.contains("<svg"));
```

Layout math is exact rationals in this crate. It does not depend on a numeric
library.

MSRV is **1.99** (`rust-version = "1.99"`). `rust-toolchain.toml` pins that channel, with clippy and
rustfmt. Optional `egui` and `epaint` are 0.36.2 and stay off unless the
`egui` feature is on.

## Usage

### SVG

```rust
use latex_rust::{latex_to_svg, MathFont, SvgOptions};

let font = MathFont::stix_two_math().unwrap();
let svg = latex_to_svg(r"e^{i\pi}+1=0", &font, &SvgOptions::new()).unwrap();
```

### PNG

Enable with `features = ["png"]`. `tiny-skia` rasterizes the same `MathBox` tree
as the SVG backend. `tiny-skia` picks its own CPU path; no configuration is
needed.

Without the feature, `latex_to_png` / `render_png` return `Err(Unsupported)`.

```rust
# #[cfg(feature = "png")]
# {
use latex_rust::{latex_to_png, MathFont, PngOptions};

let font = MathFont::stix_two_math().unwrap();
let png = latex_to_png(r"\frac{1}{2}", &font, &PngOptions::new()).unwrap();
assert!(png.starts_with(b"\x89PNG"));
# }
```

### egui

Enable with `features = ["egui"]`. Glyphs become `egui::Mesh`; rules and color
boxes become `Shape::Rect`. No SVG intermediate. Callers should keep the
returned `Vec<Shape>` if the same expression is painted every frame (cache hit).

```rust
# #[cfg(feature = "egui")]
# {
use latex_rust::{latex_to_shapes, EguiOptions, MathFont};

let font = MathFont::stix_two_math().unwrap();
let (shapes, rect) =
    latex_to_shapes(r"x^2", &font, &EguiOptions::new(), egui::Pos2::ZERO, 1.0).unwrap();
assert!(!shapes.is_empty());
let _ = rect;
# }
```

## Supported LaTeX

Math mode only. Command and environment coverage is [`data/symbols.tsv`](data/symbols.tsv).
Look up a command with `latex_rust::lookup`. Missing glyphs and unknown commands
are `Err`.

Color: `named_color` / `parse_color_spec`. Channel values use `Dim`. `spot` and
unknown names return `Err`. SVG, PNG, and egui all take `Color::to_rgba8()`.

## Performance

Times below are median-of-batch averages from `cargo bench --features png,egui`
on this machine (release, in-crate `Dim` layout). Re-run the bench on your
hardware.

| Operation | Target | Measured |
|---|---|---|
| Parse `\frac{1}{2}` | < 50µs | 1.8µs |
| Parse full display equation | < 200µs | 3.3µs |
| Layout `\frac{1}{2}` | < 100µs | 13µs |
| Layout full display equation | < 500µs | 47µs |
| SVG `\frac{1}{2}` | < 500µs | 232µs |
| SVG full display equation | < 1ms | 951µs |
| PNG 144 DPI | < 5ms | 142µs |
| PNG 300 DPI | < 15ms | 350µs |
| egui inline (cache miss) | < 2ms | 216µs |
| egui inline (cache hit) | < 0.1ms | 29µs |

KaTeX cold start in a browser is typically **200–400ms** (JS parse + layout +
font). This crate’s first `MathFont::stix_two_math()` plus an SVG of a short
expression is on the order of the layout+SVG rows above after the face is loaded
(embedded STIX Two Math, no network). There is no JS runtime to warm up.

## Fonts

The crate embeds **STIX Two Math** 2.13 (SIL OFL 1.1). Horizontal metrics and
outlines come from `skrifa` in integer font units converted to `Dim`. The
OpenType MATH table is read in this crate. Glyph outlines become SVG `<path>`
elements (and PNG/egui meshes when those features are on). The optional
`egui` 0.36.2 feature uses `epaint` 0.36.2, which reads fonts with
`skrifa`. `ttf-parser` is not in the tree.

## Notes

3.0.1 is the current version (3.0.0 was published 2026-10-15; 3.0.1 changes metadata only). Layout rules are the 2.1.x rules;
some parse trees change (see CHANGELOG.md). Since 2.1.1, `TealBlue` uses the upstream dvipsnames CMYK value, and `data/dvipsnames.tsv` carries its LPPL
notice. baselogic contributed the TeX and OpenType MATH layout work in pull
request 13. His commits are in the history under his name. They cover delimiter
sizing with a physical `\delimitershortfall`, fraction width without
null-delimiter padding, radical variant slack, large-operator limits, `ssty`
script alternates, `SpaceAfterScript`, row math italic correction, AMSMath
column spacing, display-integral side scripts, and the exact-rational and egui
index overflow checks.

Three rules were corrected on top of that work. A wide hat or tilde uses the
smallest MATH variant that covers the nucleus. AMSMath rows use LaTeX's 12 pt
`\baselineskip`, plus `\jot` in `aligned`. A radical degree is placed by the
bottom of its ink.

## Credits

The 68 dvips color names in `data/dvipsnames.tsv` come from `dvipsnam.def` in
the LaTeX graphics bundle (`drivers.dtx`, `%<*dvipsnames>`). Copyright 1994
David Carlisle and Sebastian Rahtz; 1995–1999 David Carlisle; 2000–2026 The
LaTeX Project. Licensed under LPPL-1.3c or later. The table is a modified
file: a tab-separated extract, rows sorted alphabetically by name, not the
original `dvipsnam.def`. See `NOTICE`.

STIX Two Math remains under the SIL Open Font License 1.1. See `NOTICE`.

## Acknowledgments

This crate was developed with help from AI assistants by SpaceXAI, Anthropic,
OpenAI, Google, and Meta.

## License

The Rust source is MIT OR Apache-2.0. STIX Two Math remains under the SIL Open
Font License 1.1. The dvips color names in `data/dvipsnames.tsv` remain under
LPPL-1.3c or later. See `NOTICE`.
