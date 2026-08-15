#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_directory=$(cd -- "${script_directory}/.." && pwd)
binary_directory="${HOME}/.local/bin"
data_directory="${XDG_DATA_HOME:-${HOME}/.local/share}"

cd "${project_directory}"
cargo build --release --locked --workspace \
    --bin lanpulse-app \
    --bin lanpulse-service

install -Dm755 target/release/lanpulse-app "${binary_directory}/lanpulse-app"
install -Dm755 target/release/lanpulse-service "${binary_directory}/lanpulse-service"
install -Dm644 \
    desktop-app/assets/com.lanpulse.LanPulse.desktop \
    "${data_directory}/applications/com.lanpulse.LanPulse.desktop"
install -Dm644 \
    desktop-app/assets/com.lanpulse.LanPulse.svg \
    "${data_directory}/icons/hicolor/scalable/apps/com.lanpulse.LanPulse.svg"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${data_directory}/applications"
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && [[ -f "${data_directory}/icons/hicolor/index.theme" ]]; then
    gtk-update-icon-cache --force --quiet "${data_directory}/icons/hicolor"
fi

echo "LanPulse installed for ${USER}:"
echo "  ${binary_directory}/lanpulse-app"
echo "  ${binary_directory}/lanpulse-service"
