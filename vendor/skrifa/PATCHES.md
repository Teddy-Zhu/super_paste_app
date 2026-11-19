# Skrifa 0.40.0 scoped patch

The source is the crates.io `skrifa` 0.40.0 package, originally published with
SHA-256 `7fbdfe3d2475fbd7ddd1f3e5cf8288a30eb3e5f95832829570cd88115a7434ac`.
Upstream manifests, provenance, licenses and existing tests are retained.
The crate is licensed under MIT OR Apache-2.0; see its license files.

`OutlineGlyph::visit_glyf_points` exposes the original TrueType point indices
needed for OpenType GDEF format 2 ligature carets. It borrows the existing
outline and uses the same unhinted scaler, variation coordinates, composite
transforms and point attachment rules as internal drawing. It preserves the
scaler's fractional coordinates and excludes phantom points. The callback
does not create a second point array or reconstruct indices from drawing paths.
The existing scaler manages temporary workspace and validates outlines.
PostScript outlines return `None` because they have no TrueType point indices.

Only the already-selected 0.40.0 dependency is patched. The separate 0.44.0
dependency remains the registry version. No upstream behavior or tests were
removed. The patch and consumers are uncompiled and unverified.
