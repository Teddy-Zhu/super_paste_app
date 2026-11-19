# WebKit counter styles

Source: WebKit, commit `50320fd3b3eee47ce31b9d53e902797c215fafd3`. BSD-2-Clause; retained source headers and complete notices in `../../licenses/webkit-counter-styles-*.txt`.

- `counterStyles.css`: https://raw.githubusercontent.com/WebKit/WebKit/50320fd3b3eee47ce31b9d53e902797c215fafd3/Source/WebCore/css/counterStyles.css
  SHA-256: `b6d86567156d9cf5a4bad090a74f2f511704ddc5000fc429ce5d7d7a6661a332`.
- `CSSRegisteredCounterStyle.cpp`: https://raw.githubusercontent.com/WebKit/WebKit/50320fd3b3eee47ce31b9d53e902797c215fafd3/Source/WebCore/css/CSSRegisteredCounterStyle.cpp
  SHA-256: `cf604af90d5bd8cd954149acd7c8c6fdde0046dfd07ef9c02da4024a65bb9208`.

`generate.py` adapts the predefined data to a static Rust table. Run it from any directory after a reviewed upstream update; it uses only Python standard library and does not download anything. No runtime CSS engine, registry or build-time generator is introduced.

The bounded CJK and Ethiopic formatters in `crates/app-core/src/preview/rich/lists/complex.rs` are Rust adaptations of the retained formatting source; integer handling is widened for the application domain. Application content remains native GPUI text. Stylesheet-defined custom counters are not accepted by the clipboard sanitizer; unknown names use CSS decimal fallback.
