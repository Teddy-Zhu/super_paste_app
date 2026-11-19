# Zune JPEG 0.5.15 scoped patch

Source: the crates.io `zune-jpeg` 0.5.15 package, published with SHA-256
`27bc9d5b815bc103f142aa054f561d9187d191692ec7c2d1e2b4737f8dbd7296`.
The upstream manifests, source provenance and MIT, Apache-2.0 and Zlib licenses
are retained. The separate 0.4.21 dependency remains the registry version.

`JpegDecoder::decode_into_cmyk` exports interleaved inverted CMYK for CMYK/YCCK
input through the existing decode, header, metadata and output-size checks.
CMYK uses the existing four-component passthrough. YCCK uses the same native
YCbCr-to-RGB kernel as ordinary decoding, then expands each row backwards in
the caller's output to retain inverted CMY and K. It adds no full-frame or
heap-allocated scanline intermediate. The new YCCK-to-CMYK output explicitly
selects the RGB kernel even if a previous header read selected RGBA, including
callers selecting the new output through the existing decoder options.

Each byte is 255 minus conventional ink coverage; K is not alpha. This is the
same source convention used by Skia's JPEG codec:
https://skia.googlesource.com/skia/+/main/src/codec/SkJpegCodec.cpp
Consumers invert all four components for an ICC CMYK transform, or use the
existing rounded channel-times-K RGB conversion for unprofiled/canonical
pixels. Applying the source CMYK profile after discarding K would be invalid.

`icc_profile_chunks` returns a cloneable iterator borrowing validated ICC
payloads in sequence order. Its fixed marker table detects zero, duplicate,
missing and inconsistent sequences before returning any payload. The
application calculates the total length, enforces its 4 MiB display limit,
reserves the actual retained-byte budget, then assembles once. Budget or
allocation failures remain retryable decode failures; an oversized profile
remains display metadata failure, without breaking canonical pixels.

APP2 parsing also detects impossible sequence numbers, inconsistent counts,
duplicates and excess chunks before retaining their payloads. The first such
error clears the already-retained ICC chunks and stops accumulating later ICC
payloads. The parser still checks and consumes every payload's bytes, so it
does not mask truncated segments. This preserves the existing invalid-profile
rejection while bounding retention to at most 255 chunks, even for repeated
invalid APP2 markers. The reader's small per-segment working buffer remains.

The legacy `icc_profile` accessor shares the same validation and allocates its
complete output once, replacing geometric growth from a 1000-byte buffer.
Both accessors accept 255 valid chunks, correcting the upstream off-by-one
rejection. The one-byte sequence/count fields and ordered assembly are
specified in ICC.1:2010, annex B.4:
https://www.color.org/specification/ICC1v43_2010-12.pdf

The ordinary RGB/RGBA conversion paths are unchanged. No upstream checks,
features or tests were removed. This patch and its consumers are uncompiled
and unverified.
