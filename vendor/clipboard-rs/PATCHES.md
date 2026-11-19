# clipboard-rs 0.3.5

The source is from https://crates.io/crates/clipboard-rs/0.3.5.

The crate SHA-256 is `d1c988a897ea030e32f0668c90b0192800e0c561b3d941fc366f9ad5a1bf26ba`.

The original MIT license is retained in `LICENSE`.

- Windows uses a message-only window on the calling thread during writes. A single operation publishes all representations and reports write failures. Each image can be written as PNG and a native bitmap, with either representation usable on its own. Invalid CF_HTML byte offsets return an error.
- macOS prepares the AppKit items before changing the clipboard and writes files and other representations in one operation. Missing files and failed native writes are reported to the caller.
