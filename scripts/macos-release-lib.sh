#!/usr/bin/env bash

# Return success when vtool's build metadata declares the expected macOS
# deployment floor. Modern Mach-O files use LC_BUILD_VERSION/minos, while
# binaries targeting older Intel releases can use the legacy
# LC_VERSION_MIN_MACOSX/version pair.
# This library is sourced by the validator and fixture test; ShellCheck also
# analyzes it as a standalone file.
# shellcheck disable=SC2329
macos_build_info_has_deployment_floor() {
    local expected_floor=$1

    awk -v expected_floor="$expected_floor" '
        $1 == "Load" && $2 == "command" {
            active_command = ""
            next
        }
        $1 == "cmd" {
            active_command = $2
            next
        }
        active_command == "LC_BUILD_VERSION" && $1 == "minos" {
            saw_floor = 1
            if ($2 == expected_floor) {
                matched_floor = 1
            }
            next
        }
        active_command == "LC_VERSION_MIN_MACOSX" && $1 == "version" {
            saw_floor = 1
            if ($2 == expected_floor) {
                matched_floor = 1
            }
            next
        }
        END {
            exit !(saw_floor && matched_floor)
        }
    '
}
