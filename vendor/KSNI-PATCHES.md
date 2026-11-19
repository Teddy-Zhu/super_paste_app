# ksni 0.3.6 activation token patch

`ksni/` contains the crates.io 0.3.6 source, licensed under the Unlicense;
its original `UNLICENSE`, manifest, documentation and tests are retained.

The `org.kde.StatusNotifierItem.ProvideXdgActivationToken` method forwards a
single-use desktop token to the new default-empty Tray callback. Input is
limited to 4,096 bytes. Existing activation, menu, watcher and shutdown
behavior remains upstream's. The application retains at most one token and
consumes it on the next action, forwarding it only for window activation.

KDE sends this method before both icon activation and menu-item activation:
[systemtray.cpp](https://github.com/KDE/plasma-workspace/blob/master/applets/systemtray/systemtray.cpp),
[statusnotifieritemsource.cpp](https://github.com/KDE/plasma-workspace/blob/master/applets/systemtray/statusnotifieritemsource.cpp).

These changes have not yet been compiled or tested.
