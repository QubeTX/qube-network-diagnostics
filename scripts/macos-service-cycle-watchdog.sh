#!/bin/bash
# Emergency, offline-safe restore watchdog for the ignored live macOS service-
# cycle acceptance test. This script is launched by launchd as root before the
# test process starts, so losing the controlling terminal or network cannot
# prevent the network service from being re-enabled.

set -u
umask 077

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <state-dir>" >&2
    exit 64
fi
if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
    echo "the service-cycle watchdog must run as root" >&2
    exit 77
fi

state_dir=$1
networksetup=/usr/sbin/networksetup

if [[ ! -d $state_dir \
    || ! -f $state_dir/dns.mode \
    || ! -f $state_dir/search.mode \
    || ! -f $state_dir/service-name \
    || ! -f $state_dir/interface-name ]]; then
    echo "watchdog state is incomplete: $state_dir" >&2
    exit 66
fi
service=$(sed -n '1p' "$state_dir/service-name")
interface=$(sed -n '1p' "$state_dir/interface-name")
if [[ ! $interface =~ ^en[0-9]+$ \
    || -z $service \
    || $service == *$'\n'* \
    || $service == *$'\r'* ]]; then
    echo "watchdog state contains an unsafe service/interface" >&2
    exit 78
fi

log() {
    printf '%s %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*"
}

service_is_mapped() {
    "$networksetup" -listnetworkserviceorder 2>/dev/null | awk \
        -v expected_service="$service" \
        -v expected_interface="$interface" '
            /^\((\*|[0-9]+)\) / {
                current = substr($0, index($0, " ") + 1)
                if (substr(current, 1, 1) == "*") {
                    current = substr(current, 2)
                }
                next
            }
            current == expected_service && index($0, "Device: " expected_interface ")") {
                found = 1
            }
            END { exit found ? 0 : 1 }
        '
}

service_is_enabled() {
    local line
    while IFS= read -r line; do
        if [[ $line == "$service" ]]; then
            return 0
        fi
        if [[ $line == "*$service" ]]; then
            return 1
        fi
    done < <("$networksetup" -listallnetworkservices 2>/dev/null)
    return 1
}

capture_setting() {
    local kind=$1
    local mode_file=$2
    local values_file=$3
    local output
    local command

    if [[ $kind == dns ]]; then
        command=-getdnsservers
    else
        command=-getsearchdomains
    fi
    if ! output=$("$networksetup" "$command" "$service" 2>/dev/null); then
        return 1
    fi
    if [[ $output == "There aren't any "* ]]; then
        printf '%s\n' automatic > "$mode_file"
        : > "$values_file"
    else
        printf '%s\n' values > "$mode_file"
        printf '%s\n' "$output" > "$values_file"
    fi
}

setting_matches_snapshot() {
    local kind=$1
    local current_mode="$state_dir/.watchdog-${kind}.mode"
    local current_values="$state_dir/.watchdog-${kind}.values"
    capture_setting "$kind" "$current_mode" "$current_values" || return 1
    cmp -s "$state_dir/${kind}.mode" "$current_mode" \
        && cmp -s "$state_dir/${kind}.values" "$current_values"
}

restore_setting() {
    local kind=$1
    local command
    local mode
    local value
    local args=()

    if [[ $kind == dns ]]; then
        command=-setdnsservers
    else
        command=-setsearchdomains
    fi
    mode=$(sed -n '1p' "$state_dir/${kind}.mode")
    args+=("$command" "$service")
    if [[ $mode == automatic ]]; then
        args+=(empty)
    elif [[ $mode == values ]]; then
        while IFS= read -r value || [[ -n $value ]]; do
            [[ -n $value ]] && args+=("$value")
        done < "$state_dir/${kind}.values"
        if [[ ${#args[@]} -eq 2 ]]; then
            return 1
        fi
    else
        return 1
    fi
    "$networksetup" "${args[@]}" >/dev/null 2>&1
}

default_route_matches() {
    local route_interface
    route_interface=$(/sbin/route -n get default 2>/dev/null \
        | awk '/interface:/{print $2; exit}')
    [[ $route_interface == "$interface" ]]
}

restore_once() {
    local ok=true

    if ! service_is_mapped; then
        log "ERROR: service mapping no longer matches $service -> $interface"
        return 1
    fi
    if ! "$networksetup" -setnetworkserviceenabled "$service" on >/dev/null 2>&1; then
        log "WARN: re-enable command failed for service $service"
        ok=false
    fi
    if ! setting_matches_snapshot dns; then
        log "WARN: restoring captured DNS server state"
        restore_setting dns || ok=false
    fi
    if ! setting_matches_snapshot search; then
        log "WARN: restoring captured DNS search-domain state"
        restore_setting search || ok=false
    fi
    [[ $ok == true ]]
}

fully_restored() {
    service_is_enabled \
        && service_is_mapped \
        && setting_matches_snapshot dns \
        && setting_matches_snapshot search \
        && default_route_matches
}

# shellcheck disable=SC2329 # Invoked asynchronously by the trap below.
on_signal() {
    log "watchdog received a signal; forcing one final local restore"
    restore_once || true
    exit 130
}
trap on_signal HUP INT TERM

log "watchdog armed for service $service on $interface (pid $$)"

# Give the Rust action time to complete normally. If it succeeds, its runner
# writes `disarm`; otherwise this independent process begins recovery.
sleep 30

attempt=1
while [[ $attempt -le 60 ]]; do
    restore_once || true
    if fully_restored; then
        : > "$state_dir/watchdog-restored"
        log "service is enabled with exact DNS/search state and route on $interface"
        if [[ -f $state_dir/disarm ]]; then
            log "Rust acceptance test passed; watchdog disarmed"
            exit 0
        fi
        if [[ -f $state_dir/test-exit-status ]]; then
            test_status=$(sed -n '1p' "$state_dir/test-exit-status")
            log "Rust acceptance test exited with status ${test_status:-unknown}; independent recovery is verified"
            exit 0
        fi
    else
        log "restore verification not complete (attempt $attempt/60)"
    fi
    attempt=$((attempt + 1))
    sleep 10
done

# A wedged runner must not remain capable of issuing a late disable after the
# watchdog gives up waiting. Remove the launchd-owned test job first, then make
# one final bounded series of idempotent restore attempts.
if [[ -f $state_dir/test-label ]]; then
    test_label=$(sed -n '1p' "$state_dir/test-label")
    case "$test_label" in
        com.qubetx.nd300.service-cycle-test.*)
            log "watchdog deadline reached; terminating wedged test job system/$test_label"
            /bin/launchctl bootout "system/$test_label" >/dev/null 2>&1 || true
            : > "$state_dir/watchdog-terminated-runner"
            ;;
        *)
            log "WARN: refusing malformed test label in watchdog state"
            ;;
    esac
fi

for final_attempt in 1 2 3 4 5 6; do
    restore_once || true
    if fully_restored; then
        : > "$state_dir/watchdog-restored"
        log "final recovery verified after terminating the test job"
        exit 0
    fi
    log "final recovery not complete (attempt $final_attempt/6)"
    sleep 10
done

log "ERROR: watchdog exhausted recovery attempts; manual state retained at $state_dir"
exit 1
