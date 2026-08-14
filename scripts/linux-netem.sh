#!/usr/bin/env bash
set -euo pipefail

readonly DRY_RUN="${LANPULSE_NETEM_DRY_RUN:-0}"

usage() {
    cat <<'USAGE'
Usage:
  linux-netem.sh show <iface>
  linux-netem.sh clear <iface>
  linux-netem.sh loss <iface> <percent>
  linux-netem.sh jitter <iface> <jitter-ms> [base-delay-ms]
  linux-netem.sh burst-loss <iface> <percent> <correlation-percent>
  linux-netem.sh reorder <iface> <percent> [correlation-percent]
  linux-netem.sh duplicate <iface> <percent>
  linux-netem.sh pause <iface> <duration-ms>
  linux-netem.sh preset <iface> <name>

Presets:
  jitter-1ms, jitter-5ms, jitter-10ms, jitter-20ms
  loss-0.1, loss-1, loss-3
  burst-loss-1, reorder-1, duplicate-1
  pause-100, pause-500, pause-1000

Set LANPULSE_NETEM_DRY_RUN=1 to print tc commands without changing qdisc state.
Run real qdisc changes as root. The script intentionally does not call sudo.
USAGE
}

fail() {
    printf 'linux-netem: %s\n' "$1" >&2
    exit 1
}

print_command() {
    printf '+'
    printf ' %q' "$@"
    printf '\n'
}

run_command() {
    print_command "$@"
    if [[ "$DRY_RUN" != "1" ]]; then
        "$@"
    fi
}

require_command() {
    if [[ "$DRY_RUN" == "1" ]]; then
        return
    fi
    command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

require_root() {
    if [[ "$DRY_RUN" == "1" ]]; then
        return
    fi
    [[ "${EUID:-$(id -u)}" -eq 0 ]] || fail "qdisc changes require root"
}

require_iface() {
    local iface="${1:-}"
    [[ -n "$iface" ]] || fail "missing network interface"
    [[ "$iface" =~ ^[[:alnum:]_.:-]+$ ]] || fail "invalid interface: $iface"
}

normalize_percent() {
    local value="${1:-}"
    [[ "$value" =~ ^([0-9]+([.][0-9]+)?|[.][0-9]+)%?$ ]] || fail "invalid percent: $value"
    printf '%s%%' "${value%\%}"
}

normalize_ms() {
    local value="${1:-}"
    value="${value%ms}"
    [[ "$value" =~ ^[0-9]+$ ]] || fail "invalid millisecond value: ${1:-}"
    printf '%s' "$value"
}

replace_netem() {
    local iface="$1"
    shift
    require_command tc
    require_root
    run_command tc qdisc replace dev "$iface" root netem "$@"
}

clear_qdisc() {
    local iface="$1"
    require_command tc
    require_root
    print_command tc qdisc del dev "$iface" root
    if [[ "$DRY_RUN" != "1" ]]; then
        tc qdisc del dev "$iface" root 2>/dev/null || true
    fi
}

show_qdisc() {
    local iface="$1"
    require_command tc
    run_command tc qdisc show dev "$iface"
}

apply_jitter() {
    local iface="$1"
    local jitter_ms
    local base_ms
    jitter_ms="$(normalize_ms "$2")"
    base_ms="$(normalize_ms "${3:-20}")"
    replace_netem "$iface" delay "${base_ms}ms" "${jitter_ms}ms" distribution normal
}

apply_pause() {
    local iface="$1"
    local duration_ms
    local sleep_seconds
    duration_ms="$(normalize_ms "$2")"
    sleep_seconds="$((duration_ms / 1000)).$(printf '%03d' "$((duration_ms % 1000))")"
    replace_netem "$iface" loss 100%
    run_command sleep "$sleep_seconds"
    clear_qdisc "$iface"
}

apply_preset() {
    local iface="$1"
    local preset="$2"
    case "$preset" in
        jitter-1ms) apply_jitter "$iface" 1 ;;
        jitter-5ms) apply_jitter "$iface" 5 ;;
        jitter-10ms) apply_jitter "$iface" 10 ;;
        jitter-20ms) apply_jitter "$iface" 20 ;;
        loss-0.1) replace_netem "$iface" loss 0.1% ;;
        loss-1) replace_netem "$iface" loss 1% ;;
        loss-3) replace_netem "$iface" loss 3% ;;
        burst-loss-1) replace_netem "$iface" loss 1% 75% ;;
        reorder-1) replace_netem "$iface" delay 20ms reorder 1% 50% ;;
        duplicate-1) replace_netem "$iface" duplicate 1% ;;
        pause-100) apply_pause "$iface" 100 ;;
        pause-500) apply_pause "$iface" 500 ;;
        pause-1000) apply_pause "$iface" 1000 ;;
        *) fail "unknown preset: $preset" ;;
    esac
}

main() {
    local command="${1:-}"
    local iface="${2:-}"

    if [[ -z "$command" || "$command" == "-h" || "$command" == "--help" ]]; then
        usage
        return 0
    fi

    require_iface "$iface"
    case "$command" in
        show)
            show_qdisc "$iface"
            ;;
        clear)
            clear_qdisc "$iface"
            ;;
        loss)
            replace_netem "$iface" loss "$(normalize_percent "${3:-}")"
            ;;
        jitter)
            apply_jitter "$iface" "${3:-}" "${4:-20}"
            ;;
        burst-loss)
            replace_netem "$iface" loss "$(normalize_percent "${3:-}")" "$(normalize_percent "${4:-}")"
            ;;
        reorder)
            replace_netem "$iface" delay 20ms reorder "$(normalize_percent "${3:-}")" "$(normalize_percent "${4:-50}")"
            ;;
        duplicate)
            replace_netem "$iface" duplicate "$(normalize_percent "${3:-}")"
            ;;
        pause)
            apply_pause "$iface" "${3:-}"
            ;;
        preset)
            apply_preset "$iface" "${3:-}"
            ;;
        *)
            usage >&2
            fail "unknown command: $command"
            ;;
    esac
}

main "$@"
