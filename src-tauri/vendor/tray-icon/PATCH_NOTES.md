# Vendored tray-icon patch

This directory contains `tray-icon` 0.21.3 from crates.io with the macOS
portion of [tauri-apps/tray-icon#365](https://github.com/tauri-apps/tray-icon/pull/365)
backported.

macOS 27 stops forwarding status-item clicks to the tray view while an
`NSMenu` is permanently attached. The patch retains the menu separately and
attaches it only for the duration of `performClick`, preserving left-click
window toggling and right-click menu behavior. The retained menu is cloned out
of its `RefCell` first so menu callbacks may update it without a re-entrant
borrow panic.

Remove the `[patch.crates-io]` override in the parent `Cargo.toml` once a
compatible upstream `tray-icon` release containing the fix is adopted.
