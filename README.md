# Pin

A tiny Windows system-tray utility that lets you "pin" any window so it stays on top of other windows. Inspired by [DeskPins](https://efotinis.neocities.org/deskpins/index.html), rewritten in Rust.

## How it works

1. Launch `pin.exe` — a pin icon appears in the system tray.
2. **Left-click the tray icon** — your mouse cursor turns into a pin (`pin_off`). You are now in *selection mode*.
3. **Click any window** — that window becomes topmost and a small pin badge (`pin_on`) appears at the right side of its title bar, just left of the minimize button.
4. **Click the pin badge** — the window is unpinned and the badge disappears.
5. **Right-click the tray icon** — opens a menu with `Unpin all`, `Start with Windows`, and `Quit`.
6. In selection mode you can press **Esc** or **right-click** anywhere to cancel.

The pin badge follows the target window as you move/resize it (~60 fps), and
rescales when the window moves to a monitor with a different DPI.
The `Start with Windows` tray item toggles whether Pin starts automatically when
you sign in to Windows.

Settings are stored in `%APPDATA%\Pin\pin.ini` (older versions kept `pin.ini`
next to `pin.exe`; it is migrated automatically).

## Install

Download from [Releases](https://github.com/Sqhh99/pin/releases):

- `pin-<version>-windows-x64-setup.exe` — per-user installer (no admin rights
  needed). Offers desktop shortcut and autostart options, and closes a running
  Pin gracefully when upgrading or uninstalling.
- `pin-<version>-windows-x64-portable.zip` — just `pin.exe`; unzip and run.

Each asset has a matching `.sha256` checksum file.

## Build

Requires Rust stable (1.80+), Windows target.

```bash
cargo build --release
```

The binary is at `target/release/pin.exe`. `build.rs` embeds the icon, an
application manifest (per-monitor-v2 DPI awareness), and version info taken
from `Cargo.toml`.

To build the installer locally on Windows (requires [Inno Setup 6](https://jrsoftware.org/isinfo.php)):

```powershell
cargo build --release --target x86_64-pc-windows-msvc
.github\scripts\build-installer.ps1 -Version 0.1.12
```

Cross-compile from Linux/WSL works too (install `mingw-w64` so the resource
compiler can embed the icon/manifest; without it the build still succeeds but
prints a warning):

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

## Release

`Cargo.toml` is the single source of truth for the version:

1. Bump `version` in `Cargo.toml` and commit.
2. Push a matching tag, e.g. `git tag v0.1.12 && git push origin v0.1.12`.

The `release` workflow fails if the tag and `Cargo.toml` disagree, verifies the
version info embedded in `pin.exe`, then publishes the installer, the portable
zip, and their checksums to the GitHub Release.

## Test

```bash
cargo test
```

Unit tests cover the platform-free domain layer (overlay geometry, pinnable
window filter, pinned-set transitions, settings/`Run` parsing) and asset
decoding — these also run on Linux (`cargo test --lib`). Windows-only
integration tests exercise `SetWindowPos`/`IsWindow`/DWM frame bounds against
real hidden windows.

Run with logs (debug builds keep a console window):

```bash
set RUST_LOG=debug
cargo run
```

## Architecture

```
src/
  main.rs          entry point (logger, then pin::run)
  lib.rs           module layering
  domain/          platform-free logic, unit-tested on any host
    geometry.rs    Rect, overlay placement, overlay visibility rule
    window.rs      WindowId, WindowApi trait, pinnable-window filter
    pinned.rs      PinnedSet<E>: pin/unpin bookkeeping (entries are RAII)
    settings.rs    pin.ini parsing, Run-value parsing, reconcile rules
  resources.rs     embedded PNGs, cached decode/resize, BGRA conversion
  win/             thin Win32 wrappers, no app logic
    api.rs         RealWindowApi + window queries (DPI, frame bounds, hit test)
    gdi.rs         RAII DC/bitmap guards, BGRA DIB sections
    cursor.rs      cursors from PNG, system-cursor override/restore
    registry.rs    RAII registry key
    instance.rs    single-instance mutex
  ui/              windows Pin owns (views; they post notifications to the app)
    picker.rs      selection mode: capture window + system cursor swap
    overlay.rs     pin badge: layered topmost window + tracking timer
    tray.rs        tray icon + menu
  autostart.rs     settings file + HKCU Run glue
  app/             controller: owns all state, the only place it changes
    mod.rs         startup, message loop, shutdown
    messages.rs    view notifications -> AppEvent
    state.rs       App::handle(AppEvent)
tests/
  integration_pin.rs — Win32 round-trip tests against real hidden windows
installer/
  pin.iss          Inno Setup script (per-user)
```

Key Win32 design choices (cross-referenced against DeskPins source for prior art):
- **Selection mode**: 1×1 transparent topmost popup + `SetCapture` to catch the click, **plus** `SetSystemCursor`/`SystemParametersInfo(SPI_SETCURSORS)` to swap the global cursor (the only reliably visible mechanism for a hidden popup). The cursors are also restored at startup and from the panic hook, so a crash can't leave them swapped.
- **Overlay**: `WS_EX_LAYERED` + `UpdateLayeredWindow` with premultiplied BGRA so the transparent PNG renders correctly. A 16ms `SetTimer` keeps the badge on the target's caption (using DWM's visible frame bounds) and re-applies `HWND_TOPMOST` if the target has been knocked off the topmost group; it only calls `SetWindowPos`/`ShowWindow` when something changed.
- **One owner for state**: views never change pin state themselves. A badge click posts `UNPIN_REQUESTED`; the app clears topmost and drops the badge.
- **Graceful exit**: the app window is a hidden top-level window, so `WM_CLOSE` (installer), `WM_ENDSESSION` (logoff, Restart Manager) unpin everything and remove the tray icon.
- **No DLL injection / no global hooks** — everything runs in one process via standard Win32 APIs.

## License

[AGPL-3.0-or-later](LICENSE).
