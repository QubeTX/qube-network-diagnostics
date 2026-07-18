#!/usr/bin/env bash
# Validate the signed universal PKG-in-DMG lifecycle on a native macOS host.

set -euo pipefail

if [[ $# -ne 4 ]]; then
    echo "usage: $0 <dmg> <native-archive> <version> <downgrade-fixture-pkg>" >&2
    exit 64
fi

dmg=$1
archive=$2
version=${3#v}
fixture=$4
: "${APPLE_TEAM_ID:?APPLE_TEAM_ID is required}"
: "${APPLE_INSTALLER_SIGNING_IDENTITY:?APPLE_INSTALLER_SIGNING_IDENTITY is required}"
[[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || exit 64
for required in "$dmg" "${dmg}.sha256" "$archive" "$fixture"; do
    [[ -f $required ]] || {
        echo "missing validation input: $required" >&2
        exit 66
    }
done

runner_temp=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
work_dir=$(mktemp -d "${runner_temp%/}/nd300-pkg-validation.XXXXXX")
mount="${work_dir}/mount"
mkdir "$mount"
mounted=false
cleanup() {
    if [[ $mounted == true ]]; then
        hdiutil detach "$mount" >/dev/null 2>&1 || true
    fi
    rm -rf "$work_dir"
}
trap cleanup EXIT INT TERM

(
    cd "$(dirname "$dmg")"
    shasum -a 256 -c "$(basename "$dmg").sha256"
)
codesign --verify --deep --strict --verbose=4 "$dmg"
dmg_details=$(codesign -d --verbose=4 "$dmg" 2>&1)
grep -Fqx 'Identifier=com.qubetx.nd300.dmg' <<< "$dmg_details"
grep -Fqx "TeamIdentifier=${APPLE_TEAM_ID}" <<< "$dmg_details"
xcrun stapler validate "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg"

hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$mount" "$dmg"
mounted=true
pkg="$mount/nd300.pkg"
[[ -f $pkg && ! -L $pkg ]]
signature=$(pkgutil --check-signature "$pkg")
grep -Fq "$APPLE_INSTALLER_SIGNING_IDENTITY" <<< "$signature"
grep -Fq 'Signed with a trusted timestamp' <<< "$signature"
xcrun stapler validate "$pkg"
spctl --assess --type install --verbose=4 "$pkg"

# A fresh PKG is the newest user intent. Seed a supported managed-archive
# channel and prove package postinstall removes the exact old pair and receipt.
mkdir -p "$HOME/.cargo/bin" "$HOME/.config/nd300" "${work_dir}/archive"
tar -xJf "$archive" -C "${work_dir}/archive"
archive_nd300=$(find "${work_dir}/archive" -type f -name nd300 -perm -111 -print -quit)
[[ -n $archive_nd300 ]]
archive_root=$(dirname "$archive_nd300")
cp "$archive_root/nd300" "$archive_root/speedqx" "$HOME/.cargo/bin/"
cat > "$HOME/.config/nd300/nd300-receipt.json" <<EOF
{"binaries":["nd300","speedqx"],"install_layout":"cargo-home","install_prefix":"$HOME/.cargo","provider":{"source":"cargo-dist","version":"0.31.0"},"source":{"app_name":"nd300","name":"qube-network-diagnostics","owner":"QubeTX","release_type":"github"},"version":"$version"}
EOF
[[ -x $HOME/.cargo/bin/nd300 && -f $HOME/.config/nd300/nd300-receipt.json ]]

sudo installer -pkg "$pkg" -target /
[[ ! -e $HOME/.cargo/bin/nd300 && ! -e $HOME/.cargo/bin/speedqx ]]
[[ ! -e $HOME/.config/nd300/nd300-receipt.json ]]
pkgutil --pkg-info com.qubetx.nd300.pkg | grep -F "version: $version"
receipt='/Library/Application Support/ND300/install-receipt.json'
[[ -f $receipt && ! -L $receipt ]]
nd300_sha=$(shasum -a 256 /usr/local/bin/nd300 | awk '{print $1}')
speedqx_sha=$(shasum -a 256 /usr/local/bin/speedqx | awk '{print $1}')
jq -e --arg version "$version" --arg nd300_sha "$nd300_sha" --arg speedqx_sha "$speedqx_sha" '
  .schema_version == 1 and
  .package_id == "com.qubetx.nd300.pkg" and
  .version == $version and
  .install_channel == "macos-dmg-pkg" and
  .binaries.nd300.path == "/usr/local/bin/nd300" and
  .binaries.nd300.sha256 == $nd300_sha and
  .binaries.speedqx.path == "/usr/local/bin/speedqx" and
  .binaries.speedqx.sha256 == $speedqx_sha
' "$receipt"
[[ $(/usr/local/bin/nd300 --version) == "nd300 $version" ]]
[[ $(/usr/local/bin/speedqx --version) == "speedqx $version" ]]
lipo /usr/local/bin/nd300 -verify_arch arm64 x86_64
lipo /usr/local/bin/speedqx -verify_arch arm64 x86_64

set +e
/usr/local/bin/nd300 --fast --json > "${work_dir}/diagnostic.json"
diagnostic_exit=$?
/usr/local/bin/nd300 --ascii --fast > "${work_dir}/diagnostic.txt"
ascii_exit=$?
set -e
(( diagnostic_exit >= 0 && diagnostic_exit <= 2 ))
(( ascii_exit >= 0 && ascii_exit <= 2 ))
jq -e '.schema_version == 1' "${work_dir}/diagnostic.json"
update=$(/usr/local/bin/speedqx update --json)
jq -e '.success == true and .install_channel == "macos-dmg-pkg" and .requires_user_action == false' <<< "$update"

# A same-version repair and an explicitly launched downgrade are fresh-install
# semantics. They never apply to the latest-only automatic update path.
sudo installer -pkg "$pkg" -target /
pkgutil --pkg-info com.qubetx.nd300.pkg | grep -F "version: $version"
fixture_signature=$(pkgutil --check-signature "$fixture")
grep -Fq "$APPLE_INSTALLER_SIGNING_IDENTITY" <<< "$fixture_signature"
sudo installer -pkg "$fixture" -target /
pkgutil --pkg-info com.qubetx.nd300.pkg | grep -F 'version: 999.0.0'
sudo installer -pkg "$pkg" -target /
pkgutil --pkg-info com.qubetx.nd300.pkg | grep -F "version: $version"
[[ $(/usr/local/bin/nd300 --version) == "nd300 $version" ]]
[[ $(/usr/local/bin/speedqx --version) == "speedqx $version" ]]

uninstall=$(sudo /usr/local/bin/nd300 --json uninstall)
printf '%s\n' "$uninstall" > "${work_dir}/uninstall.json"
jq -e '.success == true and .binary_removed == true and .sibling_removed == true and .receipt_removed == true' "${work_dir}/uninstall.json"
[[ ! -e /usr/local/bin/nd300 && ! -e /usr/local/bin/speedqx ]]
[[ ! -e $receipt ]]
if pkgutil --pkg-info com.qubetx.nd300.pkg >/dev/null 2>&1; then
    echo "package receipt still exists after uninstall" >&2
    exit 1
fi

sudo installer -pkg "$pkg" -target /
[[ $(/usr/local/bin/nd300 --version) == "nd300 $version" ]]
sudo /usr/local/bin/nd300 --json uninstall >/dev/null

echo "Validated universal PKG lifecycle, takeover, repair, downgrade, and uninstall."
