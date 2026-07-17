#!/bin/bash
# launchd-owned wrapper for the ignored Rust live acceptance test. It records a
# durable exit status and only disarms the independent watchdog after the exact
# production cycle has restored and verified all captured state.

set -u
umask 077

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <state-dir>" >&2
    exit 64
fi
if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
    echo "the live service-cycle runner must run as root" >&2
    exit 77
fi

state_dir=$1
for required in watchdog-pid interface-name service-name test-filter acknowledgement; do
    if [[ ! -f $state_dir/$required ]]; then
        echo "live runner state is incomplete: $required" >&2
        exit 66
    fi
done
watchdog_pid=$(sed -n '1p' "$state_dir/watchdog-pid")
interface=$(sed -n '1p' "$state_dir/interface-name")
service=$(sed -n '1p' "$state_dir/service-name")
test_filter=$(sed -n '1p' "$state_dir/test-filter")
ack=$(sed -n '1p' "$state_dir/acknowledgement")
test_binary="$state_dir/nd300-live-service-cycle-test"
status_tmp="$state_dir/.test-exit-status.tmp"
status_file="$state_dir/test-exit-status"

# Defense in depth: even if a future launchd configuration accidentally
# restarts this wrapper, a durable terminal marker makes a second destructive
# invocation impossible.
if [[ -e $status_file || -e $state_dir/disarm ]]; then
    echo "live service-cycle runner already reached a terminal state; refusing to run twice" >&2
    exit 75
fi

record_status() {
    local status=$1
    printf '%s\n' "$status" > "$status_tmp"
    mv "$status_tmp" "$status_file"
}

# shellcheck disable=SC2329 # Invoked asynchronously by the trap below.
on_signal() {
    record_status 130
    exit 130
}
trap on_signal HUP INT TERM

if [[ ! -x $test_binary ]]; then
    echo "live test binary is not executable: $test_binary" >&2
    record_status 66
    exit 66
fi

export ND300_LIVE_MACOS_SERVICE_CYCLE_ACK=$ack
export ND300_LIVE_MACOS_SERVICE_CYCLE_WATCHDOG_PID=$watchdog_pid
export ND300_LIVE_MACOS_SERVICE_CYCLE_EXPECTED_INTERFACE=$interface
export ND300_LIVE_MACOS_SERVICE_CYCLE_EXPECTED_SERVICE=$service
export RUST_BACKTRACE=1

# Prove the launchd-owned runner is alive, then leave enough time for the
# invoking controller to receive the state directory before Wi-Fi is touched.
: > "$state_dir/runner-ready"
sleep 10

"$test_binary" \
    "$test_filter" \
    --exact \
    --ignored \
    --nocapture \
    --test-threads=1
status=$?
record_status "$status"
if [[ $status -eq 0 ]]; then
    : > "$state_dir/disarm"
fi
exit "$status"
