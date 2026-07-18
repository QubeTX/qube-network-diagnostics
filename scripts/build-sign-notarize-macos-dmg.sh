#!/usr/bin/env bash
# Build the universal, signed, notarized ND-300 PKG-in-DMG distribution.
# Runs only on an ephemeral native macOS GitHub runner.

set -euo pipefail

if [[ $# -ne 4 ]]; then
    echo "usage: $0 <version> <arm64-archive> <x86_64-archive> <output-dir>" >&2
    exit 64
fi

version=${1#v}
arm_archive=$2
x86_archive=$3
output_dir=$4
if [[ ! $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
    echo "version must be a stable numeric release: $version" >&2
    exit 64
fi

required_vars=(
    APPLE_CERTIFICATE_P12_BASE64
    APPLE_CERTIFICATE_PASSWORD
    APPLE_INSTALLER_CERTIFICATE_P12_BASE64
    APPLE_INSTALLER_CERTIFICATE_PASSWORD
    APPLE_API_KEY_P8_BASE64
    APPLE_API_KEY_ID
    APPLE_API_ISSUER_ID
    APPLE_SIGNING_IDENTITY
    APPLE_INSTALLER_SIGNING_IDENTITY
    APPLE_TEAM_ID
)
for name in "${required_vars[@]}"; do
    if [[ -z ${!name:-} ]]; then
        echo "required Apple release credential is unavailable: $name" >&2
        exit 78
    fi
done

for archive in "$arm_archive" "$x86_archive"; do
    if [[ ! -f $archive ]]; then
        echo "required macOS archive is missing: $archive" >&2
        exit 66
    fi
done

runner_temp=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
work_dir=$(mktemp -d "${runner_temp%/}/nd300-dmg.XXXXXX")
keychain="${work_dir}/nd300-release.keychain-db"
keychain_password=$(openssl rand -base64 32)
credential_dir="${work_dir}/credentials"
mkdir -m 700 "$credential_dir" "$output_dir"
chmod 700 "$work_dir"

original_user_keychains=()
while IFS= read -r line; do
    path=${line#*\"}
    path=${path%\"*}
    [[ -n $path ]] && original_user_keychains+=("$path")
done < <(security list-keychains -d user)

cleanup() {
    security list-keychains -d user -s "${original_user_keychains[@]}" >/dev/null 2>&1 || true
    security delete-keychain "$keychain" >/dev/null 2>&1 || true
    rm -rf "$work_dir"
}
trap cleanup EXIT INT TERM

app_p12="${credential_dir}/developer-id-application.p12"
installer_p12="${credential_dir}/developer-id-installer.p12"
api_key="${credential_dir}/AuthKey_${APPLE_API_KEY_ID}.p8"
printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | /usr/bin/base64 -D > "$app_p12"
printf '%s' "$APPLE_INSTALLER_CERTIFICATE_P12_BASE64" | /usr/bin/base64 -D > "$installer_p12"
printf '%s' "$APPLE_API_KEY_P8_BASE64" | /usr/bin/base64 -D > "$api_key"
chmod 600 "$app_p12" "$installer_p12" "$api_key"

security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$app_p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" \
    -T /usr/bin/codesign -T /usr/bin/security
security import "$installer_p12" -k "$keychain" -P "$APPLE_INSTALLER_CERTIFICATE_PASSWORD" \
    -T /usr/bin/pkgbuild -T /usr/bin/productsign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain" "${original_user_keychains[@]}"

application_identities=$(security find-identity -v -p codesigning "$keychain")
grep -Fq "$APPLE_SIGNING_IDENTITY" <<< "$application_identities" || {
    echo "configured Developer ID Application identity was not found" >&2
    exit 1
}
security find-certificate -c "$APPLE_INSTALLER_SIGNING_IDENTITY" \
    "$keychain" >/dev/null || {
    echo "configured Developer ID Installer certificate was not found" >&2
    exit 1
}

arm_dir="${work_dir}/arm64"
x86_dir="${work_dir}/x86_64"
mkdir "$arm_dir" "$x86_dir"
COPYFILE_DISABLE=1 tar -xJf "$arm_archive" -C "$arm_dir"
COPYFILE_DISABLE=1 tar -xJf "$x86_archive" -C "$x86_dir"

find_binary() {
    local root=$1 name=$2
    find "$root" -type f -name "$name" -perm -111 -print -quit
}

universal_dir="${work_dir}/universal"
mkdir "$universal_dir"
for binary in nd300 speedqx; do
    arm_binary=$(find_binary "$arm_dir" "$binary")
    x86_binary=$(find_binary "$x86_dir" "$binary")
    if [[ -z $arm_binary || -z $x86_binary ]]; then
        echo "could not locate both architecture-specific $binary binaries" >&2
        exit 65
    fi
    universal="${universal_dir}/${binary}"
    lipo -create "$arm_binary" "$x86_binary" -output "$universal"
    chmod 755 "$universal"
    lipo -verify_arch arm64 x86_64 "$universal"
    codesign --force --identifier "com.qubetx.${binary}" --options runtime --timestamp \
        --keychain "$keychain" --sign "$APPLE_SIGNING_IDENTITY" "$universal"
    codesign --verify --strict --verbose=4 "$universal"
    details=$(codesign -d --verbose=4 "$universal" 2>&1)
    grep -Fqx "Identifier=com.qubetx.${binary}" <<< "$details"
    grep -Fqx "TeamIdentifier=${APPLE_TEAM_ID}" <<< "$details"
    grep -Eq '^CodeDirectory .*flags=.*\(runtime\)' <<< "$details"
    grep -Eq '^Timestamp=.+' <<< "$details"
    test "$("$universal" --version)" = "$binary $version"
done

notarize() {
    local artifact=$1 result status submission
    result="${work_dir}/notary-$(basename "$artifact").json"
    xcrun notarytool submit "$artifact" \
        --key "$api_key" \
        --key-id "$APPLE_API_KEY_ID" \
        --issuer "$APPLE_API_ISSUER_ID" \
        --wait --output-format json > "$result"
    status=$(jq -r '.status // empty' "$result")
    submission=$(jq -r '.id // empty' "$result")
    if [[ $status != Accepted ]]; then
        [[ -n $submission ]] && xcrun notarytool log "$submission" \
            --key "$api_key" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID" || true
        echo "Apple notarization failed for $(basename "$artifact"): ${status:-unknown}" >&2
        exit 1
    fi
    echo "Apple notarization accepted for $(basename "$artifact") (${submission})."
}

payload="${work_dir}/payload"
install -d -m 755 "${payload}/usr/local/bin"
install -m 755 "${universal_dir}/nd300" "${payload}/usr/local/bin/nd300"
install -m 755 "${universal_dir}/speedqx" "${payload}/usr/local/bin/speedqx"
install -d -m 755 "${payload}/Library/Application Support/ND300"
nd300_sha=$(shasum -a 256 "${universal_dir}/nd300" | awk '{print $1}')
speedqx_sha=$(shasum -a 256 "${universal_dir}/speedqx" | awk '{print $1}')
cat > "${payload}/Library/Application Support/ND300/install-receipt.json" <<EOF
{"schema_version":1,"package_id":"com.qubetx.nd300.pkg","version":"$version","install_channel":"macos-dmg-pkg","binaries":{"nd300":{"path":"/usr/local/bin/nd300","sha256":"$nd300_sha"},"speedqx":{"path":"/usr/local/bin/speedqx","sha256":"$speedqx_sha"}}}
EOF
chmod 644 "${payload}/Library/Application Support/ND300/install-receipt.json"

scripts_dir="${work_dir}/package-scripts"
mkdir "$scripts_dir"
cat > "${scripts_dir}/preinstall" <<'EOF'
#!/bin/sh
set -eu
for path in /usr/local/bin/nd300 /usr/local/bin/speedqx \
    '/Library/Application Support/ND300/install-receipt.json'; do
    if [ -L "$path" ] || { [ -e "$path" ] && [ ! -f "$path" ]; }; then
        echo "Refusing to replace non-regular ND-300 path: $path" >&2
        exit 1
    fi
done
exit 0
EOF
cat > "${scripts_dir}/postinstall" <<EOF
#!/bin/sh
set -eu
expected='$version'
test "\$(/usr/local/bin/nd300 --version)" = "nd300 \$expected"
test "\$(/usr/local/bin/speedqx --version)" = "speedqx \$expected"

# The explicit PKG install is the user's newest channel choice. Consolidate a
# receipt/registry-proven Cargo-home pair as the console user; ambiguous files
# are retained, and cleanup is advisory so a failed migration never rolls back
# the newly verified package payload.
console_user=\$(/usr/bin/stat -f '%Su' /dev/console 2>/dev/null || true)
case "\$console_user" in
    ''|root|loginwindow|_mbsetupuser) ;;
    *)
        /usr/bin/sudo -H -u "\$console_user" \
            /usr/bin/env -u CARGO_HOME -u SUDO_UID -u SUDO_GID \
            /usr/local/bin/nd300 migrate-cleanup --quiet \
            --install-origin macos-pkg --cargo-copy || true
        ;;
esac
exit 0
EOF
chmod 755 "${scripts_dir}/preinstall" "${scripts_dir}/postinstall"

pkg="${work_dir}/nd300.pkg"
pkgbuild --root "$payload" \
    --scripts "$scripts_dir" \
    --identifier com.qubetx.nd300.pkg \
    --version "$version" \
    --install-location / \
    --sign "$APPLE_INSTALLER_SIGNING_IDENTITY" \
    --keychain "$keychain" \
    "$pkg"

signature=$(pkgutil --check-signature "$pkg")
grep -Fq 'Signed with a trusted timestamp' <<< "$signature"
grep -Fq "$APPLE_INSTALLER_SIGNING_IDENTITY" <<< "$signature"
expected_payload="${work_dir}/expected-payload.txt"
actual_payload="${work_dir}/actual-payload.txt"
printf '%s\n' Library 'Library/Application Support' \
    'Library/Application Support/ND300' \
    'Library/Application Support/ND300/install-receipt.json' \
    usr usr/local usr/local/bin usr/local/bin/nd300 usr/local/bin/speedqx \
    | sort -u > "$expected_payload"
pkgutil --payload-files "$pkg" | sed 's#^\./##' | sort -u > "$actual_payload"
diff -u "$expected_payload" "$actual_payload"

# Hosted validation installs this private, deliberately higher-version receipt
# before reinstalling the real package. That proves an explicitly launched
# fresh installer can honor the user's downgrade intent. The fixture is a CI
# artifact only and is never attached to the public release.
downgrade_fixture="${output_dir}/nd300-private-downgrade-test-fixture.pkg"
pkgbuild --root "$payload" \
    --scripts "$scripts_dir" \
    --identifier com.qubetx.nd300.pkg \
    --version 999.0.0 \
    --install-location / \
    --sign "$APPLE_INSTALLER_SIGNING_IDENTITY" \
    --keychain "$keychain" \
    "$downgrade_fixture"
fixture_signature=$(pkgutil --check-signature "$downgrade_fixture")
grep -Fq 'Signed with a trusted timestamp' <<< "$fixture_signature"
grep -Fq "$APPLE_INSTALLER_SIGNING_IDENTITY" <<< "$fixture_signature"
pkgutil --payload-files "$downgrade_fixture" | sed 's#^\./##' | sort -u \
    > "${work_dir}/fixture-payload.txt"
diff -u "$expected_payload" "${work_dir}/fixture-payload.txt"

notarize "$pkg"
xcrun stapler staple "$pkg"
xcrun stapler validate "$pkg"
spctl --assess --type install --verbose=4 "$pkg"

dmg_root="${work_dir}/dmg-root"
mkdir "$dmg_root"
cp "$pkg" "${dmg_root}/nd300.pkg"
cat > "${dmg_root}/README.txt" <<'EOF'
ND-300 installer

Open nd300.pkg and follow Apple Installer. The signed package installs the
versionless `nd300` and `speedqx` commands system-wide in /usr/local/bin.

If installation is blocked or cancelled, download a fresh installer from:
https://reports.qubetx.com/nd300#install
EOF

dmg="${output_dir}/nd300-universal-apple-darwin.dmg"
hdiutil create -volname "ND-300" -srcfolder "$dmg_root" -format UDZO -ov "$dmg"
codesign --force --identifier com.qubetx.nd300.dmg --timestamp \
    --keychain "$keychain" --sign "$APPLE_SIGNING_IDENTITY" "$dmg"
codesign --verify --deep --strict --verbose=4 "$dmg"
notarize "$dmg"
xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg"

sha=$(shasum -a 256 "$dmg" | awk '{print $1}')
printf '%s *%s\n' "$sha" "$(basename "$dmg")" > "${dmg}.sha256"
(
    cd "$output_dir"
    shasum -a 256 -c "$(basename "$dmg").sha256"
)

echo "Built signed, notarized, stapled universal DMG: $dmg"
