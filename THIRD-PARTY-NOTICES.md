# Sources and licenses

This project is distributed under the MIT license in `LICENSE`.

Package staging collects the target-filtered Rust dependency notices. Published
crates that omit their repository-root license files use the explicit, release-bound
materials in `packaging/dependency-notices/`; its README records provenance and
standard-text exceptions. The staging inventory preserves original declarations,
material origins and hashes. Missing or mismatched materials stop packaging.

## Adapted application logic and resources

Domain classification, plain-text restoration, history/tag behavior, copy-statistics aggregation,
the color expression, OCR model catalog, model download/integrity management, remote/AI OCR response rules,
AI-usage aggregation, the WebDAV v1 document contract and reconciliation rules, MCP protocol definitions and session handling, settings layouts, physical shortcut recording, diagnostic redaction policy, preview provider ordering, file/link/contact/table recognition and lexeme-preserving JSON formatting, the ORT preparation script, source-window tracking, native focus event listeners,
KWin scripts and source-icon resolution are adapted from Super Paste, reference commit
`fc90106d5a25233fe45a6ec9facd1ad3ffc4823b`. Copyright (c) 2026 Teddy Zhu, MIT.
The custom color picker's HSL conversion and draft/commit rules are adapted from the same application's `ColorPicker.vue`.
The copies in this directory are independently maintained; no parent source or resource is a build dependency.
The application PNG, ICO and ICNS icons are independently copied from the same application's resources.

HTML preview parsing uses html5ever and markup5ever_rcdom (MIT/Apache-2.0),
matching Base's 0.27/0.3 parser family. RTF tokenization uses rtf-grimoire 0.2.1
(MIT), with this project's controlled-document adapter; it does not round-trip
through HTML. Encoding conversion uses encoding_rs (Apache-2.0/MIT/BSD-3-Clause),
and numeric CSS color parsing uses csscolorparser 0.9.1 (MIT/Apache-2.0).

Native list counter definitions are adapted from WebKit's `counterStyles.css`,
and the CJK/Ethiopic algorithms from `CSSRegisteredCounterStyle.cpp`, pinned at
`50320fd3b3eee47ce31b9d53e902797c215fafd3`. Copyright (C) 2023 Apple Inc.
(definitions) and (C) 2022-2024 Apple Inc. (formatting), BSD-2-Clause.
`vendor/css-counter-styles/` retains upstream source, provenance and the offline
static-table generator. Complete source/binary notices are distributed in
`licenses/webkit-counter-styles-definitions.txt` and
`licenses/webkit-counter-styles-formatting.txt`. No browser engine or C++ code
is compiled or required at runtime for these counters.

Display-only ICC/CICP and PNG gamma/chromaticity conversion uses moxcms 0.8.1 (BSD-3-Clause OR Apache-2.0),
already resolved by the image dependency. Profile data remains separate from
canonical pixels; transformed display pixels use the existing image resource service.
Its extended-range feature preserves narrow-range RGB through floating-point
conversion. PNG color-chunk CRC checking uses the already-resolved crc32fast 1.5.5
(MIT OR Apache-2.0); metadata inspection does not inflate image data.

Native 8/16-bit APNG composition uses png 0.18.1 (MIT OR Apache-2.0),
already resolved by image. `vendor/png` retains the published source and both
licenses; provenance and the scoped reusable-row budget fix are documented in
`vendor/png/PATCHES.md`. The application compositor is independently implemented
against the PNG specification and shares the existing image resource service.

Borrowed JPEG decoding uses zune-jpeg 0.5.15 and zune-core 0.5.3, already
resolved by image. The vendored JPEG source retains its MIT, Apache-2.0 and
Zlib licenses and provenance. Its limited CMYK/YCCK sample-preservation and
borrowed ICC APIs are documented in `vendor/zune-jpeg/PATCHES.md`; ordinary
JPEG pixel conversion is retained.

## clipboard-core

`vendor/clipboard-core` was copied from Super Paste's `src-tauri/vendor/clipboard-core` at the
reference commit above. The parent notice identifies its source as the `super_paste_rs_native`
reference project's self-contained clipboard bindings. Its original MIT license and attribution
are retained in `vendor/clipboard-core/LICENSE`.

Local adaptations expose native RGBA buffers and encoded Wayland offers separately, and add
an owned write entry point. This removes the application-level native image → PNG → decoded
image round trip and an extra owned-buffer clone. `clipboard-rs::to_rgba8()` still allocates
its returned buffer; native platform internals and system clipboard transfers may perform
additional required copies. These changes do not imply zero-copy clipboard access.

Wayland reads additionally share a per-operation cancellation signal across the watcher event
wait and offer data pipes. Stopping an owned watch or abandoning a direct read wakes blocked
pipe I/O without introducing a timer, polling thread, or intermediate clipboard payload.

The published clipboard-rs 0.3.5 source and MIT license are retained in
`vendor/clipboard-rs`. Its scoped Win32 and AppKit fixes are documented in
`vendor/clipboard-rs/PATCHES.md`.

`vendor/wl-clipboard-rs` retains the published 0.9.5 source and its MIT/Apache-2.0
licenses. The scoped caller-owned connection constructor is documented in
`vendor/wl-clipboard-rs/PATCHES.md`. Application initialization retains a temporary
CLOEXEC duplicate of its private connection so cancellation can interrupt both
protocol roundtrips; the GPUI connection is not shared or shut down.

## OCR model catalog

`resources/ocr-models.json` contains download metadata. No model weights are bundled here.
The catalog refers to the eSearch-OCR 4.0.0 release. Model weights and their redistribution
notices are not included in the application packages.

Every catalog archive has a pinned SHA-256. The three PP-OCR v6 digests match
the upstream GitHub release asset metadata. The other sixteen were calculated
from the complete archives served by the corresponding 4.0.0 release asset
endpoints, with the published byte sizes checked before recording the digests.

- Upstream: <https://github.com/xushengfeng/eSearch-OCR>

## OCR inference and layout

`crates/app-core/src/ocr/local.rs` and `layout.rs` preserve the Super Paste adaptations of
eSearch-OCR 8.5.2 text decoding, reading order and paragraph grouping. They retain the
Apache-2.0 attribution in `crates/app-core/src/ocr/LAYOUT-NOTICE.md`; the license text is
in `licenses/Apache-2.0.txt`. The local worker uses the shared image service and an owned
RGB buffer at the OAR API boundary, rather than accepting and decoding a second image payload.

`vendor/oar-ocr-core` contains OAR OCR core 0.10.2, Apache-2.0, copied from Super Paste's
vendored source at the reference commit. Its upstream license and scoped patch description
remain at `vendor/oar-ocr-core/LICENSE` and `vendor/oar-ocr-core/PATCHES.md`. Its input-name
selection and shared-image detector API are retained. No weights or ONNX Runtime binaries
are bundled in this copy.

- Upstream: <https://github.com/greatv/oar-ocr/tree/v0.10.2>

## GPUI window backends

`vendor/gpui-pre`, `vendor/gpui-pre-linux`, `vendor/gpui-pre-macos`,
`vendor/gpui-pre-windows`, `vendor/gpui-pre-apple` and `vendor/gpui-pre-wgpu` contain the crates.io
0.3.11 releases, Apache-2.0. Their package metadata identifies Zed revision
`f816e1f9263e3a8ef63c4baaa02e1441ce1c6038`. Upstream notices remain in the
vendored files; the Apache-2.0 license text is in `licenses/Apache-2.0.txt`.
The local scope and maintenance rules are recorded in `vendor/GPUI-PATCHES.md`.

`vendor/taffy` contains the crates.io Taffy 0.15.1 source (MIT), revision
`0064bd54cbe38bf1d8bcd6ec6ab0704d45fb048a`, already resolved by GPUI. The original
copyright and permission notice are retained in `vendor/taffy/LICENSE`, obtained
from the upstream v0.15.1 tag because the published crate omits that file.
The limited outside-list-marker layout patch is recorded in `vendor/taffy/PATCHES.md`.

`vendor/cosmic-text` contains the crates.io Cosmic Text 0.19.0 release under
MIT OR Apache-2.0, with its original source, tests, fonts and license files retained.
Its scoped native paragraph-direction API and source provenance are recorded in
`vendor/cosmic-text/PATCHES.md`.

Source icons use native pixel buffers or existing PNG files where available.
The `x-win` compatibility fallback exposes a PNG data URL; its base64 decode is
an explicit third-party boundary, not an internal application image format.

## Rust dependencies

Linux's XEmbed tray fallback uses gtk-rs GTK 3 bindings 0.18.2 (MIT),
with GLib/GObject, GDK and GdkPixbuf bindings from the same release family.
The gtk-rs license and contributor copyright notice are retained in
`licenses/gtk-rs/` and copied into the package resources.
The deprecated GtkStatusIcon type is bound locally against gtk-sys; GTK owns
the XEmbed protocol, scaling and native menus. GTK/GLib and their native
dependencies remain system libraries, subject to their upstream licenses
(including LGPL). They are declared in the Linux package dependencies;
this project does not bundle their shared libraries.

`vendor/gpui-component` and `vendor/gpui-base` contain their crates.io 0.9.0
releases under Apache-2.0, with upstream licenses retained. Scoped native UI and text-layout changes and
maintenance requirements are documented in `vendor/KIT-PATCHES.md`.

Dependencies retain their upstream licenses. Kit 0.9.0 and the desktop package
are pinned by the independent Cargo.lock. The packaging entry copies this lockfile
and shared license texts, and collects original Rust dependency license and notice
materials into `licenses/rust/`. `NOTICE.txt` and `index.json` identify each package
and its source materials; identical bytes share a SHA-256-named file in `texts/`.
The collection follows the target-filtered Cargo resolve closure of normal and
build dependencies, excluding dev-only edges. Workspace feature resolution can
include additional packages; this is not an inventory of linked binary contents.
Missing declarations or primary license texts, missing declared files and empty
materials stop packaging. Declared license alternatives are retained without
selection or rewriting. Material Design Icons attribution is copied separately
to `licenses/resources/icons-NOTICE.md`; ORT keeps its upstream runtime notices.
Generated materials have not yet been verified in a release package. Distribution
review must also cover Rust toolchain components, native libraries, copied assets
and any upstream conditions not represented by Cargo's graph or file naming.
