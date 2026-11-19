# Scoped local patch

Source: published `wl-clipboard-rs` 0.9.5, repository
<https://github.com/YaLTeR/wl-clipboard-rs>, commit
`16b63adcbd2191a8c29c2a467342fc24b4ea82dc`.
The original MIT and Apache-2.0 licenses remain in this directory.

`Watcher::from_connection` accepts a caller-owned Wayland connection. Common
protocol initialization and watcher construction are factored into shared
helpers; the existing constructors retain their behavior. The application can
register a CLOEXEC socket duplicate before either protocol roundtrip and shut
down this private transport when initialization is cancelled. It must not use
the GPUI connection or any connection shared with unrelated objects.

The upstream 0.9.5 handling of late KWin/XWayland MIME advertisements is retained.
Copy/paste protocol behavior, seat selection, event cancellation, offer ownership
and native-library features are otherwise unchanged.

The application uses a nonblocking Linux AF_UNIX connect. EAGAIN is reported
without retry; it does not represent a pending TCP-style EINPROGRESS connection.
See <https://man7.org/linux/man-pages/man2/connect.2.html>.
