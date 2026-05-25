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
