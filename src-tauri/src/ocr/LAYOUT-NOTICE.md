# OCR Layout Attribution

`layout.rs` adapts the default text ordering and paragraph grouping behavior used by `esearch-ocr` 8.5.2 (`dist/esearch-ocr.js`, `analyzeLayout` and its helpers). The local CTC decoder in `local.rs` also follows the package's default top-two decode threshold and repeated-token handling. The upstream project is [eSearch-OCR](https://github.com/xushengfeng/eSearch-OCR), distributed under Apache-2.0. The source package is available at [npm: esearch-ocr@8.5.2](https://www.npmjs.com/package/esearch-ocr/v/8.5.2); its license is [LICENSE](https://github.com/xushengfeng/eSearch-OCR/blob/v8.5.2/LICENSE).

This Rust implementation is a local adaptation. It does not bundle upstream JavaScript, models, or other runtime assets. The project-level third-party notices include the applicable Apache-2.0 attribution.
