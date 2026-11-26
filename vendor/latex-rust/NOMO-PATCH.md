# Nomo LaTeX compatibility and glyph fallback patch

This directory contains the production sources, data, font and notices from
`latex-rust` 3.0.3 at upstream commit `6ae94f6de78df10ab56448360b3e5dc77c11e5f4`.
Upstream: https://github.com/jscarr64/latex-rust

Nomo's changes (2026-10-29):

- Add `layout_with_glyph_fallback`, a borrowed resolver for glyphs missing from
  the math face. The original layout APIs retain their strict behavior.
- Carry a shared host outline through glyph boxes and both clone paths. The host
  supplies real advance, ink bounds and font units; TeX applies script scaling,
  fractions, spacing, colors and baselines as before. Empty outlines represent
  measured whitespace.
- Emit host outlines through the SVG backend. They must be trusted numeric path
  data from a font parser, never arbitrary document XML. Optional PNG/egui entry
  points explicitly reject these outlines; Nomo rasterizes the SVG with resvg.
- Keep host caches outside this crate, scoped to one formula. No system fonts,
  document text, callbacks or new global cache are retained here.
- Treat a backslash followed by a tab or line ending inside math as the same
  control space as `\ `, preserving alignment row breaks (`\\`).
- Support `\middle` inside delimiter groups, with all delimiters sized to
  the same group extent and vertical glyph assembly when needed.
- Support native modulo commands (`\bmod`, `\pmod`). The editor's shared
  compatibility renderer handles additional KaTeX syntax.

The small manifest omits upstream development dependencies, examples and benches;
production dependency versions and features are unchanged. The root Cargo patch
pins this copy, and it is excluded from workspace members. Keep the upstream
licenses, font OFL, data headers and NOTICE with this source. Before updating the
dependency, review the patched source files and remove this extension if the
upstream library supplies a compatible fallback API.
