#!/usr/bin/env bash
# Sign and notarize one cargo-dist macOS archive before it is uploaded.
#
# Required environment variables are GitHub Actions secrets/variables. Never
# print them, persist decoded credentials outside the ephemeral work directory,
# or add a fallback that hosts unsigned macOS artifacts.

set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "usage: $0 <apple-target> <dist-manifest.json> <artifact-directory>" >&2
    exit 64
fi

target=$1
manifest=$2
artifact_dir=$3

case "$target" in
    aarch64-apple-darwin | x86_64-apple-darwin) ;;
    *)
        echo "refusing to sign unsupported target: $target" >&2
        exit 64
        ;;
esac

required_vars=(
    APPLE_CERTIFICATE_P12_BASE64
    APPLE_CERTIFICATE_PASSWORD
    APPLE_API_KEY_P8_BASE64
    APPLE_API_KEY_ID
    APPLE_API_ISSUER_ID
    APPLE_SIGNING_IDENTITY
    APPLE_TEAM_ID
)
for name in "${required_vars[@]}"; do
    if [[ -z ${!name:-} ]]; then
        echo "required Apple release credential is unavailable: $name" >&2
        exit 78
    fi
done

if [[ ! -f $manifest ]]; then
    echo "cargo-dist manifest not found: $manifest" >&2
    exit 66
fi

archive_name="nd300-${target}.tar.xz"
archive="${artifact_dir}/${archive_name}"
sidecar="${archive}.sha256"
archive_root="nd300-${target}"

if [[ ! -f $archive ]]; then
    echo "cargo-dist macOS archive not found: $archive" >&2
    exit 66
fi

runner_temp=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
work_dir=$(mktemp -d "${runner_temp%/}/nd300-notary-${target}.XXXXXX")
credential_dir="${work_dir}/credentials"
keychain="${work_dir}/nd300-signing.keychain-db"
mkdir -m 700 "$credential_dir"
chmod 700 "$work_dir"

original_user_keychains=()
while IFS= read -r line; do
    keychain_path=${line#*\"}
    keychain_path=${keychain_path%\"*}
    if [[ -n $keychain_path ]]; then
        original_user_keychains+=("$keychain_path")
    fi
done < <(security list-keychains -d user)
keychain_search_modified=false

restore_keychain_search() {
    if [[ $keychain_search_modified != true ]]; then
        return 0
    fi
    security list-keychains -d user -s "${original_user_keychains[@]}"
    keychain_search_modified=false
}

cleanup() {
    restore_keychain_search >/dev/null 2>&1 || true
    security delete-keychain "$keychain" >/dev/null 2>&1 || true
    rm -rf "$work_dir"
}
trap cleanup EXIT INT TERM

p12_path="${credential_dir}/developer-id.p12"
api_key_path="${credential_dir}/AuthKey_${APPLE_API_KEY_ID}.p8"
keychain_password=$(openssl rand -base64 32)

printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | /usr/bin/base64 -D > "$p12_path"
printf '%s' "$APPLE_API_KEY_P8_BASE64" | /usr/bin/base64 -D > "$api_key_path"
chmod 600 "$p12_path" "$api_key_path"

security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$p12_path" \
    -k "$keychain" \
    -P "$APPLE_CERTIFICATE_PASSWORD" \
    -T /usr/bin/codesign \
    -T /usr/bin/security
security set-key-partition-list \
    -S apple-tool:,apple: \
    -s \
    -k "$keychain_password" \
    "$keychain" >/dev/null

# Resolve exactly one imported Developer ID identity and sign by its SHA-1
# fingerprint. A display name can be ambiguous on a reused macOS runner.
identity_lines=$(security find-identity -v -p codesigning "$keychain" \
    | grep -E '^[[:space:]]*[0-9]+\) [[:xdigit:]]{40} "[^"]+"$' || true)
identity_count=$(printf '%s\n' "$identity_lines" | grep -c . || true)
if [[ $identity_count -ne 1 ]]; then
    echo "expected exactly one Developer ID identity in the ephemeral keychain; found ${identity_count}" >&2
    exit 1
fi
signing_fingerprint=$(printf '%s\n' "$identity_lines" | awk '{print $2}')
if [[ ! $signing_fingerprint =~ ^[[:xdigit:]]{40}$ ]]; then
    echo "could not resolve the imported Developer ID certificate fingerprint" >&2
    exit 1
fi
# The configured selector must be the immutable fingerprint. Retain the sole
# imported certificate's display name only to verify codesign's Authority line
# after signing; it is never accepted as configuration.
imported_identity=$(printf '%s\n' "$identity_lines" | sed -E 's/^[^"]*"([^"]+)".*$/\1/')
if [[ ! $APPLE_SIGNING_IDENTITY =~ ^[[:xdigit:]]{40}$ \
    || $APPLE_SIGNING_IDENTITY != "$signing_fingerprint" ]]; then
    echo "configured Apple signing fingerprint does not match the sole imported identity" >&2
    exit 1
fi

extract_dir="${work_dir}/archive"
mkdir "$extract_dir"
COPYFILE_DISABLE=1 tar -xJf "$archive" -C "$extract_dir"

binaries=(nd300 speedqx)
identifier_for() {
    case "$1" in
        nd300) printf '%s\n' com.qubetx.nd300 ;;
        speedqx) printf '%s\n' com.qubetx.speedqx ;;
        *) return 1 ;;
    esac
}
for name in "${binaries[@]}"; do
    binary="${extract_dir}/${archive_root}/${name}"
    if [[ ! -f $binary || ! -x $binary ]]; then
        echo "expected executable is missing from ${archive_name}: ${archive_root}/${name}" >&2
        exit 65
    fi
done

echo "Signing ${archive_name} with hardened runtime..."
# A newly-created keychain is not automatically in codesign's search list on
# clean GitHub runners. Add it only for signing and restore the original list
# immediately afterward; the cleanup trap repeats restoration on every failure.
security list-keychains -d user -s "$keychain" "${original_user_keychains[@]}"
keychain_search_modified=true
for name in "${binaries[@]}"; do
    binary="${extract_dir}/${archive_root}/${name}"
    identifier=$(identifier_for "$name")
    codesign \
        --force \
        --identifier "$identifier" \
        --options runtime \
        --timestamp \
        --keychain "$keychain" \
        --sign "$signing_fingerprint" \
        "$binary"
done
restore_keychain_search

for name in "${binaries[@]}"; do
    binary="${extract_dir}/${archive_root}/${name}"
    identifier=$(identifier_for "$name")
    codesign --verify --strict --verbose=4 "$binary"
    signature_details=$(codesign -d --verbose=4 "$binary" 2>&1)
    if ! printf '%s\n' "$signature_details" | grep -Fqx "Identifier=${identifier}"; then
        echo "signed binary identifier mismatch for ${target}/${name}" >&2
        exit 1
    fi
    if ! printf '%s\n' "$signature_details" | grep -Fqx "TeamIdentifier=${APPLE_TEAM_ID}"; then
        echo "signed binary Team ID mismatch for ${target}/${name}" >&2
        exit 1
    fi
    if ! printf '%s\n' "$signature_details" | grep -Fqx "Authority=${imported_identity}"; then
        echo "signed binary Developer ID authority mismatch for ${target}/${name}" >&2
        exit 1
    fi
    certificate_prefix="${work_dir}/${name}-embedded-signing-certificate"
    codesign -d --extract-certificates="$certificate_prefix" "$binary" >/dev/null 2>&1
    embedded_fingerprint=$(openssl x509 \
        -inform DER \
        -in "${certificate_prefix}0" \
        -noout \
        -fingerprint \
        -sha1 \
        | sed 's/.*=//; s/://g')
    if [[ $embedded_fingerprint != "$signing_fingerprint" ]]; then
        echo "signed binary certificate fingerprint mismatch for ${target}/${name}" >&2
        exit 1
    fi
    if ! printf '%s\n' "$signature_details" | grep -Eq '^CodeDirectory .*flags=.*\(runtime\)'; then
        echo "signed binary is missing the hardened-runtime flag for ${target}/${name}" >&2
        exit 1
    fi
    if ! printf '%s\n' "$signature_details" | grep -Eq '^Timestamp=.+'; then
        echo "signed binary is missing a secure timestamp for ${target}/${name}" >&2
        exit 1
    fi
done
echo "Verified Developer ID identity, Team ID, hardened runtime, and timestamp for ${target}."

notary_payload="${work_dir}/notary-payload"
mkdir "$notary_payload"
for name in "${binaries[@]}"; do
    cp "${extract_dir}/${archive_root}/${name}" "${notary_payload}/${name}"
done
notary_zip="${work_dir}/${archive_root}-notary.zip"
/usr/bin/ditto -c -k --keepParent "$notary_payload" "$notary_zip"

notary_result="${work_dir}/notary-result.json"
echo "Submitting ${target} to Apple Notary Service..."
xcrun notarytool submit "$notary_zip" \
    --key "$api_key_path" \
    --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER_ID" \
    --wait \
    --output-format json > "$notary_result"

submission_id=$(jq -r '.id // empty' "$notary_result")
notary_status=$(jq -r '.status // empty' "$notary_result")
if [[ $notary_status != Accepted ]]; then
    echo "Apple notarization failed for ${target}: ${notary_status:-unknown status}" >&2
    if [[ -n $submission_id ]]; then
        xcrun notarytool log "$submission_id" \
            --key "$api_key_path" \
            --key-id "$APPLE_API_KEY_ID" \
            --issuer "$APPLE_API_ISSUER_ID" || true
    fi
    exit 1
fi
echo "Apple notarization accepted for ${target} (submission ${submission_id})."

# Standalone CLI binaries have no staplable app/pkg container. Apple's Accepted
# service record binds the online ticket to these exact signed code hashes, but
# ticket propagation can lag the notary response. Gate each exact binary on the
# install-policy assessment, which distinguishes a notarized bare Mach-O from
# an otherwise-valid unnotarized Developer ID binary. `--type execute` and
# `codesign --check-notarization` do not make that distinction for bare CLIs.
for name in "${binaries[@]}"; do
    binary="${extract_dir}/${archive_root}/${name}"
    gatekeeper_accepted=false
    assessment=
    for attempt in 1 2 3 4 5 6; do
        if assessment=$(/usr/sbin/spctl \
            --assess \
            --type install \
            --ignore-cache \
            --no-cache \
            --verbose=4 \
            "$binary" 2>&1) \
            && printf '%s\n' "$assessment" | grep -Fq 'source=Notarized Developer ID'; then
            gatekeeper_accepted=true
            break
        fi
        if [[ $attempt -lt 6 ]]; then
            echo "Waiting for Gatekeeper notarization propagation (${target}/${name}, attempt ${attempt}/6)..."
            sleep 15
        fi
    done
    if [[ $gatekeeper_accepted != true ]]; then
        echo "Gatekeeper did not accept the notarized ${target}/${name} binary:" >&2
        printf '%s\n' "$assessment" >&2
        exit 1
    fi
done
echo "Gatekeeper accepted both exact ${target} binaries as Notarized Developer ID."

replacement="${work_dir}/${archive_name}"
COPYFILE_DISABLE=1 tar -cJf "$replacement" -C "$extract_dir" "$archive_root"
mv "$replacement" "$archive"

archive_sha=$(shasum -a 256 "$archive" | awk '{print $1}')
printf '%s *%s\n' "$archive_sha" "$archive_name" > "$sidecar"

manifest_tmp="${work_dir}/dist-manifest.json"
jq \
    --arg archive "$archive_name" \
    --arg sha "$archive_sha" \
    'if .artifacts[$archive] == null then
         error("archive missing from cargo-dist manifest: " + $archive)
     else
         .artifacts[$archive].checksums.sha256 = $sha
     end' \
    "$manifest" > "$manifest_tmp"
mv "$manifest_tmp" "$manifest"

(
    cd "$artifact_dir"
    shasum -a 256 -c "${archive_name}.sha256"
)

echo "Signed, notarized, and rehashed ${archive_name}."
