#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${HOME}/.local/bin"
desktop_dir="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"
autostart_dir="${XDG_CONFIG_HOME:-${HOME}/.config}/autostart"
desktop_file="${desktop_dir}/sofle-plus2-viewer.desktop"

install_udev=false
install_autostart=false
for arg in "$@"; do
    case "$arg" in
        --udev) install_udev=true ;;
        --autostart) install_autostart=true ;;
        *)
            echo "Unknown argument: $arg" >&2
            echo "Usage: $0 [--udev] [--autostart]" >&2
            exit 2
            ;;
    esac
done

cd "$repo_root"
cargo build --release

install -d "$bin_dir" "$desktop_dir"
install -m 0755 target/release/sofle-plus2-viewer "$bin_dir/sofle-plus2-viewer"

cat > "$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Name=Sofle Plus 2 Viewer
Comment=Live XCMKB Sofle Plus 2 keyboard viewer
Exec=${bin_dir}/sofle-plus2-viewer
Terminal=false
Categories=Utility;
StartupNotify=false
EOF

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi

if "$install_autostart"; then
    install -d "$autostart_dir"
    cp "$desktop_file" "$autostart_dir/sofle-plus2-viewer.desktop"
fi

if "$install_udev"; then
    rule='SUBSYSTEM=="hidraw", ATTRS{idVendor}=="fc32", ATTRS{idProduct}=="0287", TAG+="uaccess", MODE="0660"'
    echo "$rule" | sudo tee /etc/udev/rules.d/70-sofle-plus2-viewer.rules >/dev/null
    sudo udevadm control --reload-rules
    sudo udevadm trigger
fi

echo "Installed ${bin_dir}/sofle-plus2-viewer"
echo "Installed ${desktop_file}"
if "$install_autostart"; then
    echo "Installed ${autostart_dir}/sofle-plus2-viewer.desktop"
fi
if ! "$install_udev"; then
    echo "If hidraw permission fails, rerun with: scripts/install-linux.sh --udev"
fi
