# Local changes to oar-ocr-core 0.10.2

Upstream: https://github.com/greatv/oar-ocr/tree/v0.10.2
License: Apache-2.0; the upstream LICENSE is retained alongside this file.

The application needs the original eSearch ONNX model archives. Their exported
input tensor names must be obtained from ONNX Runtime metadata.

- `src/core/inference/ort_infer_builders.rs`: when no input name is supplied,
  select the first declared model input instead of assuming `x`. A model with no
  input returns a configuration error. Explicit caller-supplied names retain
  their original behavior.
- `src/models/detection/db.rs`: the DB model builder requests automatic input-name
  selection rather than explicitly passing `x`.
- `src/predictors/text_detection.rs`: add `predict_shared` so callers retaining an
  input image can pass an `Arc<RgbImage>` into detection without cloning its full
  pixel buffer. The existing owned `predict` API remains available.

No model weights, operators, normalization kernels, or detection thresholds are
changed by this patch. Application-specific recognition shapes, dictionary
validation, image budgets, cancellation, and decoding remain in `src/ocr` in the
application, using the public OAR processors and model constructors.
