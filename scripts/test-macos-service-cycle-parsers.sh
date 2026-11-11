#!/bin/bash
# Pure fixture checks for the shell-side emergency harness. These deliberately
# cover the `(*)` record emitted while a macOS network service is disabled.

set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
orchestrator="$script_dir/run-macos-service-cycle-live.sh"
runner="$script_dir/macos-service-cycle-live-runner.sh"

if grep -Fq '/bin/launchctl submit' "$orchestrator"; then
    echo 'live harness regressed to restartable launchctl submit jobs' >&2
    exit 1
fi
grep -Fq '/bin/launchctl bootstrap system' "$orchestrator"
grep -Fq '/usr/bin/plutil -insert KeepAlive -bool NO' "$orchestrator"
grep -Fq 'refusing to run twice' "$runner"

service_mapping_matches() {
    local expected_service=$1
    local expected_interface=$2
    awk \
        -v expected_service="$expected_service" \
        -v expected_interface="$expected_interface" '
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

enabled_fixture='An asterisk (*) denotes that a network service is disabled.
(1) USB 10/100/1000 LAN
(Hardware Port: USB 10/100/1000 LAN, Device: en5)

(2) Wi-Fi
(Hardware Port: Wi-Fi, Device: en0)'

disabled_fixture='An asterisk (*) denotes that a network service is disabled.
(1) USB 10/100/1000 LAN
(Hardware Port: USB 10/100/1000 LAN, Device: en5)

(*) Wi-Fi
(Hardware Port: Wi-Fi, Device: en0)'

printf '%s\n' "$enabled_fixture" | service_mapping_matches 'Wi-Fi' en0
printf '%s\n' "$disabled_fixture" | service_mapping_matches 'Wi-Fi' en0

if printf '%s\n' "$disabled_fixture" | service_mapping_matches 'Wi-Fi' en5; then
    echo 'fixture parser accepted the wrong interface' >&2
    exit 1
fi
if printf '%s\n' "$disabled_fixture" | service_mapping_matches 'Ethernet' en0; then
    echo 'fixture parser accepted the wrong service' >&2
    exit 1
fi

echo 'macOS service-cycle parser fixtures passed'
