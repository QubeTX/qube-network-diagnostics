#!/bin/bash
# Arm two independent launchd jobs for the destructive-but-restorable macOS
# service-cycle acceptance test: an emergency watchdog and the ignored Rust
# test itself. The command returns after both jobs are owned by launchd, before
# Wi-Fi is disabled, so controller/network loss cannot cancel recovery.

set -euo pipefail
umask 077

ack_expected=I_UNDERSTAND_ND300_WILL_BRIEFLY_DISABLE_MY_ACTIVE_MACOS_NETWORK_SERVICE
test_filter=actions::fix::stages::macos_service_cycle_tests::live_exact_service_cycle_restores_all_captured_state

if [[ $# -ne 3 ]]; then
    echo "usage: sudo ND300_LIVE_MACOS_SERVICE_CYCLE_ACK=$ack_expected $0 <test-binary> <bsd-interface> <service-name>" >&2
    exit 64
fi
if [[ $(uname -s) != Darwin ]]; then
    echo "the live service-cycle qualification is macOS-only" >&2
    exit 69
fi
if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
    echo "the live service-cycle qualification must run as root" >&2
    exit 77
fi
if [[ ${ND300_LIVE_MACOS_SERVICE_CYCLE_ACK:-} != "$ack_expected" ]]; then
    echo "required acknowledgement is missing; refusing active-service mutation" >&2
    exit 78
fi

test_binary=$1
interface=$2
service=$3
networksetup=/usr/sbin/networksetup
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

if [[ ! $interface =~ ^en[0-9]+$ ]]; then
    echo "refusing non-physical-looking interface: $interface" >&2
    exit 78
fi
if [[ $service == *$'\n'* || $service == *$'\r'* || -z $service ]]; then
    echo "service name contains unsafe control characters" >&2
    exit 78
fi
if [[ ! -f $test_binary || -L $test_binary || ! -x $test_binary ]]; then
    echo "test binary is not executable: $test_binary" >&2
    exit 66
fi
test_binary=$(cd "$(dirname "$test_binary")" && pwd)/$(basename "$test_binary")
for helper in macos-service-cycle-watchdog.sh macos-service-cycle-live-runner.sh; do
    if [[ ! -f $script_dir/$helper || -L $script_dir/$helper || ! -x $script_dir/$helper ]]; then
        echo "required live-test helper is not a regular executable: $script_dir/$helper" >&2
        exit 66
    fi
done

current_interface=$(/sbin/route -n get default 2>/dev/null \
    | awk '/interface:/{print $2; exit}')
if [[ $current_interface != "$interface" ]]; then
    echo "default route is on ${current_interface:-none}, not expected physical interface $interface" >&2
    exit 78
fi

if ! "$networksetup" -listnetworkserviceorder 2>/dev/null | awk \
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
    '; then
    echo "service/interface mapping does not match $service -> $interface" >&2
    exit 78
fi
if "$networksetup" -listallnetworkservices 2>/dev/null \
    | awk -v expected="$service" '$0 == "*" expected { found = 1 } END { exit found ? 0 : 1 }'; then
    echo "service is currently disabled: $service" >&2
    exit 78
fi

stamp=$(date -u '+%Y%m%dT%H%M%SZ')
state_dir=$(/usr/bin/mktemp -d /var/tmp/nd300-service-cycle.XXXXXX)
chmod 700 "$state_dir"

# Root-owned immutable copies prevent a user-writable workspace path or test
# binary from being swapped after validation but before launchd executes it.
watchdog_script="$state_dir/macos-service-cycle-watchdog.sh"
runner_script="$state_dir/macos-service-cycle-live-runner.sh"
test_binary_copy="$state_dir/nd300-live-service-cycle-test"
/usr/bin/install -m 0700 -o root -g wheel \
    "$script_dir/macos-service-cycle-watchdog.sh" "$watchdog_script"
/usr/bin/install -m 0700 -o root -g wheel \
    "$script_dir/macos-service-cycle-live-runner.sh" "$runner_script"
/usr/bin/install -m 0700 -o root -g wheel "$test_binary" "$test_binary_copy"

capture_setting() {
    local kind=$1
    local command
    local output

    if [[ $kind == dns ]]; then
        command=-getdnsservers
    else
        command=-getsearchdomains
    fi
    output=$("$networksetup" "$command" "$service")
    if [[ $output == "There aren't any "* ]]; then
        printf '%s\n' automatic > "$state_dir/${kind}.mode"
        : > "$state_dir/${kind}.values"
    else
        printf '%s\n' values > "$state_dir/${kind}.mode"
        printf '%s\n' "$output" > "$state_dir/${kind}.values"
    fi
}

capture_setting dns
capture_setting search
"$networksetup" -getinfo "$service" > "$state_dir/service-info.before"
"$networksetup" -listnetworkserviceorder > "$state_dir/service-order.before"
/sbin/route -n get default > "$state_dir/default-route.before"
printf '%s\n' "$service" > "$state_dir/service-name"
printf '%s\n' "$interface" > "$state_dir/interface-name"
printf '%s\n' "$test_filter" > "$state_dir/test-filter"
printf '%s\n' "$ack_expected" > "$state_dir/acknowledgement"

watchdog_label="com.qubetx.nd300.service-cycle-watchdog.${stamp}.$$"
test_label="com.qubetx.nd300.service-cycle-test.${stamp}.$$"
watchdog_log="$state_dir/watchdog.log"
test_log="$state_dir/test.log"
watchdog_plist="$state_dir/watchdog.plist"
test_plist="$state_dir/test.plist"

create_one_shot_plist() {
    local plist=$1
    local label=$2
    local program=$3
    local log_path=$4

    /usr/bin/plutil -create xml1 "$plist"
    /usr/bin/plutil -insert Label -string "$label" "$plist"
    /usr/bin/plutil -insert ProgramArguments \
        -json "[\"$program\",\"$state_dir\"]" "$plist"
    /usr/bin/plutil -insert RunAtLoad -bool YES "$plist"
    /usr/bin/plutil -insert KeepAlive -bool NO "$plist"
    /usr/bin/plutil -insert ProcessType -string Background "$plist"
    /usr/bin/plutil -insert StandardOutPath -string "$log_path" "$plist"
    /usr/bin/plutil -insert StandardErrorPath -string "$log_path" "$plist"
    /bin/chmod 600 "$plist"
}

create_one_shot_plist "$watchdog_plist" "$watchdog_label" "$watchdog_script" "$watchdog_log"
create_one_shot_plist "$test_plist" "$test_label" "$runner_script" "$test_log"

# `launchctl submit` creates a keep-alive job on current macOS and can restart a
# completed destructive test. Explicit RunAtLoad + KeepAlive=false plists make
# both jobs provably one-shot while still placing them in launchd's independent
# system domain before any network mutation.
/bin/launchctl bootstrap system "$watchdog_plist"

watchdog_pid=
for _ in 1 2 3 4 5; do
    watchdog_pid=$(/bin/launchctl print "system/$watchdog_label" 2>/dev/null \
        | awk '/pid =/{print $3; exit}')
    [[ $watchdog_pid =~ ^[0-9]+$ ]] && break
    sleep 1
done
if [[ ! $watchdog_pid =~ ^[0-9]+$ ]] || ! kill -0 "$watchdog_pid" 2>/dev/null; then
    echo "watchdog did not become a live independent process; refusing cycle" >&2
    /bin/launchctl bootout "system/$watchdog_label" >/dev/null 2>&1 || true
    exit 70
fi

printf '%s\n' "$watchdog_label" > "$state_dir/watchdog-label"
printf '%s\n' "$watchdog_pid" > "$state_dir/watchdog-pid"
printf '%s\n' "$test_label" > "$state_dir/test-label"

if ! /bin/launchctl bootstrap system "$test_plist"; then
    : > "$state_dir/disarm"
    echo "could not submit the Rust test job; watchdog remains armed until it verifies state" >&2
    exit 70
fi

runner_ready=false
for _ in 1 2 3 4 5; do
    if [[ -f $state_dir/runner-ready ]]; then
        runner_ready=true
        break
    fi
    sleep 1
done
if [[ $runner_ready != true ]]; then
    /bin/launchctl bootout "system/$test_label" >/dev/null 2>&1 || true
    : > "$state_dir/disarm"
    echo "launchd test runner did not become ready; test was cancelled before network mutation" >&2
    exit 70
fi

printf '%s\n' "$state_dir"
echo "watchdog: system/$watchdog_label (pid $watchdog_pid)"
echo "test:     system/$test_label"
echo "The launchd jobs now own the test and recovery; temporary controller loss is safe."
