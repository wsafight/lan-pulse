#!/usr/bin/env bash
set -euo pipefail

readonly PACKAGE_NAME="com.lanpulse.mobile"
readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
readonly REPOSITORY_ROOT="$(cd -- "${SCRIPT_DIR}/.." >/dev/null 2>&1 && pwd)"
readonly DEBIAN_ARM64_ADB="${HOME}/.local/share/android-platform-tools-ubuntu-34.0.4/adb"

fail() {
    printf 'collect-diagnostics: %s\n' "$1" >&2
    exit 1
}

resolve_adb() {
    if [[ -n "${LANPULSE_ADB:-}" ]]; then
        printf '%s' "$LANPULSE_ADB"
    elif [[ -x "$DEBIAN_ARM64_ADB" ]]; then
        printf '%s' "$DEBIAN_ARM64_ADB"
    elif command -v adb >/dev/null 2>&1; then
        command -v adb
    else
        fail "adb was not found; set LANPULSE_ADB to a working adb executable"
    fi
}

copy_app_file() {
    local remote_path="$1"
    local destination="$2"
    if ! "$ADB" exec-out run-as "$PACKAGE_NAME" cat "$remote_path" >"$destination"; then
        rm -f -- "$destination"
        printf 'No app log at %s\n' "$remote_path" >&2
    fi
}

desktop_state_directory() {
    if [[ -n "${XDG_STATE_HOME:-}" ]]; then
        printf '%s' "$XDG_STATE_HOME"
    else
        printf '%s/.local/state' "$HOME"
    fi
}

readonly ADB="$(resolve_adb)"
timestamp="$(date '+%Y%m%d-%H%M%S')"
output_directory="${1:-${REPOSITORY_ROOT}/diagnostics/${timestamp}}"
mkdir -p -- "$output_directory"

device_count="$($ADB devices | awk 'NR > 1 && $2 == "device" { count += 1 } END { print count + 0 }')"
[[ "$device_count" -eq 1 ]] || fail "expected exactly one authorized Android device, found $device_count"

serial="$($ADB get-serialno)"
printf 'serial=%s\ncollected_at=%s\n' "$serial" "$(date --iso-8601=seconds)" \
    >"${output_directory}/manifest.txt"

copy_app_file files/diagnostics/lanpulse-mobile.log \
    "${output_directory}/lanpulse-mobile.log"
copy_app_file files/diagnostics/lanpulse-mobile.previous.log \
    "${output_directory}/lanpulse-mobile.previous.log"

"$ADB" logcat -d -v epoch 'LanPulse:I' 'AndroidRuntime:E' '*:S' \
    >"${output_directory}/android-logcat.log"
"$ADB" shell dumpsys power >"${output_directory}/android-power.txt"
"$ADB" shell dumpsys connectivity >"${output_directory}/android-connectivity.txt"
"$ADB" shell dumpsys media_session >"${output_directory}/android-media-session.txt"
"$ADB" shell dumpsys package "$PACKAGE_NAME" >"${output_directory}/android-package.txt"

desktop_state="$(desktop_state_directory)/lanpulse"
for name in lanpulse-desktop.log lanpulse-desktop.previous.log; do
    if [[ -f "${desktop_state}/${name}" ]]; then
        cp -- "${desktop_state}/${name}" "${output_directory}/${name}"
    fi
done
journalctl --user -u lanpulse-desktop.service --since '-30 minutes' --no-pager \
    >"${output_directory}/desktop-journal.log"

printf 'Diagnostics collected in %s\n' "$output_directory"
