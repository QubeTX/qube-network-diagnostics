#!/usr/bin/env bash
# Exercise published Unix update channels inside an isolated disposable home.
# This script never runs diagnostics or network-repair actions.

set -euo pipefail

if [[ $# -ne 6 ]]; then
    echo "usage: $0 <managed|cargo> <baseline> <expected> <nd300|speedqx> <strategy> <hide-cargo>" >&2
    exit 2
fi

channel=$1
baseline=$2
expected=$3
invoker=$4
expected_strategy=$5
hide_cargo=$6

case "$channel" in
    managed | cargo) ;;
    *) echo "unsupported channel: $channel" >&2; exit 2 ;;
esac
case "$invoker" in
    nd300 | speedqx) ;;
    *) echo "unsupported update entrypoint: $invoker" >&2; exit 2 ;;
esac
case "$hide_cargo" in
    true | false) ;;
    *) echo "hide-cargo must be true or false" >&2; exit 2 ;;
esac

work_root=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/nd300-unix-public.XXXXXXXX")
test_home="$work_root/home"
evidence_dir=${EVIDENCE_DIR:-"$work_root/evidence"}
mkdir -p "$test_home" "$evidence_dir"

original_home=$HOME
original_path=$PATH
export HOME="$test_home"
export CARGO_HOME="$HOME/.cargo"
export XDG_CONFIG_HOME="$HOME/.config"
export CARGO_TARGET_DIR="$work_root/cargo-target"
export RUSTUP_HOME=${RUSTUP_HOME:-"$original_home/.rustup"}
export ND300_INSTALL_DIR="$CARGO_HOME"
export ND300_NO_MODIFY_PATH=1
export PATH="$CARGO_HOME/bin:$original_path"

bin_dir="$CARGO_HOME/bin"
receipt="$XDG_CONFIG_HOME/nd300/nd300-receipt.json"
mkdir -p "$bin_dir" "$XDG_CONFIG_HOME"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

assert_version() {
    local binary=$1
    local wanted=$2
    local output
    [[ -x "$binary" ]] || fail "missing executable $binary"
    output=$("$binary" --version)
    echo "$output"
    [[ "$output" == *" $wanted" ]] || fail "$binary reported '$output', expected $wanted"
}

assert_json() {
    local path=$1
    local update_available=$2
    local strategy=$3
    python3 - "$path" "$update_available" "$strategy" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
expected_update = sys.argv[2] == "true"
expected_strategy = sys.argv[3]
payload = json.loads(path.read_text())
assert payload.get("success") is True, payload
assert payload.get("update_available") is expected_update, payload
if expected_strategy != "-":
    assert payload.get("strategy") == expected_strategy, payload
PY
}

assert_legacy_failure_json() {
    local path=$1
    python3 - "$path" <<'PY'
import json
import pathlib
import sys

payload = json.loads(pathlib.Path(sys.argv[1]).read_text())
assert payload.get("success") is False, payload
assert payload.get("update_available") is True, payload
attempts = {item.get("strategy") for item in payload.get("attempts", [])}
assert {"installer_curl", "installer_wget"}.issubset(attempts), payload
PY
}

write_pair_fingerprint() {
    local path=$1
    python3 - "$bin_dir/nd300" "$bin_dir/speedqx" "$path" <<'PY'
import hashlib
import json
import pathlib
import sys

payload = {}
for name, raw_path in zip(("nd300", "speedqx"), sys.argv[1:3]):
    path = pathlib.Path(raw_path)
    payload[name] = {
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "size": path.stat().st_size,
    }
pathlib.Path(sys.argv[3]).write_text(json.dumps(payload, sort_keys=True) + "\n")
PY
}

assert_receipt() {
    local wanted=$1
    python3 - "$receipt" "$CARGO_HOME" "$wanted" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
prefix = pathlib.Path(sys.argv[2]).resolve()
wanted = sys.argv[3]
payload = json.loads(path.read_text())
assert payload["provider"]["source"] == "cargo-dist", payload
assert payload["source"]["app_name"] == "nd300", payload
assert payload["source"]["owner"] == "QubeTX", payload
assert payload["source"]["name"] == "qube-network-diagnostics", payload
assert payload["install_layout"] == "cargo-home", payload
assert pathlib.Path(payload["install_prefix"]).resolve() == prefix, payload
assert payload["binaries"] == ["nd300", "speedqx"], payload
assert payload["version"] == wanted, payload
PY
}

echo "Installing public baseline $baseline through $channel on $(uname -sm)..."
if [[ "$channel" == managed ]]; then
    baseline_installer="$work_root/nd300-installer-$baseline.sh"
    curl --proto '=https' --tlsv1.2 -fLsS \
        "https://github.com/QubeTX/qube-network-diagnostics/releases/download/v${baseline}/nd300-installer.sh" \
        -o "$baseline_installer"
    sh "$baseline_installer"
    assert_receipt "$baseline"
else
    cargo_executable=$(command -v cargo)
    "$cargo_executable" install nd300 --version "=$baseline" --force --locked
    [[ ! -e "$receipt" ]] || fail "Cargo baseline unexpectedly created a cargo-dist receipt"
fi

assert_version "$bin_dir/nd300" "$baseline"
assert_version "$bin_dir/speedqx" "$baseline"
write_pair_fingerprint "$evidence_dir/baseline-fingerprint.json"

if [[ "$hide_cargo" == true ]]; then
    # Keep the hosted runner's unrelated Rust toolchain from changing the route
    # under test. This forces managed installs through the verified archive
    # transaction and legacy installs through their historical shell fallback.
    export PATH="$bin_dir:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
fi

set +e
"$bin_dir/$invoker" update --json --no-color | tee "$evidence_dir/update.json"
update_exit=${PIPESTATUS[0]}
set -e

recovery=not-needed
if [[ "$expected_strategy" == legacy_safe_fallback ]]; then
    if [[ $update_exit -eq 0 ]]; then
        # The historical client may start succeeding if its host shell changes.
        # Accept that, but still require a valid successful update payload.
        assert_json "$evidence_dir/update.json" true '-'
        recovery=legacy-updater-succeeded
    else
        # Released clients cannot be changed retroactively. Prove their known
        # shell incompatibility leaves the validated pair and receipt untouched,
        # then exercise the documented versionless same-channel recovery path.
        assert_legacy_failure_json "$evidence_dir/update.json"
        assert_version "$bin_dir/nd300" "$baseline"
        assert_version "$bin_dir/speedqx" "$baseline"
        assert_receipt "$baseline"
        write_pair_fingerprint "$evidence_dir/after-failed-update-fingerprint.json"
        cmp -s \
            "$evidence_dir/baseline-fingerprint.json" \
            "$evidence_dir/after-failed-update-fingerprint.json" \
            || fail "legacy updater failure mutated the installed binary pair"

        recovery_installer="$work_root/nd300-installer-latest.sh"
        curl --proto '=https' --tlsv1.2 -fLsS \
            'https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-installer.sh' \
            -o "$recovery_installer"
        sh "$recovery_installer"
        recovery=versionless-same-channel-installer
    fi
else
    [[ $update_exit -eq 0 ]] || fail "public updater exited $update_exit"
    assert_json "$evidence_dir/update.json" true "$expected_strategy"
fi

assert_version "$bin_dir/nd300" "$expected"
assert_version "$bin_dir/speedqx" "$expected"

if [[ "$channel" == managed ]]; then
    assert_receipt "$expected"
else
    [[ ! -e "$receipt" ]] || fail "Cargo update crossed into the managed-archive channel"
    if [[ -f "$CARGO_HOME/.crates2.json" ]]; then
        grep -q "nd300 $expected" "$CARGO_HOME/.crates2.json" || fail "Cargo registry does not contain nd300 $expected"
    else
        grep -q "nd300 $expected" "$CARGO_HOME/.crates.toml" || fail "Cargo registry does not contain nd300 $expected"
    fi
fi

leftovers=$(find "$bin_dir" -maxdepth 1 -type f \
    \( -name '.nd300-update-*' -o -name '*.update-old-*' \) -print)
[[ -z "$leftovers" ]] || fail "update transaction left temporary files: $leftovers"

if [[ "$invoker" == nd300 ]]; then
    noop_invoker=speedqx
else
    noop_invoker=nd300
fi
"$bin_dir/$noop_invoker" update --json --no-color | tee "$evidence_dir/already-latest.json"
assert_json "$evidence_dir/already-latest.json" false '-'

"$bin_dir/nd300" uninstall --json --no-color | tee "$evidence_dir/uninstall.json"
python3 - "$evidence_dir/uninstall.json" <<'PY'
import json
import pathlib
import sys

payload = json.loads(pathlib.Path(sys.argv[1]).read_text())
assert payload.get("success") is True, payload
PY

[[ ! -e "$bin_dir/nd300" ]] || fail "nd300 remained after uninstall"
[[ ! -e "$bin_dir/speedqx" ]] || fail "speedqx remained after uninstall"
[[ ! -e "$receipt" ]] || fail "cargo-dist receipt remained after uninstall"

cat > "$evidence_dir/summary.txt" <<EOF
platform=$(uname -sm)
channel=$channel
baseline=$baseline
expected=$expected
entrypoint=$invoker
strategy=$expected_strategy
legacy_recovery=$recovery
paired_update=pass
already_latest=pass
uninstall=pass
EOF

echo "Unix public update matrix case PASS: $channel $baseline -> $expected via $invoker"
