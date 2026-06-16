# Pin

A tiny Windows system-tray utility that lets you "pin" any window so it stays on top of other windows. Inspired by [DeskPins](https://efotinis.neocities.org/deskpins/index.html), rewritten in Rust.

## How it works

1. Launch `pin.exe` — a pin icon appears in the system tray.
2. **Left-click the tray icon** — your mouse cursor turns into a pin (`pin_off`). You are now in *selection mode*.
3. **Click any window** — that window becomes topmost and a small pin badge (`pin_on`) appears at the right side of its title bar, just left of the minimize button.
4. **Click the pin badge** — the window is unpinned and the badge disappears.
5. **Right-click the tray icon** — opens a menu with `Unpin all` and `Quit`.
6. In selection mode you can press **Esc** or **right-click** anywhere to cancel.

The pin badge follows the target window as you move/resize it (~60 fps).

## Build

Requires Rust stable, Windows target.

```bash
cargo build --release
```

The binary is at `target/release/pin.exe`.

Tagged releases are built by GitHub Actions. Pushing a `v*` tag builds the
Windows release binary, packages it as an MSI installer with WiX/cargo-wix, and
uploads `pin-<tag>-windows-x64.msi` to the GitHub Release.

To build the MSI locally on Windows:

```powershell
cargo build --release --target x86_64-pc-windows-msvc
pwsh ./scripts/gen-license-rtf.ps1
cargo wix --no-build --target x86_64-pc-windows-msvc
```

The installer shows the full AGPL license text and offers an optional desktop
shortcut (enabled by default).

Cross-compile from Linux/WSL works too:

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

## Test

```bash
cargo test
```

There are ~20 unit tests covering icon decoding, geometry math, and pin-set state transitions; plus two Windows-only integration tests that exercise `SetWindowPos`/`GetWindowLong` against a real hidden window.

Run with logs:

```bash
set RUST_LOG=debug
cargo run
```

## Architecture

```
src/
  main.rs       — entry point (just calls pin::run)
  lib.rs        — module declarations
  app.rs        — main message loop, owns PinnedSet/Tray/Picker
  tray.rs      — system-tray icon + right-click menu (tray-icon crate)
  selection.rs  — "selection mode": transparent capture window + SetSystemCursor
  overlay.rs    — floating pin_on badge: layered topmost window + 16ms timer
  pinned.rs     — OS-agnostic pin/unpin set (testable with FakeApi)
  win.rs        — Win32 thin wrapper + WindowApi trait for testability
  resources.rs  — embedded PNG/ICO + decode/resize/BGRA helpers
tests/
  integration_pin.rs — Win32 round-trip test of WS_EX_TOPMOST
```

Key Win32 design choices (cross-referenced against DeskPins source for prior art):
- **Selection mode**: 1×1 transparent topmost popup + `SetCapture` to catch the click, **plus** `SetSystemCursor`/`SystemParametersInfo(SPI_SETCURSORS)` to swap the global cursor (the only reliably visible mechanism for a hidden popup).
- **Overlay**: `WS_EX_LAYERED` + `UpdateLayeredWindow` with premultiplied BGRA so the transparent PNG renders correctly. A 16ms `SetTimer` repositions the badge over the target and re-applies `HWND_TOPMOST` if the target has been knocked off the topmost group.
- **No DLL injection / no global hooks** — everything runs in one process via standard Win32 APIs.

## License

[AGPL-3.0-or-later](LICENSE).
