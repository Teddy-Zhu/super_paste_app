# Cosmic Text 0.19.0 scoped patch

Source: crates.io `cosmic-text` 0.19.0, upstream revision
`c24886c2471e5606587c46090cd25dbbf209186b`.
The published crate SHA-256 is
`be17b688510d934ce13f48a2beba700e11583e281e0fda99c22bb256a14eda73`.
Original package metadata, source, tests, fonts and their licenses are retained.
The upstream license is MIT OR Apache-2.0; see `LICENSE-MIT` and `LICENSE-APACHE`.

`ShapeLine::new_with_direction` and `build_with_direction` accept an optional
`unicode_bidi::Level` and pass it to the existing `BidiInfo` analysis. Existing
constructors keep automatic direction. This exposes the native paragraph base
level to GPUI without inserting control characters, changing source offsets,
or creating an alternate shaping implementation or cache.

`Attrs::shaping_span` preserves GPUI's explicit native font-run boundaries.
Its default zero value keeps the ordinary compatibility policy; GPUI assigns
one nonzero identity per font run and retains that identity in fallback spans.
`AttrsOwned` carries the same scalar through the existing attribute list. The
compatibility check can consequently distinguish two runs in the same family
without perturbing font size or repurposing the glyph's application metadata.
The ASCII and grapheme shaping paths both use that compatibility check. The
complex path merges grapheme starts with explicit native span transitions,
including UTF-8 boundaries inside an extended grapheme. This preserves a
requested size/font cut instead of silently applying the first character's
attributes to the complete grapheme. Ordinary zero-span attributes retain their
grapheme boundary policy. Both streams borrow source ranges and visit only
attribute spans overlapping the current word; no boundary vector is retained.
The ASCII compatibility scan likewise visits only overlapping spans and checks
uncovered default-attribute intervals, rather than treating those intervals as
having the explicit span's font. Shaping starts with the first actual attributes,
avoiding redundant empty runs.

For these nonzero native spans and directly adjacent default spans,
`shape_fallback` supplies the original line's pre/post source to Harfrust. Harfrust keeps its existing bounded context (five
codepoints on either side); no neighboring source strings or extra glyphs are
inserted into the run. This allows font/size cuts to retain the shaper's Arabic
joining context while the run itself remains a distinct shaping buffer. The
reused Unicode buffer clears its context on return to the existing scratch
storage. Default-zero callers outside native span boundaries retain their previous
context policy.

The optional `shape-run-cache` key does not contain adjacent source context.
Native spans and their directly adjacent default spans therefore share the
same context predicate and use the existing uncached shaping path if
that feature is enabled. GPUI already owns the complete source/font layout
cache; this patch adds neither another context cache nor context strings to
word keys. This does not implement soft-wrap shaping or choose line breaks.

GPUI continues to split paragraph separators before shaping. This patch does
not implement inline isolate ranges, line-specific bidi reordering, justification
or selection affinity. Those remain separate GPUI/Base responsibilities.

Compiled through the desktop integration build. Native-style regression checks
link its exact Cosmic rlib; results and remaining platform/UI limits are recorded
in the project README. This is not complete coverage of every scoped patch.
On update, review the constructor and
allocation-reuse paths together with bidi analysis, attribute compatibility,
fallback shaping, the optional word cache and the GPUI call sites.

Native `ital`/`slnt` instances use the existing font cache, extended with style.
Static faces and equivalent style locations reuse their original Arc. Matching
recognizes supported native style axes; fallback, shape/layout glyphs and glyph
cache keys preserve the request. Harfrust, metrics and both Swash raster/outline
paths use the same full normalized location, including `wght`. An actual native
slant prevents an additional FAKE_ITALIC transform. Legacy CacheKey::new retains
its native-face semantics; layout explicitly supplies its requested style.
Default oblique uses OpenType slnt=-14; italic prefers ital=1 and preserves the
designed default slant of that italic instance. Validated authored angles now
travel through attributes, matching, fallback, shaping, glyph/cache keys and
Swash. The existing instance cache normalizes equivalent clamped axis endpoints.
Width and signed native angle ranges precede weight matching; categorical fontdb
queries must not override this angular order. GPUI retains its shared italic /
oblique scale, mapping italic strength 1 to 11 degrees. Synthetic raster and
outline paths use the same authored shear, with no extra shaping or cache.
The GPUI caller rejects singular +/-90 degree requests and limits large raster
allocations; bitmap transformations retain Swash's existing limitations.
Regression coverage reuses Inter in memory with an axis tag and range remap;
original font files and their licenses remain unchanged.
