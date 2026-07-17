#!/usr/bin/env bash
# Validate one packaged ND300 macOS archive without installing or mutating the
# host. Pass --require-trust only after Developer ID signing and notarization;
# the signing workflow separately requires Apple `notarytool` status Accepted,
# while this validator requires Gatekeeper's online install-policy acceptance
# for each exact bare Mach-O CLI (there is no staplable app/pkg container).

set -euo pipefail

script_dir=$(
    unset CDPATH
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd
)
# shellcheck source=scripts/macos-release-lib.sh
. "${script_dir}/macos-release-lib.sh"

if [[ $# -lt 3 || $# -gt 4 ]]; then
    echo "usage: $0 <archive.tar.xz> <apple-target> <version> [--require-trust]" >&2
    exit 64
fi

archive=$1
target=$2
version=$3
trust=${4:-}
expected_team_id=M9D5379H93
expected_signing_fingerprint=739B04530883FF9B665C66BD464F98C622971B32

case "$target" in
    aarch64-apple-darwin)
        expected_arch=arm64
        expected_minos=11.0
        ;;
    x86_64-apple-darwin)
        expected_arch=x86_64
        expected_minos=10.12
        ;;
    *)
        echo "unsupported Apple target: $target" >&2
        exit 64
        ;;
esac
if [[ -n $trust && $trust != --require-trust ]]; then
    echo "unknown option: $trust" >&2
    exit 64
fi
if [[ ! -f $archive ]]; then
    echo "archive not found: $archive" >&2
    exit 66
fi

archive=$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")
archive_name=$(basename "$archive")
archive_root=${archive_name%.tar.xz}
sidecar="${archive}.sha256"

if [[ ! -f $sidecar ]]; then
    echo "archive checksum sidecar not found: $sidecar" >&2
    exit 66
fi
(
    cd "$(dirname "$archive")"
    shasum -a 256 -c "$(basename "$sidecar")"
)

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/nd300-validate-${target}.XXXXXX")
cleanup() {
    rm -rf "$work_dir"
}
trap cleanup EXIT INT TERM

COPYFILE_DISABLE=1 tar -xJf "$archive" -C "$work_dir"
root="${work_dir}/${archive_root}"

for required in nd300 speedqx man/nd300.1 man/speedqx.1 README.md CHANGELOG.md LICENSE; do
    if [[ ! -e ${root}/${required} ]]; then
        echo "required archive entry missing: ${archive_root}/${required}" >&2
        exit 65
    fi
done

identifier_for() {
    case "$1" in
        nd300) printf '%s\n' com.qubetx.nd300 ;;
        speedqx) printf '%s\n' com.qubetx.speedqx ;;
        *) return 1 ;;
    esac
}
for name in nd300 speedqx; do
    binary="${root}/${name}"
    identifier=$(identifier_for "$name")
    if [[ ! -x $binary ]]; then
        echo "packaged binary is not executable: $name" >&2
        exit 65
    fi
    if ! file "$binary" | grep -Fq "$expected_arch"; then
        echo "wrong architecture for ${name}; expected ${expected_arch}" >&2
        file "$binary" >&2
        exit 1
    fi
    version_output=$("$binary" --version)
    if [[ $version_output != "$name $version" ]]; then
        echo "wrong packaged version for ${name}; expected ${version}" >&2
        exit 1
    fi
    build_info=$(vtool -show-build "$binary" 2>&1)
    if ! printf '%s\n' "$build_info" | macos_build_info_has_deployment_floor "$expected_minos"; then
        echo "deployment floor mismatch for ${name}; expected macOS ${expected_minos}" >&2
        printf '%s\n' "$build_info" >&2
        exit 1
    fi

    if [[ $trust == --require-trust ]]; then
        codesign --verify --strict --verbose=4 "$binary"
        signature_details=$(codesign -d --verbose=4 "$binary" 2>&1)
        printf '%s\n' "$signature_details" | grep -Fqx "Identifier=${identifier}"
        printf '%s\n' "$signature_details" | grep -Fqx "TeamIdentifier=${expected_team_id}"
        printf '%s\n' "$signature_details" | grep -Eq '^CodeDirectory .*flags=.*\(runtime\)'
        printf '%s\n' "$signature_details" | grep -Eq '^Timestamp=.+'
        certificate_prefix="${work_dir}/${name}-embedded-signing-certificate"
        codesign -d --extract-certificates="$certificate_prefix" "$binary" >/dev/null 2>&1
        embedded_fingerprint=$(openssl x509 \
            -inform DER \
            -in "${certificate_prefix}0" \
            -noout \
            -fingerprint \
            -sha1 \
            | sed 's/.*=//; s/://g')
        if [[ $embedded_fingerprint != "$expected_signing_fingerprint" ]]; then
            echo "signed binary certificate fingerprint mismatch for ${target}/${name}" >&2
            exit 1
        fi
        # Reproduce a browser-downloaded artifact's quarantine state before
        # asking Gatekeeper and executing the CLI. Extended attributes do not
        # alter the signed Mach-O bytes and are applied only to this extracted
        # validation copy.
        /usr/bin/xattr -w \
            com.apple.quarantine \
            '0081;00000000;ND300;00000000-0000-0000-0000-000000000000' \
            "$binary"
        if ! gatekeeper_assessment=$(/usr/sbin/spctl \
            --assess \
            --type install \
            --ignore-cache \
            --no-cache \
            --verbose=4 \
            "$binary" 2>&1); then
            echo "Gatekeeper rejected ${target}/${name}:" >&2
            printf '%s\n' "$gatekeeper_assessment" >&2
            exit 1
        fi
        if ! printf '%s\n' "$gatekeeper_assessment" | grep -Fq 'source=Notarized Developer ID'; then
            echo "Gatekeeper did not identify ${target}/${name} as Notarized Developer ID" >&2
            printf '%s\n' "$gatekeeper_assessment" >&2
            exit 1
        fi
        quarantined_version_output=$("$binary" --version)
        if [[ $quarantined_version_output != "$name $version" ]]; then
            echo "quarantined ${target}/${name} did not execute at the expected version" >&2
            exit 1
        fi
    fi
done

echo "Validated ${archive_name}: ${expected_arch}, macOS ${expected_minos}+, nd300/speedqx ${version}${trust:+, trusted}."
