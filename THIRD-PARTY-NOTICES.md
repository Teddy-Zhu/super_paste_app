# Third-party software notices

Super Paste is licensed under MIT; see `LICENSE`.

## Application source and bundled dependencies

- `src-tauri/vendor/clipboard-core` contains self-contained clipboard bindings from the `super_paste_rs_native` reference project. Its MIT license is included in source at `src-tauri/vendor/clipboard-core/LICENSE` and in release resources at `licenses/MIT-clipboard-core.txt`. It provides low-level clipboard access; application history, capture rules, OCR orchestration, and other business behavior remain in this project.
- `src-tauri/vendor/oar-ocr-core` contains OAR OCR core 0.10.2 under Apache-2.0. The upstream license is retained in source at `src-tauri/vendor/oar-ocr-core/LICENSE` and in release resources at `licenses/Apache-2.0.txt`; the local changes are described in `src-tauri/vendor/oar-ocr-core/PATCHES.md`.
- `src-tauri/src/ocr/layout.rs` is a Rust implementation derived from the layout and reading-order behavior in `esearch-ocr` 8.5.2, licensed under Apache-2.0. The Apache-2.0 text at `licenses/Apache-2.0.txt` also applies to this adaptation. The upstream project is <https://github.com/xushengfeng/eSearch-OCR>; derivation details are documented in source at `src-tauri/src/ocr/LAYOUT-NOTICE.md` and in release resources at `licenses/LAYOUT-NOTICE.md`.
- Rust crates and frontend packages retain their respective upstream licenses. Their dependency manifests and lockfiles identify the versions used by this project.

## ONNX Runtime

Release packages bundle the CPU shared library from ONNX Runtime 1.28.0. The library is downloaded from a pinned official release archive, or built from the pinned official source revision for macOS Intel. Each target's `resources/ort/<target>/` directory contains the upstream `LICENSE`, `ThirdPartyNotices.txt`, and `runtime-manifest.json` with source and SHA-256 details. DirectML and other execution providers are not bundled.

Upstream project: <https://github.com/microsoft/onnxruntime>

## OCR model files

Model weights are not bundled with Super Paste. Model archives are downloaded on demand from the eSearch-OCR 4.0.0 release, as listed in `resources/ocr-models.json`. The eSearch-OCR model package and PaddleOCR-derived model files are distributed under Apache-2.0 by their upstream authors.

Upstream projects:

- <https://github.com/xushengfeng/eSearch-OCR>
- <https://github.com/PaddlePaddle/PaddleOCR>

## Tauri and system libraries

Tauri, its Rust crates, Vue, Vite, and other frontend packages are distributed under their respective upstream licenses. Platform installers rely on system-provided WebView, GTK, WebKitGTK, AppIndicator, and related libraries, which remain under the licenses provided by their respective operating-system distributions.
