# Sofle Plus 2 Viewer

Native Linux/Windows viewer for the XCMKB Sofle Plus 2.

Features:

- Live raw-HID matrix highlighting
- Active layer display from firmware telemetry
- Touchpad contact visualization for the TPS65 large touchpad
- Always-on-top toggle
- Compact keyboard-only view at small window sizes

Run:

```sh
cargo run
```

Probe the connected board without opening the window:

```sh
cargo run -- --probe
```

The app talks to the board through Vial raw HID, so it can highlight the actual
switch matrix instead of guessing from normal OS key events.

## Prerequisites

Vial does not need to be installed to run this viewer. The viewer talks directly
to the keyboard through the same raw HID interface that Vial uses.

Required:

- Sofle Plus 2 connected by USB
- Firmware with Vial/raw-HID enabled
- Telemetry-enabled firmware for live layer and touchpad tracking
- Close Vial while the viewer is running, because both apps use the same raw HID
  interface

Linux:

- A graphical desktop session, X11 or Wayland
- Permission to read and write the keyboard hidraw device
- Rust stable toolchain if building or installing from this source repo

Windows:

- Windows 10 or newer
- Rust stable toolchain if building or installing from this source repo
- No separate Vial install or driver should be needed for normal HID access

## Keymaps And Layers

The Vial desktop app is not the source of the keymap. The source is the
keyboard firmware and the dynamic keymap data stored on the keyboard.

On startup and when pressing `Reload keymap`, the viewer asks the keyboard for:

- VIA/Vial protocol version
- Number of dynamic keymap layers
- Keycode at each layer/row/column position
- Layer names when the firmware exposes them

Live active layer, pressed keys, and touchpad contact data come from the
telemetry feature added to the firmware. If that telemetry is missing, the
viewer can still read the matrix through the Vial raw-HID fallback, but active
layer tracking is less exact.

## Install

Linux:

```sh
bash scripts/install-linux.sh
```

If hidraw permissions fail on Linux:

```sh
bash scripts/install-linux.sh --udev
```

Windows PowerShell:

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
.\scripts\install-windows.ps1
```

Both installers build the release binary and install a normal app launcher. Close
Vial before starting the viewer, because both applications use the same raw HID
interface.

## Development

Required: Rust stable toolchain.

Useful checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo check
cargo check --target x86_64-pc-windows-msvc
```

The Windows target check validates the Windows HID backend, but a real Windows
machine is still needed to run the installed `.exe` against the keyboard.

## License

MIT. See [LICENSE](LICENSE).
