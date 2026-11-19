# Supplemental dependency notices

Some locked crates declare a license but omit the repository-root license files
from their published archive. `index.json` supplies materials for those exact
releases; packaging requires the name, version, registry source, declared license
and published Cargo manifest SHA-256 to match. Each material also has its own
SHA-256 and public origin. Packaging reads these local files without downloading
notices, and records their origins and basis in the staged notice inventory.

Most materials are copied without modification from the repository revision in
the release's `.cargo_vcs_info.json`. Repository-root attribution files accompany
the license texts where available. Published Cargo manifests retain the release's
authors and original license declaration.

Some releases require an explicit standard-text basis:

- `simd_helpers 0.1.0` and `seahash 4.1.0`: the declared MIT license text comes
  from a pinned SPDX license-data revision. Its copyright placeholders remain
  unchanged; they are not represented as an upstream copyright notice. Original
  release manifests, README files and library sources retain available attribution.
- `mac 0.1.1` and `pathfinder_geometry 0.5.1`: the published dual-license
  declarations permit Apache-2.0. Its standard text accompanies the original
  release attribution and declarations. The inventory preserves both declared
  alternatives; it does not claim that an absent MIT notice was recovered.
- Newer objc2 crates provide a licensing statement that links to standard
  license texts. The inventory includes that statement and the applicable
  MIT or Apache-2.0 text from the pinned SPDX revision.
- `file_icon_provider 1.0.2` declares MIT in its manifest. The inventory
  includes the standard text and the original project manifest.

When dependencies change, obtain the corresponding release and upstream materials,
review their basis and update the explicit binding and hashes. Do not add a
license-expression-only fallback or copy another package's copyright notice.
Unknown missing materials and integrity mismatches continue to stop packaging.

The content-addressed `.txt` files are marked `-text` in this project's
`.gitattributes`, preserving upstream bytes across Windows and Unix checkouts.
