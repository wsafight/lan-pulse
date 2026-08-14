#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

run_case() {
    LANPULSE_NETEM_DRY_RUN=1 "${SCRIPT_DIR}/linux-netem.sh" "$@"
}

assert_contains() {
    local output="$1"
    local expected="$2"
    if [[ "$output" != *"$expected"* ]]; then
        printf 'Expected output to contain:\n%s\n\nActual output:\n%s\n' "$expected" "$output" >&2
        exit 1
    fi
}

output="$(run_case loss eth0 1)"
assert_contains "$output" "+ tc qdisc replace dev eth0 root netem loss 1%"

output="$(run_case jitter wlan0 5 20)"
assert_contains "$output" "+ tc qdisc replace dev wlan0 root netem delay 20ms 5ms distribution normal"

output="$(run_case preset wlan0 pause-100)"
assert_contains "$output" "+ tc qdisc replace dev wlan0 root netem loss 100%"
assert_contains "$output" "+ sleep 0.100"
assert_contains "$output" "+ tc qdisc del dev wlan0 root"

output="$(run_case preset enp3s0 reorder-1)"
assert_contains "$output" "+ tc qdisc replace dev enp3s0 root netem delay 20ms reorder 1% 50%"

if run_case loss 'bad iface' 1 >/dev/null 2>&1; then
    printf 'Expected invalid interface to fail\n' >&2
    exit 1
fi

printf 'linux-netem dry-run tests passed\n'
