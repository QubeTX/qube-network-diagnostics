#!/usr/bin/env bash

set -euo pipefail

script_dir=$(
    unset CDPATH
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd
)
# shellcheck source=scripts/macos-release-lib.sh
. "${script_dir}/macos-release-lib.sh"

assert_accepts() {
    local expected_floor=$1
    local label=$2

    if ! macos_build_info_has_deployment_floor "$expected_floor"; then
        echo "expected deployment-floor fixture to pass: ${label}" >&2
        exit 1
    fi
}

assert_rejects() {
    local expected_floor=$1
    local label=$2

    if macos_build_info_has_deployment_floor "$expected_floor"; then
        echo "expected deployment-floor fixture to fail: ${label}" >&2
        exit 1
    fi
}

assert_accepts 11.0 "modern arm64 LC_BUILD_VERSION" <<'EOF'
Load command 10
      cmd LC_BUILD_VERSION
  cmdsize 32
 platform MACOS
    minos 11.0
      sdk 15.5
EOF

assert_accepts 10.12 "legacy Intel LC_VERSION_MIN_MACOSX" <<'EOF'
Load command 9
      cmd LC_VERSION_MIN_MACOSX
  cmdsize 16
  version 10.12
      sdk 15.5
EOF

assert_rejects 10.12 "wrong legacy floor" <<'EOF'
Load command 9
      cmd LC_VERSION_MIN_MACOSX
  cmdsize 16
  version 10.13
      sdk 15.5
EOF

assert_rejects 10.12 "unscoped matching version" <<'EOF'
Load command 9
      cmd LC_LOAD_DYLIB
  cmdsize 48
  version 10.12
EOF

assert_rejects 10.12 "missing deployment command" <<'EOF'
Load command 9
      cmd LC_UUID
  cmdsize 24
     uuid 00000000-0000-0000-0000-000000000000
EOF

echo "macOS release metadata parser fixtures passed."
